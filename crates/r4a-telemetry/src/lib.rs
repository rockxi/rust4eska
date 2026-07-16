pub mod collector;
pub mod store;

use serde::{Deserialize, Serialize};

/// Одна строка лога контейнера. Поля 1:1 совпадают с колонками таблицы
/// `r4a.logs` в ClickHouse — INSERT/SELECT идут в формате JSONEachRow без маппинга.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub node: String,
    pub container: String,
    /// Docker-timestamp строки, unix millis.
    pub ts_ms: u64,
    /// "stdout" | "stderr"
    pub stream: String,
    pub line: String,
}

/// Куда агенту слать логи. Мастер отдаёт это по GET /api/logs/agent-config
/// (защищено cluster secret), пока логи не настроены — 404.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogsChTarget {
    /// База ClickHouse HTTP, например "http://10.42.0.2:8123"
    pub endpoint: String,
    /// Пароль пользователя default
    pub password: String,
}

/// SQL схемы ClickHouse (создаёт мастер после старта контейнера).
pub const CH_CREATE_DATABASE: &str = "CREATE DATABASE IF NOT EXISTS r4a";
pub const CH_CREATE_LOGS_TABLE: &str = "\
CREATE TABLE IF NOT EXISTS r4a.logs (
    node LowCardinality(String),
    container LowCardinality(String),
    ts_ms UInt64,
    stream LowCardinality(String),
    line String,
    INDEX line_ngram line TYPE ngrambf_v1(3, 4096, 2, 0) GRANULARITY 4
) ENGINE = MergeTree
PARTITION BY toDate(toDateTime(intDiv(ts_ms, 1000)))
ORDER BY (node, container, ts_ms)
TTL toDateTime(intDiv(ts_ms, 1000)) + INTERVAL 14 DAY";

/// Догоняет индекс на line для таблиц, созданных до появления поиска
/// (CREATE TABLE IF NOT EXISTS не добавляет колонки/индексы к уже существующей таблице).
pub const CH_ADD_LOGS_LINE_INDEX: &str =
    "ALTER TABLE r4a.logs ADD INDEX IF NOT EXISTS line_ngram line TYPE ngrambf_v1(3, 4096, 2, 0) GRANULARITY 4";
pub const CH_MATERIALIZE_LOGS_LINE_INDEX: &str =
    "ALTER TABLE r4a.logs MATERIALIZE INDEX line_ngram";

/// Точка истории метрик ноды (CPU/RAM/VRAM на момент времени).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricPoint {
    pub node: String,
    pub ts_ms: u64,
    pub cpu_percent: f32,
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
    pub vram_used_mb: Option<u64>,
    pub vram_total_mb: Option<u64>,
}
