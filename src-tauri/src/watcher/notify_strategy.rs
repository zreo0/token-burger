use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebouncedEvent};

use crate::adapters::{all_agents, AgentDataBatch, AgentPipeline};
use crate::behavior::dispatcher::BehaviorDispatcher;
use crate::db::WriteRequest;
use crate::watcher::BehaviorRuntime;

struct PathChangeContext<'a> {
    agent_names: &'a [String],
    agents: &'a [Box<dyn AgentPipeline>],
    path_to_adapter: &'a HashMap<String, usize>,
    write_tx: &'a Sender<WriteRequest>,
    behavior: Option<BehaviorRuntime>,
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
    behavior: Option<BehaviorRuntime>,
) {
    // path → adapter index
    let mut path_to_adapter: HashMap<String, usize> = HashMap::new();
    // path → file size（用于增量读取），初始化时优先使用 DB 中的 offset
    let mut file_offsets: HashMap<String, u64> = initial_offsets;
    let mut codex_model_cache: HashMap<String, String> = HashMap::new();
    let agents = all_agents();

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
                    &agents,
                    &path_to_adapter,
                    &mut file_offsets,
                    &mut codex_model_cache,
                    &write_tx,
                    behavior.clone(),
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
                        &agents,
                        &path_to_adapter,
                        &mut file_offsets,
                        &mut codex_model_cache,
                        &write_tx,
                        behavior.clone(),
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
    agent_names: &[String],
    agents: &[Box<dyn AgentPipeline>],
    path_to_adapter: &HashMap<String, usize>,
    file_offsets: &mut HashMap<String, u64>,
    codex_model_cache: &mut HashMap<String, String>,
    write_tx: &Sender<WriteRequest>,
    behavior: Option<BehaviorRuntime>,
) {
    let context = PathChangeContext {
        agent_names,
        agents,
        path_to_adapter,
        write_tx,
        behavior,
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
    agent_names: &[String],
    agents: &[Box<dyn AgentPipeline>],
    path_to_adapter: &HashMap<String, usize>,
    file_offsets: &mut HashMap<String, u64>,
    codex_model_cache: &mut HashMap<String, String>,
    write_tx: &Sender<WriteRequest>,
    behavior: Option<BehaviorRuntime>,
) {
    let context = PathChangeContext {
        agent_names,
        agents,
        path_to_adapter,
        write_tx,
        behavior,
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
    let agent_name = &context.agent_names[adapter_idx];
    let agent = match context.agents.iter().find(|a| a.agent_name() == agent_name) {
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
        let batch = AgentDataBatch::JsonlIncrement {
            agent_name: agent_name.to_string(),
            source_key: path_str.to_string(),
            path: path.to_path_buf(),
            content,
            token_context: None,
            initial_model: None,
            previous_offset: 0,
            next_offset: new_size,
        };
        let extraction = agent.extract_tokens(&batch);
        if let Some(final_model) = extraction.final_model {
            codex_model_cache.insert(path_str.to_string(), final_model);
        }
        let logs = extraction.logs;
        if !logs.is_empty() {
            log::info!(
                "[{}] {}: 解析 {} 条记录 (轮转重读)",
                source,
                agent_name,
                logs.len()
            );
            let _ = context.write_tx.send(WriteRequest::InsertTokenLogs(logs));
        }
        dispatch_behavior_events(agent.as_ref(), &batch, &context.behavior);
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

    let batch =
        match build_changed_batch(path, path_str, prev_offset, agent_name, codex_model_cache) {
            Ok(batch) => batch,
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
    let extraction = agent.extract_tokens(&batch);
    if let Some(final_model) = extraction.final_model {
        codex_model_cache.insert(path_str.to_string(), final_model);
    }
    let logs = extraction.logs;
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
    dispatch_behavior_events(agent.as_ref(), &batch, &context.behavior);
    file_offsets.insert(path_str.to_string(), new_size);
    let _ = context.write_tx.send(WriteRequest::UpdateOffset {
        file_path: path_str.to_string(),
        offset: new_size,
    });
}

fn build_changed_batch(
    path: &std::path::Path,
    path_str: &str,
    prev_offset: u64,
    agent_name: &str,
    codex_model_cache: &mut HashMap<String, String>,
) -> std::io::Result<AgentDataBatch> {
    let next_offset = std::fs::metadata(path)?.len();
    if agent_name != "codex" {
        let content = read_from_offset(path, prev_offset)?;
        return Ok(AgentDataBatch::JsonlIncrement {
            agent_name: agent_name.to_string(),
            source_key: path_str.to_string(),
            path: path.to_path_buf(),
            content,
            token_context: None,
            initial_model: None,
            previous_offset: prev_offset,
            next_offset,
        });
    }

    let behavior_content = read_from_offset(path, prev_offset)?;
    let (token_context, initial_model) = match codex_model_cache.get(path_str) {
        Some(model) => (None, Some(model.clone())),
        None => (
            Some(std::fs::read_to_string(path)?),
            Some(crate::adapters::codex::DEFAULT_CODEX_MODEL.to_string()),
        ),
    };

    Ok(AgentDataBatch::JsonlIncrement {
        agent_name: agent_name.to_string(),
        source_key: path_str.to_string(),
        path: path.to_path_buf(),
        content: behavior_content,
        token_context,
        initial_model,
        previous_offset: prev_offset,
        next_offset,
    })
}

fn dispatch_behavior_events(
    agent: &dyn AgentPipeline,
    batch: &AgentDataBatch,
    behavior: &Option<BehaviorRuntime>,
) {
    let Some(runtime) = behavior else {
        return;
    };
    dispatch_behavior_events_if_enabled(
        agent,
        batch,
        runtime.is_enabled(),
        Some(&runtime.dispatcher),
    );
}

fn dispatch_behavior_events_if_enabled(
    agent: &dyn AgentPipeline,
    batch: &AgentDataBatch,
    enabled: bool,
    dispatcher: Option<&Arc<BehaviorDispatcher>>,
) {
    if !enabled {
        return;
    };
    let Some(dispatcher) = dispatcher else {
        return;
    };

    for event in agent.extract_behavior(batch) {
        dispatcher.handle_event(event);
    }
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
    use crate::adapters::{
        AgentSource, BehaviorExtractor, DataSource, TokenExtraction, TokenExtractor,
    };
    use std::io::Write;
    use std::sync::atomic::AtomicUsize;

    fn extract_codex_logs(
        adapter: &CodexAdapter,
        batch: &AgentDataBatch,
        cache: &mut HashMap<String, String>,
    ) -> Vec<crate::adapters::TokenLog> {
        let extraction = adapter.extract_tokens(batch);
        if let Some(final_model) = extraction.final_model {
            cache.insert(batch.source_key().to_string(), final_model);
        }
        extraction.logs
    }

    struct CountingBehaviorAgent {
        calls: Arc<AtomicUsize>,
    }

    impl AgentSource for CountingBehaviorAgent {
        fn agent_name(&self) -> &str {
            "counting"
        }

        fn data_source(&self) -> DataSource {
            DataSource::Jsonl { paths: Vec::new() }
        }

        fn log_paths(&self) -> Vec<String> {
            Vec::new()
        }
    }

    impl TokenExtractor for CountingBehaviorAgent {
        fn extract_tokens(&self, _batch: &AgentDataBatch) -> TokenExtraction {
            TokenExtraction::default()
        }
    }

    impl BehaviorExtractor for CountingBehaviorAgent {
        fn extract_behavior(
            &self,
            _batch: &AgentDataBatch,
        ) -> Vec<crate::behavior::AgentBehaviorEvent> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Vec::new()
        }
    }

    fn behavior_test_batch() -> AgentDataBatch {
        AgentDataBatch::JsonlIncrement {
            agent_name: "counting".to_string(),
            source_key: "/tmp/counting.jsonl".to_string(),
            path: "/tmp/counting.jsonl".into(),
            content: "{}\n".to_string(),
            token_context: None,
            initial_model: None,
            previous_offset: 0,
            next_offset: 3,
        }
    }

    #[test]
    fn behavior_dispatch_skips_extractor_when_runtime_missing_or_disabled() {
        let calls = Arc::new(AtomicUsize::new(0));
        let agent = CountingBehaviorAgent {
            calls: calls.clone(),
        };
        let batch = behavior_test_batch();

        dispatch_behavior_events(&agent, &batch, &None);
        assert_eq!(calls.load(Ordering::Relaxed), 0);

        dispatch_behavior_events_if_enabled(&agent, &batch, false, None);
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }

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

        let batch =
            build_changed_batch(&path, &path_str, initial.len() as u64, "codex", &mut cache)
                .unwrap();
        let logs = extract_codex_logs(&adapter, &batch, &mut cache);

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

        let batch =
            build_changed_batch(&path, &path_str, initial.len() as u64, "codex", &mut cache)
                .unwrap();
        let logs = extract_codex_logs(&adapter, &batch, &mut cache);

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

        let batch =
            build_changed_batch(&path, &path_str, initial.len() as u64, "codex", &mut cache)
                .unwrap();
        let logs = extract_codex_logs(&adapter, &batch, &mut cache);

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
        let adapters: Vec<Box<dyn crate::adapters::AgentPipeline>> = vec![Box::new(CodexAdapter)];
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
            None,
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
