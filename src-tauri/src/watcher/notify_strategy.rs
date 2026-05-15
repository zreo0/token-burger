use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebouncedEvent};

use crate::adapters::{all_adapters, codex};
use crate::db::WriteRequest;

struct PathChangeContext<'a> {
    adapter_names: &'a [String],
    adapters: &'a [Box<dyn crate::adapters::AgentAdapter>],
    path_to_adapter: &'a HashMap<String, usize>,
    write_tx: &'a Sender<WriteRequest>,
}

/// 基于 notify-debouncer-full 的实时监听策略
///
/// 用 notify 推荐的系统 backend 监听文件变化，500ms debounce 后读取增量内容。
/// 如果被监听目录/文件不存在则回退到 3s glob 轮询检测新路径，
/// 一旦发现新路径就注册到 watcher。
pub fn run_notify_polling(
    adapter_names: Vec<String>,
    log_patterns: Vec<Vec<String>>,
    write_tx: Sender<WriteRequest>,
    stop_flag: Arc<AtomicBool>,
    initial_offsets: HashMap<String, u64>,
) {
    // path → adapter index
    let mut path_to_adapter: HashMap<String, usize> = HashMap::new();
    // path → file size（用于增量读取），初始化时优先使用 DB 中的 offset
    let mut file_offsets: HashMap<String, u64> = initial_offsets;
    let mut codex_model_cache: HashMap<String, String> = HashMap::new();
    let adapters = all_adapters();

    // 设置 debouncer 通道
    let (tx, rx) = std::sync::mpsc::channel::<Result<Vec<DebouncedEvent>, Vec<notify::Error>>>();

    let mut debouncer = match new_debouncer(Duration::from_millis(500), None, tx) {
        Ok(d) => d,
        Err(e) => {
            log::error!("无法创建 notify debouncer: {}", e);
            return;
        }
    };

    // 辅助函数：展开 glob 模式，注册 watcher，记录初始 offset
    let register_paths = |patterns: &[String],
                          adapter_idx: usize,
                          start_new_at_current_size: bool,
                          path_to_adapter: &mut HashMap<String, usize>,
                          file_offsets: &mut HashMap<String, u64>,
                          debouncer: &mut notify_debouncer_full::Debouncer<
        notify::RecommendedWatcher,
        notify_debouncer_full::RecommendedCache,
    >| {
        for pattern in patterns {
            if let Ok(entries) = glob::glob(pattern) {
                for entry in entries.flatten() {
                    let path = entry.to_string_lossy().to_string();
                    if path_to_adapter.contains_key(&path) {
                        continue;
                    }
                    if !file_offsets.contains_key(&path) {
                        let offset = if start_new_at_current_size {
                            std::fs::metadata(&entry).map(|m| m.len()).unwrap_or(0)
                        } else {
                            0
                        };
                        file_offsets.insert(path.clone(), offset);
                    }
                    path_to_adapter.insert(path.clone(), adapter_idx);

                    // 监听文件的父目录（更可靠地捕获新建/追加事件）
                    let watch_path = entry.parent().unwrap_or(&entry);
                    let _ = debouncer.watch(watch_path, RecursiveMode::Recursive);
                }
            }
        }
    };

    // 初始注册
    for (idx, patterns) in log_patterns.iter().enumerate() {
        register_paths(
            patterns,
            idx,
            true,
            &mut path_to_adapter,
            &mut file_offsets,
            &mut debouncer,
        );
    }

    // 主循环：优先处理 debounced 事件，定期扫描新路径并补读漏掉的追加内容。
    let mut since_last_scan = std::time::Instant::now();
    while !stop_flag.load(Ordering::Relaxed) {
        // 使用较短超时以便及时响应 stop_flag
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Ok(events)) => {
                if stop_flag.load(Ordering::Relaxed) {
                    break;
                }
                process_events(
                    &events,
                    &adapter_names,
                    &adapters,
                    &path_to_adapter,
                    &mut file_offsets,
                    &mut codex_model_cache,
                    &write_tx,
                );
            }
            Ok(Err(errs)) => {
                for e in errs {
                    log::warn!("notify 错误: {}", e);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // notify 可能漏掉 append 或返回目录路径，定期 size 扫描用于兜底
                if since_last_scan.elapsed() >= Duration::from_secs(3) {
                    for (idx, patterns) in log_patterns.iter().enumerate() {
                        register_paths(
                            patterns,
                            idx,
                            false,
                            &mut path_to_adapter,
                            &mut file_offsets,
                            &mut debouncer,
                        );
                    }
                    reconcile_registered_paths(
                        &adapter_names,
                        &adapters,
                        &path_to_adapter,
                        &mut file_offsets,
                        &mut codex_model_cache,
                        &write_tx,
                    );
                    since_last_scan = std::time::Instant::now();
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    // 清理
    drop(debouncer);
}

/// 处理 debounced 文件变化事件
fn process_events(
    events: &[DebouncedEvent],
    adapter_names: &[String],
    adapters: &[Box<dyn crate::adapters::AgentAdapter>],
    path_to_adapter: &HashMap<String, usize>,
    file_offsets: &mut HashMap<String, u64>,
    codex_model_cache: &mut HashMap<String, String>,
    write_tx: &Sender<WriteRequest>,
) {
    let context = PathChangeContext {
        adapter_names,
        adapters,
        path_to_adapter,
        write_tx,
    };

    for event in events {
        for path in &event.paths {
            let path_str = path.to_string_lossy().to_string();
            process_path_change(
                path,
                &path_str,
                "notify",
                &context,
                file_offsets,
                codex_model_cache,
            );
        }
    }
}

fn reconcile_registered_paths(
    adapter_names: &[String],
    adapters: &[Box<dyn crate::adapters::AgentAdapter>],
    path_to_adapter: &HashMap<String, usize>,
    file_offsets: &mut HashMap<String, u64>,
    codex_model_cache: &mut HashMap<String, String>,
    write_tx: &Sender<WriteRequest>,
) {
    let context = PathChangeContext {
        adapter_names,
        adapters,
        path_to_adapter,
        write_tx,
    };

    let paths: Vec<String> = path_to_adapter.keys().cloned().collect();
    for path_str in paths {
        process_path_change(
            std::path::Path::new(&path_str),
            &path_str,
            "reconcile",
            &context,
            file_offsets,
            codex_model_cache,
        );
    }
}

fn process_path_change(
    path: &std::path::Path,
    path_str: &str,
    source: &str,
    context: &PathChangeContext<'_>,
    file_offsets: &mut HashMap<String, u64>,
    codex_model_cache: &mut HashMap<String, String>,
) {
    let adapter_idx = match context.path_to_adapter.get(path_str) {
        Some(idx) => *idx,
        None => return,
    };
    let agent_name = &context.adapter_names[adapter_idx];
    let adapter = match context
        .adapters
        .iter()
        .find(|a| a.agent_name() == agent_name)
    {
        Some(a) => a,
        None => return,
    };

    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return,
    };
    let new_size = meta.len();
    let prev_offset = file_offsets.get(path_str).copied().unwrap_or(0);

    if new_size < prev_offset {
        codex_model_cache.remove(path_str);
        log::info!(
            "[{}] {}: 文件轮转检测 offset {}→{}, 从头读取",
            source,
            agent_name,
            prev_offset,
            new_size
        );
        let content = match read_from_offset(path, 0) {
            Ok(content) => content,
            Err(err) => {
                log::warn!(
                    "[{}] {}: 轮转重读失败，保留 offset {}: {}",
                    source,
                    agent_name,
                    prev_offset,
                    err
                );
                return;
            }
        };
        let logs = if agent_name == "codex" {
            let parsed = codex::parse_content_with_model(&content, codex::DEFAULT_CODEX_MODEL);
            codex_model_cache.insert(path_str.to_string(), parsed.final_model);
            parsed.logs
        } else {
            adapter.parse_content(&content)
        };
        if !logs.is_empty() {
            log::info!(
                "[{}] {}: 解析 {} 条记录 (轮转重读)",
                source,
                agent_name,
                logs.len()
            );
            let _ = context.write_tx.send(WriteRequest::InsertTokenLogs(logs));
        }
        file_offsets.insert(path_str.to_string(), new_size);
        let _ = context.write_tx.send(WriteRequest::UpdateOffset {
            file_path: path_str.to_string(),
            offset: new_size,
        });
        return;
    }

    if new_size == prev_offset {
        return;
    }

    let logs = match parse_changed_content(
        path,
        path_str,
        prev_offset,
        agent_name,
        adapter.as_ref(),
        codex_model_cache,
    ) {
        Ok(logs) => logs,
        Err(err) => {
            log::warn!(
                "[{}] {}: 增量读取失败，保留 offset {}: {}",
                source,
                agent_name,
                prev_offset,
                err
            );
            return;
        }
    };
    if !logs.is_empty() {
        let total_tokens: i64 = logs.iter().map(|l| l.token_count).sum();
        let models: Vec<&str> = logs
            .iter()
            .map(|l| l.model_id.as_str())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        log::info!(
            "[{}] {}: 增量读取 offset {}→{}, {} 条记录, {} tokens, models={:?}",
            source,
            agent_name,
            prev_offset,
            new_size,
            logs.len(),
            total_tokens,
            models
        );
        let _ = context.write_tx.send(WriteRequest::InsertTokenLogs(logs));
    }
    file_offsets.insert(path_str.to_string(), new_size);
    let _ = context.write_tx.send(WriteRequest::UpdateOffset {
        file_path: path_str.to_string(),
        offset: new_size,
    });
}

fn parse_changed_content(
    path: &std::path::Path,
    path_str: &str,
    prev_offset: u64,
    agent_name: &str,
    adapter: &dyn crate::adapters::AgentAdapter,
    codex_model_cache: &mut HashMap<String, String>,
) -> std::io::Result<Vec<crate::adapters::TokenLog>> {
    if agent_name != "codex" {
        let content = read_from_offset(path, prev_offset)?;
        return Ok(adapter.parse_content(&content));
    }

    let (content, initial_model) = match codex_model_cache.get(path_str) {
        Some(model) => (read_from_offset(path, prev_offset)?, model.as_str()),
        None => (std::fs::read_to_string(path)?, codex::DEFAULT_CODEX_MODEL),
    };

    let parsed = codex::parse_content_with_model(&content, initial_model);
    codex_model_cache.insert(path_str.to_string(), parsed.final_model);

    Ok(parsed.logs)
}

/// 从指定 offset 读取文件内容
pub(crate) fn read_from_offset(path: &std::path::Path, offset: u64) -> std::io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::codex::CodexAdapter;
    use std::io::Write;

    #[test]
    fn codex_cache_miss_recovers_model_then_incremental_uses_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let initial = r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}
{"timestamp":"2026-01-14T07:23:24.629Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1}}}}
"#;
        std::fs::write(&path, initial).unwrap();
        let path_str = path.to_string_lossy().to_string();
        let adapter = CodexAdapter;
        let mut cache = HashMap::new();

        let logs = parse_changed_content(
            &path,
            &path_str,
            initial.len() as u64,
            "codex",
            &adapter,
            &mut cache,
        )
        .unwrap();

        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].model_id, "gpt-5.4");
        assert_eq!(cache.get(&path_str).map(String::as_str), Some("gpt-5.4"));

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        let incremental = r#"{"timestamp":"2026-01-14T07:24:24.629Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":2}}}}
"#;
        file.write_all(incremental.as_bytes()).unwrap();

        let logs = parse_changed_content(
            &path,
            &path_str,
            initial.len() as u64,
            "codex",
            &adapter,
            &mut cache,
        )
        .unwrap();

        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].model_id, "gpt-5.4");
        assert_eq!(logs[0].token_count, 2);
    }

    #[test]
    fn codex_incremental_model_switch_updates_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let initial = r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}
