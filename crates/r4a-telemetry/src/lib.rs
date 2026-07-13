pub mod collector;
pub mod store;

use serde::{Deserialize, Serialize};

/// Одна строка лога контейнера.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub node: String,
    pub container: String,
    /// Время получения строки агентом, unix millis.
    pub ts_ms: u64,
    /// "stdout" | "stderr"
    pub stream: String,
    pub line: String,
}

/// Батч, который агент отправляет на мастер.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogBatch {
    pub entries: Vec<LogEntry>,
}
