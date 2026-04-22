use std::collections::HashMap;

use rusqlite::{params, Connection};

use crate::adapters::TokenLog;
use crate::types::TokenSummary;

/// 批量插入 TokenLog（1000 条分批事务）
pub fn batch_insert_token_logs(
    conn: &Connection,
    logs: &[TokenLog],
) -> Result<(), rusqlite::Error> {
    const BATCH_SIZE: usize = 1000;

    for chunk in logs.chunks(BATCH_SIZE) {
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO token_logs
                 (agent_name, provider, model_id, token_type, token_count,
                  session_id, request_id, latency_ms, is_error, metadata, cost, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )?;

            for log in chunk {
                let token_type_str = serde_json::to_value(&log.token_type)
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| "input".to_string());

                stmt.execute(params![
                    log.agent_name,
                    log.provider,
                    log.model_id,
                    token_type_str,
                    log.token_count,
                    log.session_id,
                    log.request_id,
                    log.latency_ms,
                    log.is_error as i32,
                    log.metadata,
                    log.cost,
                    log.timestamp,
                ])?;
            }
        }
        tx.commit()?;
    }
    Ok(())
}

/// 获取指定时间范围的 token 汇总
pub fn get_token_summary(conn: &Connection, range: &str) -> Result<TokenSummary, rusqlite::Error> {
    let time_filter = match range {
        "today" => "date(timestamp, 'localtime') = date('now', 'localtime')",
        "7d" => "datetime(timestamp, 'localtime') >= datetime('now', '-7 days', 'localtime')",
        "30d" => "datetime(timestamp, 'localtime') >= datetime('now', '-30 days', 'localtime')",
        _ => "date(timestamp, 'localtime') = date('now', 'localtime')",
    };

    let sql = format!(
        "SELECT token_type, COALESCE(SUM(token_count), 0)
         FROM token_logs WHERE {} GROUP BY token_type",
        time_filter
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;

    let mut input = 0i64;
    let mut cache_create = 0i64;
    let mut cache_read = 0i64;
    let mut output = 0i64;

    for row in rows {
        let (token_type, count) = row?;
        match token_type.as_str() {
            "input" => input = count,
            "cache_create" => cache_create = count,
            "cache_read" => cache_read = count,
            "output" => output = count,
            _ => {}
        }
    }

    // 按 agent 分组
    let by_agent = get_grouped_summary(conn, time_filter, "agent_name")?;
    // 按 model 分组
    let by_model = get_grouped_summary(conn, time_filter, "model_id")?;
    let agent_cost = get_agent_cost_summary(conn, range)?;

    Ok(TokenSummary {
        input,
        cache_create,
        cache_read,
        output,
        total: input + cache_create + cache_read + output,
        agent_cost,
        by_agent,
        by_model,
    })
}

/// 获取指定时间范围内 agent 自带 cost 的汇总
pub fn get_agent_cost_summary(conn: &Connection, range: &str) -> Result<f64, rusqlite::Error> {
    let time_filter = match range {
        "today" => "date(timestamp, 'localtime') = date('now', 'localtime')",
        "7d" => "datetime(timestamp, 'localtime') >= datetime('now', '-7 days', 'localtime')",
        "30d" => "datetime(timestamp, 'localtime') >= datetime('now', '-30 days', 'localtime')",
        _ => "date(timestamp, 'localtime') = date('now', 'localtime')",
    };

    let sql = format!(
        "SELECT COALESCE(SUM(cost), 0.0) FROM token_logs WHERE {} AND cost IS NOT NULL",
        time_filter
    );

    conn.query_row(&sql, [], |row| row.get(0))
}

/// 按指定字段分组查询 token 汇总
fn get_grouped_summary(
    conn: &Connection,
    time_filter: &str,
    group_field: &str,
) -> Result<HashMap<String, crate::types::TokenBreakdown>, rusqlite::Error> {
    let sql = format!(
        "SELECT {}, token_type, COALESCE(SUM(token_count), 0), COALESCE(SUM(cost), 0.0)
         FROM token_logs WHERE {} GROUP BY {}, token_type",
        group_field, time_filter, group_field
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, f64>(3)?,
        ))
    })?;

    let mut result: HashMap<String, crate::types::TokenBreakdown> = HashMap::new();
    for row in rows {
        let (key, token_type, count, cost) = row?;
        let entry = result.entry(key).or_default();
        entry.agent_cost += cost;
        match token_type.as_str() {
            "input" => entry.input = count,
            "cache_create" => entry.cache_create = count,
            "cache_read" => entry.cache_read = count,
            "output" => entry.output = count,
            _ => {}
        }
    }
    Ok(result)
}

