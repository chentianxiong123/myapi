//! 网关请求日志（observability，学 LiteLLM spend-logs）
//!
//! 每次转发后记录一行到 SQLite request_logs 表：时间/模型/供应商/通道/状态/耗时/TTFT/token。
//! 异步非阻塞：用 tokio::task::spawn_blocking 包 rusqlite（同步库），日志失败不影响转发。

use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::PathBuf;

/// 日志库路径（可 MYAPI_LOG_DB 覆盖，默认 <root>/config/gateway-logs.db）
pub fn db_path(root: &std::path::Path) -> PathBuf {
    std::env::var("MYAPI_LOG_DB").map(PathBuf::from).unwrap_or_else(|_| {
        root.join("config").join("gateway-logs.db")
    })
}

/// 建表（幂等，启动时调一次）
pub fn init_table(root: &std::path::Path) -> Result<(), String> {
    let conn = Connection::open(db_path(root)).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS request_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts TEXT NOT NULL,
            model TEXT NOT NULL,
            provider TEXT NOT NULL,
            channel TEXT NOT NULL,
            status INTEGER NOT NULL,
            latency_ms INTEGER NOT NULL,
            ttft_ms INTEGER NOT NULL,
            input_tokens INTEGER,
            output_tokens INTEGER,
            error TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_request_logs_ts ON request_logs(ts);
        CREATE INDEX IF NOT EXISTS idx_request_logs_model ON request_logs(model);",
    ).map_err(|e| e.to_string())?;
    Ok(())
}

/// 一条请求日志
#[derive(Debug, Clone, Default)]
pub struct LogEntry {
    pub model: String,
    pub provider: String,
    pub channel: String,
    pub status: u16,
    pub latency_ms: u64,
    pub ttft_ms: u64,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub error: Option<String>,
}

/// 记录一条（异步非阻塞，日志失败仅 eprintln 不影响转发）
pub async fn log_request(root: std::path::PathBuf, entry: LogEntry) {
    let res = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let conn = Connection::open(db_path(&root)).map_err(|e| e.to_string())?;
        let ts = chrono_now_secs_iso();
        conn.execute(
            "INSERT INTO request_logs (ts, model, provider, channel, status, latency_ms, ttft_ms, input_tokens, output_tokens, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![ts, entry.model, entry.provider, entry.channel, entry.status as i64,
                    entry.latency_ms as i64, entry.ttft_ms as i64,
                    entry.input_tokens, entry.output_tokens, entry.error],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }).await;
    match res {
        Ok(Ok(())) => {}
        Ok(Err(e)) => eprintln!("[gateway-logs] 写入失败: {}", e),
        Err(e) => eprintln!("[gateway-logs] task join 失败: {}", e),
    }
}

/// 查询最近 N 条（/v1/logs 用）
pub fn query_recent(root: &std::path::Path, limit: usize) -> Result<Value, String> {
    let conn = Connection::open(db_path(root)).map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT ts, model, provider, channel, status, latency_ms, ttft_ms, input_tokens, output_tokens, error
         FROM request_logs ORDER BY id DESC LIMIT ?1"
    ).map_err(|e| e.to_string())?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok(json!({
            "ts": row.get::<_, String>(0)?,
            "model": row.get::<_, String>(1)?,
            "provider": row.get::<_, String>(2)?,
            "channel": row.get::<_, String>(3)?,
            "status": row.get::<_, i64>(4)?,
            "latency_ms": row.get::<_, i64>(5)?,
            "ttft_ms": row.get::<_, i64>(6)?,
            "input_tokens": row.get::<_, Option<i64>>(7)?,
            "output_tokens": row.get::<_, Option<i64>>(8)?,
            "error": row.get::<_, Option<String>>(9)?,
        }))
    }).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(json!({ "object": "list", "data": out }))
}

/// 当前 UTC 时间 ISO8601（无 chrono 依赖，手拼）
fn chrono_now_secs_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 简单转 ISO（不做时区，UTC）
    let days = secs / 86400;
    let rem = secs % 86400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // 从 1970-01-01 起算日期（简化，够用）
    let (y, mo, d) = civil_from_days(days as i64);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, m, s)
}

/// 天数 → (年, 月, 日)（Howard Hinnant 算法）
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe/1460 + doe/36524 - doe/146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365*yoe + yoe/4 - yoe/100);
    let mp = (5*doy + 2)/153;
    let d = (doy - (153*mp+2)/5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 按通道聚合统计（/v1/stats 用，前端模型页/仪表盘真实数据源）
/// 返回每通道：请求数/错误数/错误率/平均 TTFT/平均耗时/token 合计
pub fn query_stats(root: &std::path::Path) -> Result<Value, String> {
    let conn = Connection::open(db_path(root)).map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT channel,
                COUNT(*) AS total,
                SUM(CASE WHEN status >= 400 THEN 1 ELSE 0 END) AS errors,
                AVG(ttft_ms) AS avg_ttft,
                AVG(latency_ms) AS avg_latency,
                SUM(COALESCE(input_tokens,0)) AS in_tok,
                SUM(COALESCE(output_tokens,0)) AS out_tok
         FROM request_logs
         GROUP BY channel
         ORDER BY total DESC"
    ).map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| {
        let total: i64 = row.get(1)?;
        let errors: i64 = row.get(2)?;
        let avg_ttft: f64 = row.get(3).unwrap_or(0.0);
        let avg_latency: f64 = row.get(4).unwrap_or(0.0);
        // 状态推导：错误率>10% → error；有熔断意味但这里简化按错误率
        let err_rate = if total > 0 { errors as f64 / total as f64 } else { 0.0 };
        let status = if err_rate > 0.1 { "error" } else { "healthy" };
        Ok(json!({
            "channel": row.get::<_, String>(0)?,
            "total_requests": total,
            "errors": errors,
            "error_rate": (err_rate * 10000.0).round() / 100.0, // 百分比两位小数
            "avg_ttft_ms": (avg_ttft * 10.0).round() / 10.0,
            "avg_latency_ms": (avg_latency * 10.0).round() / 10.0,
            "status": status,
            "total_input_tokens": row.get::<_, i64>(5)?,
            "total_output_tokens": row.get::<_, i64>(6)?,
        }))
    }).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(json!({ "object": "list", "data": out }))
}
