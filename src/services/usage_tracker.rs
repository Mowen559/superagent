use crate::config::app_paths::AppPaths;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Once;

static INIT: Once = Once::new();

pub fn init_db() {
    INIT.call_once(|| {
        let db_path = get_db_path();

        let paths = AppPaths::new();
        paths.ensure_data_dir(&["data"]);

        if let Ok(conn) = Connection::open(&db_path) {
            let _ = conn.execute(
                "CREATE TABLE IF NOT EXISTS api_usage_logs (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                    account_id TEXT NOT NULL,
                    model TEXT NOT NULL,
                    prompt_tokens INTEGER NOT NULL,
                    completion_tokens INTEGER NOT NULL,
                    total_cost REAL NOT NULL,
                    method TEXT NOT NULL DEFAULT 'POST',
                    path TEXT NOT NULL DEFAULT '',
                    protocol TEXT NOT NULL DEFAULT '',
                    status INTEGER NOT NULL DEFAULT 200,
                    duration_ms INTEGER NOT NULL DEFAULT 0,
                    cached_tokens INTEGER NOT NULL DEFAULT 0,
                    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
                    request_body TEXT,
                    response_body TEXT
                )",
                [],
            );
            for migration in [
                "ALTER TABLE api_usage_logs ADD COLUMN method TEXT NOT NULL DEFAULT 'POST'",
                "ALTER TABLE api_usage_logs ADD COLUMN path TEXT NOT NULL DEFAULT ''",
                "ALTER TABLE api_usage_logs ADD COLUMN protocol TEXT NOT NULL DEFAULT ''",
                "ALTER TABLE api_usage_logs ADD COLUMN status INTEGER NOT NULL DEFAULT 200",
                "ALTER TABLE api_usage_logs ADD COLUMN duration_ms INTEGER NOT NULL DEFAULT 0",
                "ALTER TABLE api_usage_logs ADD COLUMN cached_tokens INTEGER NOT NULL DEFAULT 0",
                "ALTER TABLE api_usage_logs ADD COLUMN cache_write_tokens INTEGER NOT NULL DEFAULT 0",
                "ALTER TABLE api_usage_logs ADD COLUMN request_body TEXT",
                "ALTER TABLE api_usage_logs ADD COLUMN response_body TEXT",
                "ALTER TABLE api_usage_logs ADD COLUMN attempt_receipts TEXT",
                "ALTER TABLE api_usage_logs ADD COLUMN ttft_ms INTEGER",
                "ALTER TABLE api_usage_logs ADD COLUMN reasoning_tokens INTEGER NOT NULL DEFAULT 0",
            ] {
                let _ = conn.execute(migration, []);
            }
        } else {
            tracing::error!("Failed to create usage.db at {:?}", db_path);
        }
    });
}

fn get_db_path() -> PathBuf {
    AppPaths::new().data_path(&["data", "usage.db"])
}

pub fn record_usage(
    account_id: String,
    model: String,
    prompt_tokens: u32,
    completion_tokens: u32,
    total_cost: f64,
) {
    if prompt_tokens == 0 && completion_tokens == 0 {
        return;
    }

    tokio::task::spawn_blocking(move || {
        let db_path = get_db_path();
        match Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_WRITE) {
            Ok(conn) => {
                let res = conn.execute(
                    "INSERT INTO api_usage_logs (account_id, model, prompt_tokens, completion_tokens, total_cost) VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![account_id, model, prompt_tokens, completion_tokens, total_cost],
                );
                if let Err(e) = res {
                    tracing::error!("Failed to insert usage log: {}", e);
                }
            }
            Err(e) => tracing::error!("Failed to open usage.db for recording: {}", e),
        }
    });
}

