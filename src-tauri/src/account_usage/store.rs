use rusqlite::{params, Connection, OptionalExtension};

use crate::account_usage::{
    enum_from_db, enum_to_db, redact_secret_text, redact_snapshot_error, AccountUsageConfidence,
    AccountUsageError, AccountUsageMetric, AccountUsageMetricScope, AccountUsageProviderState,
    AccountUsageSnapshot, AccountUsageSource, AccountUsageStatus,
};

const ACCOUNT_USAGE_SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS account_usage_snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id TEXT NOT NULL,
    account_key TEXT NOT NULL,
    account_label TEXT,
    plan TEXT,
    status TEXT NOT NULL,
    source_type TEXT NOT NULL,
    confidence TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    period_start TEXT,
    period_end TEXT,
    reset_at TEXT,
    stale INTEGER NOT NULL DEFAULT 0,
    error_code TEXT,
    error_message TEXT,
    retry_after_secs INTEGER,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(provider_id, account_key)
);

CREATE INDEX IF NOT EXISTS idx_account_usage_snapshots_provider
ON account_usage_snapshots(provider_id, updated_at);

CREATE TABLE IF NOT EXISTS account_usage_metrics (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    snapshot_id INTEGER NOT NULL,
    metric_key TEXT NOT NULL,
    label TEXT NOT NULL,
    unit TEXT NOT NULL,
    scope TEXT NOT NULL,
    used REAL,
    limit_value REAL,
    remaining REAL,
    percentage REAL,
    reset_at TEXT,
    FOREIGN KEY(snapshot_id) REFERENCES account_usage_snapshots(id) ON DELETE CASCADE,
    UNIQUE(snapshot_id, metric_key, scope)
);

CREATE TABLE IF NOT EXISTS account_usage_provider_states (
    provider_id TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL DEFAULT 0,
    refresh_interval_secs INTEGER NOT NULL,
    last_refresh_at TEXT,
    retry_after_until TEXT,
    credential_ref TEXT,
    credential_label TEXT,
    auto_discovery_enabled INTEGER NOT NULL DEFAULT 0,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
";

pub fn init_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(ACCOUNT_USAGE_SCHEMA_SQL)
}

pub fn ensure_provider_state(
    conn: &Connection,
    provider_id: &str,
    refresh_interval_secs: u64,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT OR IGNORE INTO account_usage_provider_states
         (provider_id, enabled, refresh_interval_secs, auto_discovery_enabled, updated_at)
         VALUES (?1, 0, ?2, 0, CURRENT_TIMESTAMP)",
        params![provider_id, refresh_interval_secs as i64],
    )?;
    Ok(())
}

pub fn upsert_snapshot(
    conn: &Connection,
    snapshot: &AccountUsageSnapshot,
) -> Result<(), rusqlite::Error> {
    let mut snapshot = snapshot.clone();
    redact_snapshot_error(&mut snapshot);
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO account_usage_snapshots
         (provider_id, account_key, account_label, plan, status, source_type, confidence,
          observed_at, period_start, period_end, reset_at, stale, error_code, error_message,
          retry_after_secs, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, CURRENT_TIMESTAMP)
         ON CONFLICT(provider_id, account_key) DO UPDATE SET
            account_label = excluded.account_label,
            plan = excluded.plan,
            status = excluded.status,
            source_type = excluded.source_type,
            confidence = excluded.confidence,
            observed_at = excluded.observed_at,
            period_start = excluded.period_start,
            period_end = excluded.period_end,
            reset_at = excluded.reset_at,
            stale = excluded.stale,
            error_code = excluded.error_code,
            error_message = excluded.error_message,
            retry_after_secs = excluded.retry_after_secs,
            updated_at = CURRENT_TIMESTAMP",
        params![
            snapshot.provider_id,
            snapshot.account_key,
            snapshot.account_label,
            snapshot.plan,
            enum_to_db(&snapshot.status),
            enum_to_db(&snapshot.source),
            enum_to_db(&snapshot.confidence),
            snapshot.observed_at,
            snapshot.period_start,
            snapshot.period_end,
            snapshot.reset_at,
            snapshot.stale as i32,
            snapshot.error.as_ref().map(|error| enum_to_db(&error.code)),
            snapshot.error.as_ref().map(|error| error.message.clone()),
            snapshot.error.as_ref().and_then(|error| error.retry_after_secs).map(|v| v as i64),
        ],
    )?;

    let snapshot_id: i64 = tx.query_row(
        "SELECT id FROM account_usage_snapshots WHERE provider_id = ?1 AND account_key = ?2",
        params![snapshot.provider_id, snapshot.account_key],
        |row| row.get(0),
    )?;
    tx.execute(
        "DELETE FROM account_usage_metrics WHERE snapshot_id = ?1",
        params![snapshot_id],
    )?;
    for metric in &snapshot.metrics {
        insert_metric(&tx, snapshot_id, metric)?;
    }
    tx.commit()
}

