use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use crate::adapters::{all_agents, AgentDataBatch};
use crate::db::WriteRequest;
use crate::watcher::BehaviorRuntime;

/// 定时轮询策略（用于 JSON 文件，基于 mtime 变化检测）
pub fn run_polling(
    adapter_names: Vec<String>,
    log_patterns: Vec<Vec<String>>,
    write_tx: Sender<WriteRequest>,
    stop_flag: Arc<AtomicBool>,
    poll_interval_secs: u32,
    behavior_runtime: Option<BehaviorRuntime>,
) {
    let mut mtime_cache: HashMap<String, u64> = HashMap::new();
    let mut file_offsets: HashMap<String, u64> = HashMap::new();
    let agents = all_agents();

    loop {
        // 可中断的等待：每 500ms 检查一次 stop_flag
        let deadline = std::time::Instant::now() + Duration::from_secs(poll_interval_secs as u64);
        while std::time::Instant::now() < deadline {
            if stop_flag.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }

        if stop_flag.load(Ordering::Relaxed) {
            return;
        }

        for (idx, patterns) in log_patterns.iter().enumerate() {
            let agent_name = &adapter_names[idx];
            let agent = agents.iter().find(|a| a.agent_name() == agent_name);
            if agent.is_none() {
                continue;
            }
            let agent = agent.unwrap();

            for pattern in patterns {
                let entries = match glob::glob(pattern) {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                for entry in entries.flatten() {
                    let path = entry.to_string_lossy().to_string();
                    let meta = match std::fs::metadata(&entry) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };

                    let mtime = meta
                        .modified()
                        .unwrap_or(std::time::UNIX_EPOCH)
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();

                    let prev_mtime = mtime_cache.get(&path).copied();
                    if prev_mtime == Some(mtime) {
                        continue;
                    }

                    if let Ok(content) = std::fs::read_to_string(&entry) {
                        let batch = build_polling_batch(
                            agent_name,
                            &entry,
                            &path,
                            content,
                            &meta,
                            &file_offsets,
                        );
                        let logs = agent.extract_tokens(&batch).logs;
                        if !logs.is_empty() {
                            let total_tokens: i64 = logs.iter().map(|l| l.token_count).sum();
                            log::info!(
                                "[polling] {}: 文件变更 {}, {} 条记录, {} tokens",
                                agent_name,
                                path,
                                logs.len(),
                                total_tokens
                            );
                            let _ = write_tx.send(WriteRequest::InsertTokenLogs(logs));
                        }

                        if let Some(runtime) = behavior_runtime.as_ref() {
                            if runtime.is_enabled() {
                                for event in agent.extract_behavior(&batch) {
                                    runtime.dispatcher.handle_event(event);
                                }
                            }
                        }
                    }
                    mtime_cache.insert(path.clone(), mtime);
                    file_offsets.insert(path, meta.len());
                }
            }
        }
    }
}

fn build_polling_batch(
    agent_name: &str,
    entry: &std::path::Path,
    path: &str,
    content: String,
    meta: &std::fs::Metadata,
    file_offsets: &HashMap<String, u64>,
) -> AgentDataBatch {
    let mtime = meta
        .modified()
        .unwrap_or(std::time::UNIX_EPOCH)
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if entry.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
        let behavior_content = if agent_name == "codex" {
            match file_offsets.get(path).copied() {
                Some(prev_size) if meta.len() >= prev_size => {
                    super::notify_strategy::read_from_offset(entry, prev_size).unwrap_or_default()
                }
                Some(_) => content.clone(),
                None => String::new(),
            }
        } else {
            content.clone()
        };

        return AgentDataBatch::JsonlIncrement {
            agent_name: agent_name.to_string(),
            source_key: path.to_string(),
            path: entry.to_path_buf(),
            content: behavior_content,
            token_context: Some(content),
            initial_model: None,
            previous_offset: file_offsets.get(path).copied().unwrap_or(0),
            next_offset: meta.len(),
        };
    }

    AgentDataBatch::JsonDocument {
        agent_name: agent_name.to_string(),
        source_key: path.to_string(),
        path: entry.to_path_buf(),
        content,
        mtime,
    }
}
