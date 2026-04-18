use std::collections::HashMap;
use std::path::Path;

use rusqlite::OpenFlags;

/// 从数据库加载所有文件偏移量
pub fn load_offsets_from_db(db_path: &Path) -> HashMap<String, u64> {
    let mut offsets = HashMap::new();
    let conn =
        match rusqlite::Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(c) => c,
            Err(_) => return offsets,
        };
    let mut stmt = match conn.prepare("SELECT file_path, last_offset FROM file_offsets") {
        Ok(s) => s,
        Err(_) => return offsets,
    };
    if let Ok(rows) = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    }) {
        for row in rows.flatten() {
            offsets.insert(row.0, row.1 as u64);
        }
    }
    offsets
}
