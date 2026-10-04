use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant, SystemTime};

use super::{
    cold_start_file_source, flush_writes, notify_strategy, sqlite_strategy::StartupSqlite,
    WatcherConfig,
};
use crate::adapters::{AgentDataBatch, AgentPipeline, DataSource};
use crate::db::WriteRequest;
use crate::types::ColdStartProgress;

/** 启动期间的文件读取位置，仅在历史覆盖完成后持久化实时位置 */
struct StartupFile {
    agent: usize,
    jsonl: bool,
    /** 实时读取起点；历史未补齐前不得直接持久化此位置 */
    offset: u64,
    modified: SystemTime,
    /** 文件起点到实时位置之间的历史已成功入库 */
    covered: bool,
}

/**
 * 同一调度器交替处理新增记录与历史任务，避免两个监听器争用断点
 * 每个历史文件或 SQLite 分页都等待写入确认，防止写队列积压
 */
pub(super) struct StartupScheduler<'a> {
    agents: &'a [Box<dyn AgentPipeline>],
    tx: &'a Sender<WriteRequest>,
    keep_days: u32,
    files: HashMap<String, StartupFile>,
    pending: VecDeque<String>,
    sqlite: Vec<(usize, StartupSqlite)>,
    sqlite_index: usize,
    known: HashMap<String, u64>,
    models: HashMap<String, String>,
    live_interval: Duration,
    last_live: Instant,
    recent_more: bool,
    today: SystemTime,
    pub progress: ColdStartProgress,
}

/** 展开文件 source 的路径并去重，返回真实文件列表 */
pub(super) fn discover_files(agent: &dyn AgentPipeline) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    agent
        .log_paths()
        .iter()
        .flat_map(|pattern| {
            glob::glob(pattern)
                .map(|entries| entries.flatten().collect::<Vec<_>>())
                .unwrap_or_default()
        })
        .filter(|path| {
            path.is_file() && seen.insert(path.canonicalize().unwrap_or_else(|_| path.clone()))
        })
        .collect()
}

impl<'a> StartupScheduler<'a> {
    /** 建立启动快照与待补录任务，参数为已有 pipeline、写通道和设置 */
    pub(super) fn new(
        agents: &'a [Box<dyn AgentPipeline>],
        tx: &'a Sender<WriteRequest>,
        db_path: &Path,
        config: &WatcherConfig,
        known: HashMap<String, u64>,
    ) -> Self {
        let today = chrono::Local::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(chrono::Local)
            .earliest()
            .unwrap_or_else(chrono::Local::now);
        let mut result = Self {
            agents,
            tx,
            keep_days: config.keep_days,
            files: HashMap::new(),
            pending: VecDeque::new(),
            sqlite: Vec::new(),
            sqlite_index: 0,
            known,
            models: HashMap::new(),
            live_interval: Duration::from_secs(if config.watch_mode == "realtime" {
                1
            } else {
                u64::from(config.polling_interval_secs.max(1))
            }),
            last_live: Instant::now() - Duration::from_secs(86400),
            recent_more: false,
            today: today.into(),
            progress: ColdStartProgress {
                phase: "recent".to_string(),
                ..Default::default()
            },
        };
        result.discover(true);
        for (index, agent) in agents.iter().enumerate() {
            if let DataSource::Sqlite { db_path: path } = agent.data_source() {
                if !path.exists() {
                    continue;
                }
                let since = result.known.get(&super::sqlite_offset_key(&path)).copied();
                match StartupSqlite::new(
                    agent.as_ref(),
                    &path,
                    db_path,
                    tx,
                    since,
                    config.keep_days,
                ) {
                    Ok(source) => {
                        result.progress.total += source.remaining() as u32;
                        result.sqlite.push((index, source));
                    }
                    Err(error) => result.record_error(error.to_string()),
                }
            }
        }
        result
    }