"#;
        std::fs::write(&path, initial).unwrap();
        let path_str = path.to_string_lossy().to_string();
        let adapter = CodexAdapter;
        let mut cache = HashMap::from([(path_str.clone(), "gpt-5.4".to_string())]);

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        let incremental = r#"{"type":"turn_context","payload":{"model":"gpt-5.5"}}
{"timestamp":"2026-01-14T07:24:24.629Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":2}}}}
"#;
        file.write_all(incremental.as_bytes()).unwrap();

        let logs = parse_changed_content(
            &path,
            &path_str,
            initial.len() as u64,
            "codex",
            &adapter,
            &mut cache,
        )
        .unwrap();

        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].model_id, "gpt-5.5");
        assert_eq!(cache.get(&path_str).map(String::as_str), Some("gpt-5.5"));
    }

    #[test]
    fn reconcile_registered_paths_reads_growth_without_notify_event() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let initial = r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}
"#;
        std::fs::write(&path, initial).unwrap();
        let path_str = path.to_string_lossy().to_string();

        let incremental = r#"{"timestamp":"2026-01-14T07:24:24.629Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":2}}}}
"#;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(incremental.as_bytes()).unwrap();
        file.flush().unwrap();

        let adapter_names = vec!["codex".to_string()];
        let adapters: Vec<Box<dyn crate::adapters::AgentAdapter>> = vec![Box::new(CodexAdapter)];
        let path_to_adapter = HashMap::from([(path_str.clone(), 0usize)]);
        let mut file_offsets = HashMap::from([(path_str.clone(), initial.len() as u64)]);
        let mut codex_model_cache = HashMap::new();
        let (write_tx, write_rx) = std::sync::mpsc::channel();

        reconcile_registered_paths(
            &adapter_names,
            &adapters,
            &path_to_adapter,
            &mut file_offsets,
            &mut codex_model_cache,
            &write_tx,
        );

        match write_rx.recv().unwrap() {
            WriteRequest::InsertTokenLogs(logs) => {
                assert_eq!(logs.len(), 1);
                assert_eq!(logs[0].model_id, "gpt-5.4");
                assert_eq!(logs[0].token_count, 2);
            }
            _ => panic!("expected token logs"),
        }
        match write_rx.recv().unwrap() {
            WriteRequest::UpdateOffset { file_path, offset } => {
                assert_eq!(file_path, path_str);
                assert_eq!(offset, (initial.len() + incremental.len()) as u64);
            }
            _ => panic!("expected offset update"),
        }
    }
}
