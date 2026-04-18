use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use crate::adapters::all_adapters;
use crate::db::WriteRequest;

/// 定时轮询策略（用于 JSON 文件，基于 mtime 变化检测）
pub fn run_polling(
    adapter_names: Vec<String>,
    log_patterns: Vec<Vec<String>>,
    write_tx: Sender<WriteRequest>,
    stop_flag: Arc<AtomicBool>,
    poll_interval_secs: u32,
) {
    let mut mtime_cache: HashMap<String, u64> = HashMap::new();
    let adapters = all_adapters();

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
            let adapter = adapters.iter().find(|a| a.agent_name() == agent_name);
            if adapter.is_none() {
                continue;
            }
            let adapter = adapter.unwrap();

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

                    // JSON 文件整体重新解析
                    if let Ok(content) = std::fs::read_to_string(&entry) {
                        let logs = adapter.parse_content(&content);
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
                    }
                    mtime_cache.insert(path, mtime);
                }
            }
        }
    }
}