/// 清理数据
pub fn clear_data(conn: &Connection, keep_days: Option<u32>) -> Result<(), rusqlite::Error> {
    match keep_days {
        Some(days) => {
            conn.execute(
                &format!(
                    "DELETE FROM token_logs WHERE datetime(timestamp, 'localtime') < datetime('now', '-{} days', 'localtime')",
                    days
                ),
                [],
            )?;
        }
        None => {
            conn.execute("DELETE FROM token_logs", [])?;
            conn.execute("DELETE FROM file_offsets", [])?;
        }
    }
    Ok(())
}

/// 获取文件偏移量
#[allow(dead_code)]
pub fn get_offset(conn: &Connection, file_path: &str) -> Result<Option<u64>, rusqlite::Error> {
    let mut stmt = conn.prepare("SELECT last_offset FROM file_offsets WHERE file_path = ?1")?;
    let result = stmt
        .query_row(params![file_path], |row| row.get::<_, i64>(0))
        .ok()
        .map(|v| v as u64);
    Ok(result)
}

/// 更新文件偏移量
#[allow(dead_code)]
pub fn update_offset(
    conn: &Connection,
    file_path: &str,
    offset: u64,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT OR REPLACE INTO file_offsets (file_path, last_offset, updated_at)
         VALUES (?1, ?2, CURRENT_TIMESTAMP)",
        params![file_path, offset as i64],
    )?;
    Ok(())
}

/// 获取单个设置
pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>, rusqlite::Error> {
    let mut stmt = conn.prepare("SELECT value FROM app_settings WHERE key = ?1")?;
    let result = stmt
        .query_row(params![key], |row| row.get::<_, String>(0))
        .ok();
    Ok(result)
}

/// 设置单个设置值
pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT OR REPLACE INTO app_settings (key, value, updated_at)
         VALUES (?1, ?2, CURRENT_TIMESTAMP)",
        params![key, value],
    )?;
    Ok(())
}

