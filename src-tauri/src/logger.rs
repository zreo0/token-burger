use log::{LevelFilter, Log, Metadata, Record};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 同时输出到终端（stderr）和日志文件的 Logger。
/// 不依赖 RUST_LOG 环境变量，始终记录 token_burger 的日志。
pub struct DualLogger {
    file: Mutex<Option<File>>,
    log_dir: PathBuf,
    current_date: Mutex<String>,
}

impl DualLogger {
    fn new(log_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&log_dir);
        cleanup_old_logs(&log_dir, 3);

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let file = open_log_file(&log_dir, &today);

        Self {
            file: Mutex::new(file),
            log_dir,
            current_date: Mutex::new(today),
        }
    }

    /// 检查是否跨天，必要时切换日志文件。
    fn ensure_current_file(&self) {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let mut current_date = self.current_date.lock().unwrap();
        if *current_date == today {
            return;
        }

        *current_date = today.clone();
        let mut file = self.file.lock().unwrap();
        *file = open_log_file(&self.log_dir, &today);
        cleanup_old_logs(&self.log_dir, 3);
    }
}

impl Log for DualLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        // 只记录 token_burger 自身的日志，不过滤级别（由 set_max_level 控制）
        metadata.target().starts_with("token_burger")
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        self.ensure_current_file();

        let timestamp = chrono::Local::now()
            .format("%Y-%m-%dT%H:%M:%S%.3f%:z")
            .to_string();
        // JSON Lines 格式，便于 jq 解析
        let message = format!("{}", record.args())
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let line = format!(
            "{{\"ts\":\"{}\",\"level\":\"{}\",\"mod\":\"{}\",\"msg\":\"{}\"}}\n",
            timestamp,
            record.level(),
            record.target().trim_start_matches("token_burger::"),
            message
        );

        // 写终端
        eprint!("{}", line);

        // 写文件
        if let Ok(mut file) = self.file.lock() {
            if let Some(file) = file.as_mut() {
                let _ = file.write_all(line.as_bytes());
                let _ = file.flush();
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            if let Some(file) = file.as_mut() {
                let _ = file.flush();
            }
        }
    }
}

fn open_log_file(dir: &Path, date: &str) -> Option<File> {
    let path = dir.join(format!("{}.log", date));
    OpenOptions::new().create(true).append(true).open(path).ok()
}

/// 删除保留天数之外的旧日志。
fn cleanup_old_logs(dir: &Path, keep_days: i64) {
    let cutoff = (chrono::Local::now() - chrono::Duration::days(keep_days))
        .format("%Y-%m-%d")
        .to_string();

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".log") {
                continue;
            }

            let date_part = name.trim_end_matches(".log");
            if date_part < cutoff.as_str() {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// 获取日志目录。
fn get_log_dir() -> PathBuf {
    #[cfg(debug_assertions)]
    {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest_dir
            .parent()
            .unwrap_or(manifest_dir.as_path())
            .join("logs")
    }

    #[cfg(not(debug_assertions))]
    {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("token-burger")
            .join("logs")
    }
}

/// 初始化日志系统。
pub fn init() {
    let logger = DualLogger::new(get_log_dir());
    let _ = log::set_boxed_logger(Box::new(logger));
    log::set_max_level(LevelFilter::Debug);
}
