use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebouncedEvent};

use crate::adapters::all_adapters;
use crate::db::WriteRequest;

/// 基于 notify-debouncer-full 的实时监听策略
///
/// 用 inotify/kqueue 真正监听文件变化，500ms debounce 后读取增量内容。
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
                    // 新文件默认从 0 开始，避免冷启动与注册之间的窗口期内容被跳过。
                    if !file_offsets.contains_key(&path) {
                        file_offsets.insert(path.clone(), 0u64);
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
            &mut path_to_adapter,
            &mut file_offsets,
            &mut debouncer,
        );
    }

    // 主循环：优先处理 debounced 事件，定期扫描新路径
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
                    &write_tx,
                );
            }
            Ok(Err(errs)) => {
                for e in errs {
                    log::warn!("notify 错误: {}", e);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // 每 3s 扫描一次 glob 注册新出现的文件
                if since_last_scan.elapsed() >= Duration::from_secs(3) {
                    for (idx, patterns) in log_patterns.iter().enumerate() {
                        register_paths(
                            patterns,
                            idx,
                            &mut path_to_adapter,
                            &mut file_offsets,
                            &mut debouncer,
                        );
                    }
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
    write_tx: &Sender<WriteRequest>,
) {
    for event in events {
        for path in &event.paths {
            let path_str = path.to_string_lossy().to_string();
            let adapter_idx = match path_to_adapter.get(&path_str) {
                Some(idx) => *idx,
                None => continue,
            };
            let agent_name = &adapter_names[adapter_idx];
            let adapter = match adapters.iter().find(|a| a.agent_name() == agent_name) {
                Some(a) => a,
                None => continue,
            };

            let meta = match std::fs::metadata(path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let new_size = meta.len();
            let prev_offset = file_offsets.get(&path_str).copied().unwrap_or(0);

            if new_size < prev_offset {
                // 文件被截断/轮转，从头重新读取
                log::info!(
                    "[notify] {}: 文件轮转检测 offset {}→{}, 从头读取",
                    agent_name,
                    prev_offset,
                    new_size
                );
                if let Ok(content) = read_from_offset(path, 0) {
                    let logs = adapter.parse_content(&content);
                    if !logs.is_empty() {
                        log::info!(
                            "[notify] {}: 解析 {} 条记录 (轮转重读)",
                            agent_name,
                            logs.len()
                        );
                        let _ = write_tx.send(WriteRequest::InsertTokenLogs(logs));
                    }
                }
                file_offsets.insert(path_str.clone(), new_size);
                let _ = write_tx.send(WriteRequest::UpdateOffset {
                    file_path: path_str,
                    offset: new_size,
                });
                continue;
            }

            if new_size == prev_offset {
                // 无变化
                continue;
            }

            // 读取增量内容
            if let Ok(content) = read_from_offset(path, prev_offset) {
                let logs = adapter.parse_content(&content);
                if !logs.is_empty() {
                    // 汇总关键信息：agent、记录数、token 总量
                    let total_tokens: i64 = logs.iter().map(|l| l.token_count).sum();
                    let models: Vec<&str> = logs
                        .iter()
                        .map(|l| l.model_id.as_str())
                        .collect::<std::collections::HashSet<_>>()
                        .into_iter()
                        .collect();
                    log::info!(
                        "[notify] {}: 增量读取 offset {}→{}, {} 条记录, {} tokens, models={:?}",
                        agent_name,
                        prev_offset,
                        new_size,
                        logs.len(),
                        total_tokens,
                        models
                    );
                    let _ = write_tx.send(WriteRequest::InsertTokenLogs(logs));
                }
            }
            file_offsets.insert(path_str.clone(), new_size);
            let _ = write_tx.send(WriteRequest::UpdateOffset {
                file_path: path_str,
                offset: new_size,
            });
        }
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