#[derive(Debug, Clone)]
pub struct ApiUsageRecord {
    pub account_id: String,
    pub model: String,
    pub protocol: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub duration_ms: u64,
    pub ttft_ms: Option<u64>,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub reasoning_tokens: u32,
    pub cached_tokens: u32,
    pub cache_write_tokens: u32,
    pub total_cost: f64,
    pub request_body: Option<String>,
    pub response_body: Option<String>,
    pub attempt_receipts: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct ApiUsageLog {
    pub id: i64,
    pub timestamp: String,
    pub account_id: String,
    pub model: String,
    pub protocol: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub duration_ms: u64,
    pub ttft_ms: Option<u64>,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub reasoning_tokens: u32,
    pub cached_tokens: u32,
    pub cache_write_tokens: u32,
    pub total_cost: f64,
    pub tps: Option<f64>,
    pub request_body: Option<Value>,
    pub response_body: Option<Value>,
    pub attempt_receipts: Option<Value>,
}

#[derive(Serialize)]
pub struct DailyUsage {
    pub date: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_cost: f64,
    pub cached_tokens: u32,
    pub cache_write_tokens: u32,
}

pub type UsageLog = ApiUsageLog;

pub fn record_api_call(record: ApiUsageRecord) {
    let worker = move || {
        let db_path = get_db_path();
        match Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_WRITE) {
            Ok(conn) => {
                let res = conn.execute(
                    "INSERT INTO api_usage_logs (
                        account_id, model, protocol, method, path, status, duration_ms, ttft_ms,
                        prompt_tokens, completion_tokens, reasoning_tokens, cached_tokens, cache_write_tokens,
                        total_cost, request_body, response_body, attempt_receipts
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                    rusqlite::params![
                        record.account_id,
                        record.model,
                        record.protocol,
                        record.method,
                        record.path,
                        record.status,
                        record.duration_ms.min(i64::MAX as u64) as i64,
                        record.ttft_ms.map(|v| v.min(i64::MAX as u64) as i64),
                        record.prompt_tokens,
                        record.completion_tokens,
                        record.reasoning_tokens,
                        record.cached_tokens,
                        record.cache_write_tokens,
                        record.total_cost,
                        record.request_body.map(|body| truncate_for_storage(&body)),
                        record.response_body.map(|body| truncate_for_storage(&body)),
                        record
                            .attempt_receipts
                            .and_then(|val| serde_json::to_string(&val).ok()),
                    ],
                );
                if let Err(e) = res {
                    tracing::error!("Failed to insert API usage log: {}", e);
                }
            }
            Err(e) => tracing::error!("Failed to open usage.db for API usage recording: {}", e),
        }
    };

    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn_blocking(worker);
    } else {
        std::thread::spawn(worker);
    }
}

pub fn record_request(
    account_id: String,
    model: String,
    prompt_tokens: u32,
    completion_tokens: u32,
    total_cost: f64,
    status: u16,
    method: String,
    protocol: String,
    path: String,
    duration_ms: u64,
) {
    record_api_call(ApiUsageRecord {
        account_id,
        model,
        protocol,
        method,
        path,
        status,
        duration_ms,
        ttft_ms: None,
        prompt_tokens,
        completion_tokens,
        reasoning_tokens: 0,
        cached_tokens: 0,
        cache_write_tokens: 0,
        total_cost,
        request_body: None,
        response_body: None,
        attempt_receipts: None,
    });
}