    /** 发现新文件；初始边界用于区分既有历史与启动后新增内容 */
    fn discover(&mut self, initial: bool) {
        let mut added = Vec::new();
        for (index, agent) in self.agents.iter().enumerate() {
            let jsonl = match agent.data_source() {
                DataSource::Jsonl { .. } => true,
                DataSource::Json { .. } => false,
                DataSource::Sqlite { .. } => continue,
            };
            for path in discover_files(agent.as_ref()) {
                let key = path.to_string_lossy().to_string();
                if self.files.contains_key(&key) {
                    continue;
                }
                let Ok(meta) = path.metadata() else {
                    continue;
                };
                let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                let offset = if initial && jsonl {
                    jsonl_boundary(&path, meta.len()).unwrap_or(0)
                } else if initial {
                    meta.len()
                } else {
                    0
                };
                self.files.insert(
                    key.clone(),
                    StartupFile {
                        agent: index,
                        jsonl,
                        offset,
                        modified,
                        covered: false,
                    },
                );
                added.push((modified, key));
            }
        }
        // 只用 mtime 排定读取优先级，是否属于今天仍由记录时间决定
        added.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        if initial {
            self.progress.total += added.len() as u32;
            self.pending.extend(added.into_iter().map(|(_, key)| key));
        }
    }

    /** 执行一次优先采集与一个历史任务，返回是否仍有待处理任务 */
    pub(super) fn step(&mut self) -> bool {
        if self.recent_more || self.last_live.elapsed() >= self.live_interval {
            self.poll_live();
        }
        if let Some(path) = self.pending.pop_front() {
            let file = &self.files[&path];
            let agent = file.agent;
            let jsonl = file.jsonl;
            self.progress.agent = self.agents[agent].agent_name().to_string();
            self.progress.phase = if file.modified >= self.today || self.recent_more {
                "recent"
            } else {
                "history"
            }
            .to_string();
            let mut errors = 0;
            let updated = cold_start_file_source(
                self.agents[agent].as_ref(),
                self.tx,
                self.keep_days,
                &self.known,
                vec![PathBuf::from(&path)],
                jsonl,
                &mut |failed| {
                    if failed {
                        errors += 1;
                    }
                },
            );
            self.progress.errors += errors;
            match flush_writes(self.tx) {
                Ok(()) if errors == 0 => {
                    self.known.extend(updated);
                    let file = self.files.get_mut(&path).unwrap();
                    if let Some(offset) = self.known.get(&path) {
                        file.offset = *offset;
                        file.covered = true;
                    }
                }
                Ok(()) => {}
                Err(error) => self.record_error(error),
            }
            self.progress.files_checked += 1;
            self.progress.completed += 1;
            return true;
        }
        for _ in 0..self.sqlite.len() {
            let index = self.sqlite_index % self.sqlite.len();
            self.sqlite_index += 1;
            let (agent, source) = &mut self.sqlite[index];
            if source.remaining() == 0 {
                continue;
            }
            self.progress.agent = self.agents[*agent].agent_name().to_string();
            self.progress.phase = if self.recent_more {
                "recent"
            } else {
                "history"
            }
            .to_string();
            let before = source.remaining();
            let outcome = source.history_step(self.agents[*agent].as_ref(), self.tx);
            self.progress.completed += (before - source.remaining()) as u32;
            if let Err(error) = outcome {
                self.record_error(error.to_string());
            }
            return true;
        }
        if self.recent_more {
            return true;
        }
        self.progress.phase = "writing".to_string();
        false
    }

    /** 优先读取启动后变动的文件和今日 SQLite 记录，不推进未补齐历史的断点 */
    fn poll_live(&mut self) {
        self.discover(false);
        let paths = self.files.keys().cloned().collect::<Vec<_>>();
        for path in paths {
            if let Err(error) = self.poll_file(&path) {
                self.record_error(error.to_string());
            }
        }
        self.recent_more = false;
        for index in 0..self.sqlite.len() {
            let (agent, source) = &mut self.sqlite[index];
            match source.poll_recent(self.agents[*agent].as_ref(), self.tx) {
                Ok(more) => self.recent_more |= more,
                Err(error) => self.record_error(error.to_string()),
            }
        }
        self.progress.live = true;
        self.last_live = Instant::now();
    }

