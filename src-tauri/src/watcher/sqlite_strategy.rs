use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use crate::adapters::{all_agents, AgentDataBatch};
use crate::db::WriteRequest;
use crate::watcher::BehaviorRuntime;

/// 定时 SQLite 轮询策略（用于 OpenCode 等使用外部 DB 的适配器）
pub fn run_sqlite_polling(
    agent_name: String,
    db_path: PathBuf,
    write_tx: Sender<WriteRequest>,
    stop_flag: Arc<AtomicBool>,
    poll_interval_secs: u32,
    initial_offset: Option<u64>,
    behavior_runtime: Option<BehaviorRuntime>,
) {
    let agents = all_agents();
    let agent = match agents.iter().find(|a| a.agent_name() == agent_name) {
        Some(a) => a,
        None => return,
    };

    let mut last_offset = initial_offset;
    let offset_key = super::sqlite_offset_key(&db_path);

    loop {
        // 可中断的等待
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

        if !db_path.exists() {
            continue;
        }

        let query_result = agent
            .query_sqlite_rows(&db_path, last_offset)
            .map(|row_batch| {
                let high_watermark = row_batch.high_watermark;
                let batch = AgentDataBatch::SqliteRows {
                    agent_name: agent_name.clone(),
                    source_key: offset_key.clone(),
                    db_path: db_path.clone(),
                    rows: row_batch.rows,
                    previous_watermark: last_offset,
                    next_watermark: high_watermark,
                };
                let logs = agent.extract_tokens(&batch).logs;
                let behavior_events = if behavior_runtime
                    .as_ref()
                    .is_some_and(BehaviorRuntime::is_enabled)
                {
                    agent.extract_behavior(&batch)
                } else {
                    Vec::new()
                };

                (logs, behavior_events, high_watermark)
            });

        match query_result {
            Ok((logs, behavior_events, high_watermark)) => {
                let Some(offset) = high_watermark else {
                    continue;
                };
                let has_logs = !logs.is_empty();
                let total_tokens: i64 = logs.iter().map(|l| l.token_count).sum();
                let total_cost: f64 = logs.iter().filter_map(|l| l.cost).sum();

                if !has_logs {
                    last_offset = Some(offset);
                    let _ = write_tx.send(WriteRequest::UpdateOffset {
                        file_path: offset_key.clone(),
                        offset,
                    });
                    if let Some(runtime) = behavior_runtime.as_ref() {
                        if runtime.is_enabled() {
                            for event in behavior_events {
                                runtime.dispatcher.handle_event(event);
                            }
                        }
                    }
                    continue;
                }

                log::info!(
                    "[sqlite] {}: 轮询获取 {} 条新记录, {} tokens, cost=${:.4}",
                    agent_name,
                    logs.len(),
                    total_tokens,
                    total_cost
                );
                if super::insert_logs_and_update_offset(&write_tx, logs, offset_key.clone(), offset)
                {
                    if let Some(runtime) = behavior_runtime.as_ref() {
                        if runtime.is_enabled() {
                            for event in behavior_events {
                                runtime.dispatcher.handle_event(event);
                            }
                        }
                    }
                    last_offset = Some(offset);
                }
            }
            Err(e) => {
                log::warn!("{}: SQLite 轮询出错: {}", agent_name, e);
            }
        }
    }
}