pub fn get_daily_usage(days: u32) -> Result<Vec<DailyUsage>, rusqlite::Error> {
    let conn = Connection::open_with_flags(get_db_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare(
        "SELECT DATE(timestamp) as date,
                COALESCE(SUM(prompt_tokens), 0),
                COALESCE(SUM(completion_tokens), 0),
                COALESCE(SUM(total_cost), 0),
                COALESCE(SUM(cached_tokens), 0),
                COALESCE(SUM(cache_write_tokens), 0)
         FROM api_usage_logs
         WHERE timestamp >= date('now', ?)
         GROUP BY DATE(timestamp)
         ORDER BY date ASC",
    )?;
    let time_modifier = format!("-{} days", days);
    let iter = stmt.query_map(rusqlite::params![time_modifier], |row| {
        Ok(DailyUsage {
            date: row.get(0)?,
            prompt_tokens: row.get::<_, i64>(1)?.max(0) as u32,
            completion_tokens: row.get::<_, i64>(2)?.max(0) as u32,
            total_cost: row.get::<_, f64>(3).unwrap_or(0.0),
            cached_tokens: row.get::<_, i64>(4)?.max(0) as u32,
            cache_write_tokens: row.get::<_, i64>(5)?.max(0) as u32,
        })
    })?;
    iter.collect()
}

pub fn get_usage_logs(limit: u32) -> Result<Vec<UsageLog>, rusqlite::Error> {
    list_usage_logs(limit as usize).map_err(rusqlite::Error::InvalidParameterName)
}

pub fn list_usage_logs(limit: usize) -> Result<Vec<ApiUsageLog>, String> {
    let conn = Connection::open(get_db_path()).map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, timestamp, account_id, model, protocol, method, path, status, duration_ms, ttft_ms,
                    prompt_tokens, completion_tokens, reasoning_tokens, cached_tokens, cache_write_tokens, total_cost,
                    request_body, response_body, attempt_receipts
             FROM api_usage_logs
             ORDER BY id DESC
             LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([limit.min(1000) as i64], |row| {
            let duration_ms = row.get::<_, i64>(8)?.max(0) as u64;
            let ttft_ms = row.get::<_, Option<i64>>(9)?.map(|v| v.max(0) as u64);
            let prompt_tokens = row.get::<_, i64>(10)?.max(0) as u32;
            let completion_tokens = row.get::<_, i64>(11)?.max(0) as u32;
            let reasoning_tokens = row.get::<_, Option<i64>>(12)?.unwrap_or(0).max(0) as u32;
            let cached_tokens = row.get::<_, i64>(13)?.max(0) as u32;
            let cache_write_tokens = row.get::<_, i64>(14)?.max(0) as u32;
            let total_cost = row.get(15)?;
            let request_text: Option<String> = row.get(16)?;
            let response_text: Option<String> = row.get(17)?;
            let attempt_receipts_text: Option<String> = row.get(18)?;

            let tps = if completion_tokens > 0 {
                let generating_time_ms = if let Some(ttft) = ttft_ms {
                    duration_ms.saturating_sub(ttft)
                } else {
                    duration_ms
                };
                if generating_time_ms > 0 {
                    Some((completion_tokens as f64) / (generating_time_ms as f64 / 1000.0))
                } else {
                    None
                }
            } else {
                None
            };

            Ok(ApiUsageLog {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                account_id: row.get(2)?,
                model: row.get(3)?,
                protocol: row.get(4)?,
                method: row.get(5)?,
                path: row.get(6)?,
                status: row.get::<_, i64>(7)?.clamp(0, u16::MAX as i64) as u16,
                duration_ms,
                ttft_ms,
                prompt_tokens,
                completion_tokens,
                reasoning_tokens,
                cached_tokens,
                cache_write_tokens,
                total_cost,
                tps,
                request_body: request_text.as_deref().map(parse_json_or_text),
                response_body: response_text.as_deref().map(parse_json_or_text),
                attempt_receipts: attempt_receipts_text.and_then(|t| serde_json::from_str(&t).ok()),
            })
        })
        .map_err(|e| e.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn daily_usage_stats() -> Result<Vec<Value>, String> {
    let conn = Connection::open(get_db_path()).map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT date(timestamp) AS usage_date,
                    COALESCE(SUM(prompt_tokens), 0),
                    COALESCE(SUM(completion_tokens), 0),
                    COALESCE(SUM(cached_tokens), 0),
                    COALESCE(SUM(cache_write_tokens), 0)
             FROM api_usage_logs
             WHERE timestamp >= datetime('now', '-30 days')
             GROUP BY usage_date
             ORDER BY usage_date ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(json!({
                "date": row.get::<_, String>(0)?,
                "prompt_tokens": row.get::<_, i64>(1)?,
                "completion_tokens": row.get::<_, i64>(2)?,
                "cached_tokens": row.get::<_, i64>(3)?,
                "cache_write_tokens": row.get::<_, i64>(4)?,
            }))
        })
        .map_err(|e| e.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn parse_json_or_text(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| json!({ "text": text }))
}

fn truncate_for_storage(value: &str) -> String {
    crate::shared_kernel::text::truncate_with_suffix(value, 200_000, "\n... truncated ...")
}

pub fn clear_usage_logs() -> rusqlite::Result<()> {
    let db_path = get_db_path();
    let conn = Connection::open(&db_path)?;
    conn.execute("DELETE FROM api_usage_logs", [])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tps_calculation_and_record_mapping() {
        init_db();
        let record = ApiUsageRecord {
            account_id: "test-acc-1".to_string(),
            model: "deepseek-r1".to_string(),
            protocol: "OpenAI".to_string(),
            method: "POST".to_string(),
            path: "/v1/chat/completions".to_string(),
            status: 200,
            duration_ms: 3000,
            ttft_ms: Some(1000),
            prompt_tokens: 100,
            completion_tokens: 80,
            reasoning_tokens: 40,
            cached_tokens: 20,
            cache_write_tokens: 0,
            total_cost: 0.05,
            request_body: None,
            response_body: None,
            attempt_receipts: None,
        };

        // Generating time is duration_ms (3000) - ttft_ms (1000) = 2000ms = 2.0s
        // TPS = 80 completion tokens / 2.0s = 40.0 tokens/s
        let generating_time_ms = record.duration_ms.saturating_sub(record.ttft_ms.unwrap());
        let tps = (record.completion_tokens as f64) / (generating_time_ms as f64 / 1000.0);
        assert!((tps - 40.0).abs() < 1e-6);
    }
}