    /** 采集一个文件新增内容，成功提交后推进内存位置，返回读取或写入错误 */
    fn poll_file(&mut self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let file = self.files.get_mut(path).unwrap();
        let meta = match std::fs::metadata(path) {
            Ok(meta) => meta,
            Err(_) => return Ok(()),
        };
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if meta.len() == file.offset && modified == file.modified {
            return Ok(());
        }
        let agent = self.agents[file.agent].as_ref();
        let batch = if file.jsonl {
            let offset = if meta.len() <= file.offset {
                self.models.remove(path);
                0
            } else {
                file.offset
            };
            notify_strategy::build_changed_batch(
                Path::new(path),
                path,
                offset,
                agent.agent_name(),
                &mut self.models,
            )?
        } else {
            AgentDataBatch::JsonDocument {
                agent_name: agent.agent_name().to_string(),
                source_key: path.to_string(),
                path: PathBuf::from(path),
                content: std::fs::read_to_string(path)?,
                mtime: modified
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            }
        };
        let extraction = agent.extract_tokens(&batch);
        if let Some(model) = extraction.final_model {
            self.models.insert(path.to_string(), model);
        }
        if !extraction.logs.is_empty() {
            self.tx
                .send(WriteRequest::InsertTokenLogs(extraction.logs))?;
            flush_writes(self.tx)?;
        }
        file.offset = match batch {
            AgentDataBatch::JsonlIncrement {
                previous_offset,
                next_offset,
                ..
            } => {
                file.covered |= previous_offset == 0;
                next_offset
            }
            AgentDataBatch::JsonDocument { content, .. } => {
                file.covered = true;
                content.len() as u64
            }
            _ => unreachable!(),
        };
        file.modified = modified;
        Ok(())
    }

    /** 最后采集一次新增内容，确认历史覆盖后保存实时断点，返回监听接续位置 */
    pub(super) fn finish(&mut self) -> HashMap<String, u64> {
        self.poll_live();
        for index in 0..self.sqlite.len() {
            if let Err(error) = self.sqlite[index].1.finish(self.tx) {
                self.record_error(error.to_string());
            }
        }
        for (path, file) in &self.files {
            if file.covered {
                let _ = self.tx.send(WriteRequest::UpdateOffset {
                    file_path: path.clone(),
                    offset: file.offset,
                });
                self.known.insert(path.clone(), file.offset);
            } else {
                self.known.entry(path.clone()).or_insert(0);
            }
        }
        if let Err(error) = flush_writes(self.tx) {
            self.record_error(error);
        }
        self.known.clone()
    }

    /** 记录加载失败，保留界面上的部分数据提示 */
    fn record_error(&mut self, error: String) {
        self.progress.errors += 1;
        log::warn!("启动后台同步未完成部分数据: {}", error);
    }
}