fn insert_metric(
    conn: &Connection,
    snapshot_id: i64,
    metric: &AccountUsageMetric,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO account_usage_metrics
         (snapshot_id, metric_key, label, unit, scope, used, limit_value, remaining, percentage, reset_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            snapshot_id,
            metric.metric_key,
            metric.label,
            metric.unit,
            enum_to_db(&metric.scope),
            metric.used,
            metric.limit,
            metric.remaining,
            metric.percentage,
            metric.reset_at,
        ],
    )?;
    Ok(())
}

pub fn latest_snapshots(conn: &Connection) -> Result<Vec<AccountUsageSnapshot>, rusqlite::Error> {
    latest_snapshots_for_provider(conn, None)
}

pub fn latest_snapshots_by_provider(
    conn: &Connection,
    provider_id: &str,
) -> Result<Vec<AccountUsageSnapshot>, rusqlite::Error> {
    latest_snapshots_for_provider(conn, Some(provider_id))
}

fn latest_snapshots_for_provider(
    conn: &Connection,
    provider_id: Option<&str>,
) -> Result<Vec<AccountUsageSnapshot>, rusqlite::Error> {
    let sql = match provider_id {
        Some(_) => {
            "SELECT id, provider_id, account_key, account_label, plan, status, source_type,
                    confidence, observed_at, period_start, period_end, reset_at, stale,
                    error_code, error_message, retry_after_secs
             FROM account_usage_snapshots WHERE provider_id = ?1 ORDER BY updated_at DESC"
        }
        None => {
            "SELECT id, provider_id, account_key, account_label, plan, status, source_type,
                    confidence, observed_at, period_start, period_end, reset_at, stale,
                    error_code, error_message, retry_after_secs
             FROM account_usage_snapshots ORDER BY provider_id, updated_at DESC"
        }
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = match provider_id {
        Some(id) => stmt.query_map(params![id], snapshot_from_row)?,
        None => stmt.query_map([], snapshot_from_row)?,
    };

    let mut snapshots = Vec::new();
    for row in rows {
        let (snapshot_id, mut snapshot) = row?;
        snapshot.metrics = metrics_for_snapshot(conn, snapshot_id)?;
        snapshots.push(snapshot);
    }
    Ok(snapshots)
}

fn snapshot_from_row(
    row: &rusqlite::Row<'_>,
) -> Result<(i64, AccountUsageSnapshot), rusqlite::Error> {
    let snapshot_id: i64 = row.get(0)?;
    let status_str: String = row.get(5)?;
    let source_str: String = row.get(6)?;
    let confidence_str: String = row.get(7)?;
    let error_code: Option<String> = row.get(13)?;
    let error_message: Option<String> = row.get(14)?;
    let retry_after_secs: Option<i64> = row.get(15)?;
    let error = error_code
        .or(error_message.clone())
        .map(|code_or_message| AccountUsageError {
            code: enum_from_db(&code_or_message, AccountUsageStatus::Error),
            message: redact_secret_text(&error_message.unwrap_or_else(|| code_or_message.clone())),
            retry_after_secs: retry_after_secs.map(|value| value as u64),
        });

    Ok((
        snapshot_id,
        AccountUsageSnapshot {
            provider_id: row.get(1)?,
            account_key: row.get(2)?,
            account_label: row.get(3)?,
            plan: row.get(4)?,
            status: enum_from_db(&status_str, AccountUsageStatus::Error),
            source: enum_from_db(&source_str, AccountUsageSource::Unsupported),
            confidence: enum_from_db(&confidence_str, AccountUsageConfidence::Low),
            observed_at: row.get(8)?,
            period_start: row.get(9)?,
            period_end: row.get(10)?,
            reset_at: row.get(11)?,
            stale: row.get::<_, i64>(12)? != 0,
            error,
            metrics: Vec::new(),
        },
    ))
}

fn metrics_for_snapshot(
    conn: &Connection,
    snapshot_id: i64,
) -> Result<Vec<AccountUsageMetric>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT metric_key, label, unit, scope, used, limit_value, remaining, percentage, reset_at
         FROM account_usage_metrics WHERE snapshot_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map(params![snapshot_id], |row| {
        let scope_str: String = row.get(3)?;
        Ok(AccountUsageMetric {
            metric_key: row.get(0)?,
            label: row.get(1)?,
            unit: row.get(2)?,
            scope: enum_from_db(&scope_str, AccountUsageMetricScope::Account),
            used: row.get(4)?,
            limit: row.get(5)?,
            remaining: row.get(6)?,
            percentage: row.get(7)?,
            reset_at: row.get(8)?,
        })
    })?;

    let mut metrics = Vec::new();
    for row in rows {
        metrics.push(row?);
    }
    Ok(metrics)
}