/// 获取所有设置
#[allow(dead_code)]
pub fn get_all_settings(conn: &Connection) -> Result<HashMap<String, String>, rusqlite::Error> {
    let mut stmt = conn.prepare("SELECT key, value FROM app_settings")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut settings = HashMap::new();
    for row in rows {
        let (key, value) = row?;
        settings.insert(key, value);
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{TokenLog, TokenType};

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::SCHEMA_SQL).unwrap();
        conn
    }

    fn make_log(agent: &str, model: &str, tt: TokenType, count: i64, req_id: &str) -> TokenLog {
        TokenLog {
            id: None,
            agent_name: agent.into(),
            provider: "TestProvider".into(),
            model_id: model.into(),
            token_type: tt,
            token_count: count,
            session_id: Some("sess-1".into()),
            request_id: Some(req_id.into()),
            latency_ms: None,
            is_error: false,
            metadata: None,
            cost: None,
            timestamp: chrono::Local::now()
                .format("%Y-%m-%dT%H:%M:%S%:z")
                .to_string(),
        }
    }

    #[test]
    fn test_batch_insert_and_dedup() {
        let conn = setup_db();
        let logs = vec![
            make_log(
                "claude-code",
                "claude-3-7-sonnet",
                TokenType::Input,
                100,
                "req-1",
            ),
            make_log(
                "claude-code",
                "claude-3-7-sonnet",
                TokenType::Output,
                50,
                "req-1",
            ),
            // 重复记录
            make_log(
                "claude-code",
                "claude-3-7-sonnet",
                TokenType::Input,
                100,
                "req-1",
            ),
        ];

        batch_insert_token_logs(&conn, &logs).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM token_logs", [], |row| row.get(0))
            .unwrap();
        // 第三条重复应被 IGNORE
        assert_eq!(count, 2);
    }

    #[test]
    fn test_token_summary() {
        let conn = setup_db();
        let logs = vec![
            make_log(
                "claude-code",
                "claude-3-7-sonnet",
                TokenType::Input,
                1000,
                "req-1",
            ),
            make_log(
                "claude-code",
                "claude-3-7-sonnet",
                TokenType::Output,
                200,
                "req-1",
            ),
            make_log("codex", "gpt-4o", TokenType::Input, 500, "req-2"),
        ];
        batch_insert_token_logs(&conn, &logs).unwrap();

        let summary = get_token_summary(&conn, "today").unwrap();
        assert_eq!(summary.input, 1500);
        assert_eq!(summary.output, 200);
        assert_eq!(summary.total, 1700);
        assert_eq!(summary.agent_cost, 0.0);
        assert!(summary.by_agent.contains_key("claude-code"));
        assert!(summary.by_model.contains_key("gpt-4o"));
    }

    #[test]
    fn test_agent_cost_summary() {
        let conn = setup_db();
        let mut input_log = make_log("opencode", "gpt-4.1", TokenType::Input, 100, "req-1");
        input_log.cost = Some(0.25);
        let output_log = make_log("opencode", "gpt-4.1", TokenType::Output, 50, "req-1");

        batch_insert_token_logs(&conn, &[input_log, output_log]).unwrap();

        let summary = get_token_summary(&conn, "today").unwrap();
        assert_eq!(summary.agent_cost, 0.25);
        assert_eq!(summary.by_agent.get("opencode").unwrap().agent_cost, 0.25);
        assert_eq!(summary.by_model.get("gpt-4.1").unwrap().agent_cost, 0.25);
        assert_eq!(get_agent_cost_summary(&conn, "today").unwrap(), 0.25);
    }

    #[test]
    fn test_settings_crud() {
        let conn = setup_db();

        // 初始为空
        assert!(get_setting(&conn, "keep_days").unwrap().is_none());

        // 写入
        set_setting(&conn, "keep_days", "30").unwrap();
        assert_eq!(get_setting(&conn, "keep_days").unwrap().unwrap(), "30");

        // 覆盖
        set_setting(&conn, "keep_days", "7").unwrap();
        assert_eq!(get_setting(&conn, "keep_days").unwrap().unwrap(), "7");

        // 获取全部
        set_setting(&conn, "language", "en").unwrap();
        let all = get_all_settings(&conn).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_offset_crud() {
        let conn = setup_db();

        assert!(get_offset(&conn, "/tmp/test.jsonl").unwrap().is_none());

        update_offset(&conn, "/tmp/test.jsonl", 1024).unwrap();
        assert_eq!(get_offset(&conn, "/tmp/test.jsonl").unwrap().unwrap(), 1024);

        update_offset(&conn, "/tmp/test.jsonl", 2048).unwrap();
        assert_eq!(get_offset(&conn, "/tmp/test.jsonl").unwrap().unwrap(), 2048);
    }

    #[test]
    fn test_clear_data() {
        let conn = setup_db();
        let logs = vec![make_log(
            "claude-code",
            "claude-3-7-sonnet",
            TokenType::Input,
            100,
            "req-1",
        )];
        batch_insert_token_logs(&conn, &logs).unwrap();
        update_offset(&conn, "/tmp/test.jsonl", 1024).unwrap();

        // 清空全部
        clear_data(&conn, None).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM token_logs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM file_offsets", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