/** 找到启动快照最后一个完整 JSONL 行的字节边界，避免从半个 UTF-8 字符开始读取 */
fn jsonl_boundary(path: &Path, end: u64) -> std::io::Result<u64> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let mut position = end;
    while position > 0 {
        let start = position.saturating_sub(8192);
        file.seek(SeekFrom::Start(start))?;
        let mut buffer = vec![0; (position - start) as usize];
        file.read_exact(&mut buffer)?;
        if let Some(index) = buffer.iter().rposition(|byte| *byte == b'\n') {
            return Ok(start + index as u64 + 1);
        }
        position = start;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{
        AgentSource, BehaviorExtractor, TokenExtraction, TokenExtractor, TokenLog, TokenType,
    };
    use crate::db::{init_db, queries};
    use std::io::Write;

    struct FileAgent {
        root: PathBuf,
    }
    impl AgentSource for FileAgent {
        fn agent_name(&self) -> &str {
            "test-file"
        }
        fn data_source(&self) -> DataSource {
            DataSource::Jsonl {
                paths: vec![self.root.clone()],
            }
        }
        fn log_paths(&self) -> Vec<String> {
            vec![format!("{}/*.jsonl", self.root.display())]
        }
    }
    impl TokenExtractor for FileAgent {
        fn extract_tokens(&self, batch: &AgentDataBatch) -> TokenExtraction {
            TokenExtraction::from_logs(
                batch
                    .token_content()
                    .unwrap()
                    .lines()
                    .map(|id| TokenLog {
                        id: None,
                        agent_name: "test-file".into(),
                        provider: "test".into(),
                        model_id: "test".into(),
                        token_type: TokenType::Input,
                        token_count: 10,
                        session_id: None,
                        request_id: Some(id.into()),
                        latency_ms: None,
                        is_error: false,
                        metadata: None,
                        cost: None,
                        timestamp: chrono::Utc::now().to_rfc3339(),
                    })
                    .collect(),
            )
        }
    }
    impl BehaviorExtractor for FileAgent {
        fn extract_behavior(&self, _: &AgentDataBatch) -> Vec<crate::behavior::AgentBehaviorEvent> {
            panic!("启动补录和启动期采集不得回放行为提醒");
        }
    }

    /** 使用真实 SQLite 写入验证调度顺序与去重，返回通道和写线程 */
    fn writer(path: PathBuf) -> (Sender<WriteRequest>, std::thread::JoinHandle<()>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let conn = init_db(&path).unwrap();
            while let Ok(request) = rx.recv() {
                match request {
                    WriteRequest::InsertTokenLogs(logs) => {
                        queries::batch_insert_token_logs(&conn, &logs).unwrap()
                    }
                    WriteRequest::InsertFileLogs {
                        logs,
                        file_path,
                        offset,
                    } => queries::batch_insert_token_logs_and_update_offset(
                        &conn, &logs, &file_path, offset,
                    )
                    .unwrap(),
                    WriteRequest::UpdateOffset { file_path, offset } => {
                        queries::update_offset(&conn, &file_path, offset).unwrap()
                    }
                    WriteRequest::Flush(reply) => reply.send(Ok(())).unwrap(),
                    _ => panic!("unexpected file request"),
                }
            }
        });
        (tx, handle)
    }

    /** 历史尚未完成时，今日与新追加记录已经入库；重启后无遗漏或重复 */
    #[test]
    fn live_usage_preempts_history_without_skipping_it_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.jsonl");
        let today = dir.path().join("today.jsonl");
        std::fs::write(&old, "old-1\n").unwrap();
        std::fs::File::open(&old)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(86400))
            .unwrap();
        std::fs::write(&today, "today-1\n").unwrap();
        let db_path = dir.path().join("local.db");
        let conn = init_db(&db_path).unwrap();
        let (tx, worker) = writer(db_path.clone());
        let agents: Vec<Box<dyn AgentPipeline>> = vec![Box::new(FileAgent {
            root: dir.path().to_path_buf(),
        })];
        let config = WatcherConfig {
            keep_days: 90,
            watch_mode: "realtime".into(),
            polling_interval_secs: 10,
        };
        let mut scheduler = StartupScheduler::new(&agents, &tx, &db_path, &config, HashMap::new());
        assert!(scheduler.step());
        assert_eq!(scheduler.progress.completed, 1);
        assert_eq!(
            queries::get_token_summary(&conn, "today").unwrap().total,
            10
        );
        std::fs::OpenOptions::new()
            .append(true)
            .open(&old)
            .unwrap()
            .write_all(b"live-1\n")
            .unwrap();
        scheduler.poll_live();
        assert_eq!(
            queries::get_token_summary(&conn, "today").unwrap().total,
            20
        );
        // 新增记录可见，但旧文件历史尚未读取，持久化断点不能跳到文件尾
        assert_eq!(
            queries::get_offset(&conn, &old.to_string_lossy()).unwrap(),
            None
        );
        drop(scheduler);
        let mut restarted = StartupScheduler::new(
            &agents,
            &tx,
            &db_path,
            &config,
            super::super::offset::load_offsets_from_db(&db_path),
        );
        while restarted.step() {}
        let offsets = restarted.finish();
        assert_eq!(
            queries::get_token_summary(&conn, "today").unwrap().total,
            30
        );
        assert_eq!(
            offsets[&old.to_string_lossy().to_string()],
            std::fs::metadata(&old).unwrap().len()
        );
        drop(restarted);
        drop(tx);
        worker.join().unwrap();
    }

    /** 半个 UTF-8 字符或半行不会被推进断点，续写后能够完整读取 */
    #[test]
    fn partial_line_keeps_boundary_until_append_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.jsonl");
        std::fs::write(&path, [b"ok\n".as_slice(), &[0xe6, 0x96]].concat()).unwrap();
        assert_eq!(jsonl_boundary(&path, 5).unwrap(), 3);
        assert_eq!(
            notify_strategy::read_complete_jsonl(&path, 0).unwrap(),
            "ok\n"
        );
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&[0xb0, b'\n'])
            .unwrap();
        assert_eq!(
            notify_strategy::read_complete_jsonl(&path, 3).unwrap(),
            "新\n"
        );
    }
}