pub fn get_provider_state(
    conn: &Connection,
    provider_id: &str,
) -> Result<Option<AccountUsageProviderState>, rusqlite::Error> {
    conn.query_row(
        "SELECT provider_id, enabled, refresh_interval_secs, last_refresh_at, retry_after_until,
                credential_ref, credential_label, auto_discovery_enabled
         FROM account_usage_provider_states WHERE provider_id = ?1",
        params![provider_id],
        provider_state_from_row,
    )
    .optional()
}

pub fn list_provider_states(
    conn: &Connection,
) -> Result<Vec<AccountUsageProviderState>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT provider_id, enabled, refresh_interval_secs, last_refresh_at, retry_after_until,
                credential_ref, credential_label, auto_discovery_enabled
         FROM account_usage_provider_states ORDER BY provider_id",
    )?;
    let rows = stmt.query_map([], provider_state_from_row)?;
    let mut states = Vec::new();
    for row in rows {
        states.push(row?);
    }
    Ok(states)
}

fn provider_state_from_row(
    row: &rusqlite::Row<'_>,
) -> Result<AccountUsageProviderState, rusqlite::Error> {
    Ok(AccountUsageProviderState {
        provider_id: row.get(0)?,
        enabled: row.get::<_, i64>(1)? != 0,
        refresh_interval_secs: row.get::<_, i64>(2)? as u64,
        last_refresh_at: row.get(3)?,
        retry_after_until: row.get(4)?,
        credential_ref: row.get(5)?,
        credential_label: row.get(6)?,
        auto_discovery_enabled: row.get::<_, i64>(7)? != 0,
    })
}

pub fn upsert_provider_state(
    conn: &Connection,
    state: &AccountUsageProviderState,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO account_usage_provider_states
         (provider_id, enabled, refresh_interval_secs, last_refresh_at, retry_after_until,
          credential_ref, credential_label, auto_discovery_enabled, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, CURRENT_TIMESTAMP)
         ON CONFLICT(provider_id) DO UPDATE SET
            enabled = excluded.enabled,
            refresh_interval_secs = excluded.refresh_interval_secs,
            last_refresh_at = excluded.last_refresh_at,
            retry_after_until = excluded.retry_after_until,
            credential_ref = excluded.credential_ref,
            credential_label = excluded.credential_label,
            auto_discovery_enabled = excluded.auto_discovery_enabled,
            updated_at = CURRENT_TIMESTAMP",
        params![
            state.provider_id,
            state.enabled as i32,
            state.refresh_interval_secs as i64,
            state.last_refresh_at,
            state.retry_after_until,
            state.credential_ref,
            state.credential_label,
            state.auto_discovery_enabled as i32,
        ],
    )?;
    Ok(())
}

pub fn mark_provider_snapshots_stale(
    conn: &Connection,
    provider_id: &str,
    error: &AccountUsageError,
) -> Result<(), rusqlite::Error> {
    let redacted_message = redact_secret_text(&error.message);
    conn.execute(
        "UPDATE account_usage_snapshots
         SET status = ?1, stale = 1, error_code = ?2, error_message = ?3,
             retry_after_secs = ?4, updated_at = CURRENT_TIMESTAMP
         WHERE provider_id = ?5",
        params![
            enum_to_db(&AccountUsageStatus::Stale),
            enum_to_db(&error.code),
            redacted_message,
            error.retry_after_secs.map(|value| value as i64),
            provider_id,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::SCHEMA_SQL).unwrap();
        init_schema(&conn).unwrap();
        conn
    }

    fn sample_snapshot() -> AccountUsageSnapshot {
        AccountUsageSnapshot {
            provider_id: "test".to_string(),
            account_key: "account-1".to_string(),
            account_label: Some("user@example.com".to_string()),
            plan: Some("Pro".to_string()),
            status: AccountUsageStatus::Ok,
            source: AccountUsageSource::OfficialApi,
            confidence: AccountUsageConfidence::High,
            observed_at: crate::account_usage::now_rfc3339(),
            period_start: None,
            period_end: None,
            reset_at: None,
            stale: false,
            error: None,
            metrics: vec![AccountUsageMetric {
                metric_key: "quota.primary".to_string(),
                label: "Primary quota".to_string(),
                unit: "request".to_string(),
                scope: AccountUsageMetricScope::Personal,
                used: Some(10.0),
                limit: Some(100.0),
                remaining: Some(90.0),
                percentage: Some(10.0),
                reset_at: None,
            }],
        }
    }

    #[test]
    fn test_account_usage_schema_init() {
        let conn = setup_db();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM account_usage_snapshots", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        let token_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM token_logs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(token_count, 0);
    }

    #[test]
    fn test_upsert_snapshot_replaces_metrics() {
        let conn = setup_db();
        let mut snapshot = sample_snapshot();
        upsert_snapshot(&conn, &snapshot).unwrap();
        snapshot.metrics[0].used = Some(25.0);
        snapshot.metrics[0].remaining = Some(75.0);
        upsert_snapshot(&conn, &snapshot).unwrap();

        let snapshots = latest_snapshots(&conn).unwrap();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].metrics.len(), 1);
        assert_eq!(snapshots[0].metrics[0].used, Some(25.0));
    }

    #[test]
    fn test_mark_provider_snapshots_stale() {
        let conn = setup_db();
        upsert_snapshot(&conn, &sample_snapshot()).unwrap();
        let error = AccountUsageError::new(AccountUsageStatus::Network, "网络失败");
        mark_provider_snapshots_stale(&conn, "test", &error).unwrap();

        let snapshots = latest_snapshots(&conn).unwrap();
        assert!(snapshots[0].stale);
        assert_eq!(snapshots[0].status, AccountUsageStatus::Stale);
    }

    #[test]
    fn test_provider_state_persists_reference_not_secret() {
        let conn = setup_db();
        let state = AccountUsageProviderState {
            provider_id: "cursor".to_string(),
            enabled: true,
            refresh_interval_secs: 600,
            last_refresh_at: None,
            retry_after_until: None,
            credential_ref: Some("cursor:default:session".to_string()),
            credential_label: Some("user@example.com".to_string()),
            auto_discovery_enabled: false,
        };
        upsert_provider_state(&conn, &state).unwrap();

        let raw_secret = "secret-cookie-value";
        let rows: String = conn
            .query_row(
                "SELECT provider_id || ':' || COALESCE(credential_ref, '') || ':' || COALESCE(credential_label, '')
                 FROM account_usage_provider_states WHERE provider_id = 'cursor'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!rows.contains(raw_secret));
    }

    #[test]
    fn test_redacts_secret_like_text() {
        let redacted = crate::account_usage::redact_secret_text(
            "authorization: Bearer abc123\ncookie=session=secret; path=/\nraw ghp_123456789secret and eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signature12345678",
        );
        assert!(!redacted.contains("abc123"));
        assert!(!redacted.contains("secret"));
        assert!(!redacted.contains("ghp_123456789secret"));
        assert!(!redacted.contains("eyJhbGciOiJIUzI1NiJ9"));
        assert!(redacted.contains("<redacted>"));
    }

    #[test]
    fn test_snapshot_error_is_redacted_before_persist_and_read() {
        let conn = setup_db();
        let mut snapshot = sample_snapshot();
        snapshot.error = Some(AccountUsageError::new(
            AccountUsageStatus::AuthRequired,
            "upstream returned ghp_123456789secret",
        ));
        upsert_snapshot(&conn, &snapshot).unwrap();

        let raw_message: String = conn
            .query_row(
                "SELECT error_message FROM account_usage_snapshots WHERE provider_id = 'test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!raw_message.contains("ghp_123456789secret"));

        let snapshots = latest_snapshots(&conn).unwrap();
        assert!(!snapshots[0]
            .error
            .as_ref()
            .unwrap()
            .message
            .contains("ghp_123456789secret"));
    }
}
