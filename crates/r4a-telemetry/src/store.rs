use crate::LogEntry;
use anyhow::Result;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

const TREE: &str = "logs";

/// Отдельный sled-инстанс для логов (append-only, не смешивается с основной БД).
/// Ключ: `{node}\0{container}\0{ts_ms:016x}{seq:08x}` — префиксный скан даёт
/// все строки контейнера в хронологическом порядке.
#[derive(Clone)]
pub struct LogStore {
    db: sled::Db,
    seq: Arc<AtomicU32>,
}

fn key_prefix(node: &str, container: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(node.len() + container.len() + 2);
    k.extend_from_slice(node.as_bytes());
    k.push(0);
    k.extend_from_slice(container.as_bytes());
    k.push(0);
    k
}

impl LogStore {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let db = sled::open(path)?;
        Ok(Self { db, seq: Arc::new(AtomicU32::new(0)) })
    }

    pub fn append(&self, entries: &[LogEntry]) -> Result<()> {
        let tree = self.db.open_tree(TREE)?;
        for e in entries {
            let mut key = key_prefix(&e.node, &e.container);
            key.extend_from_slice(format!("{:016x}", e.ts_ms).as_bytes());
            let seq = self.seq.fetch_add(1, Ordering::Relaxed);
            key.extend_from_slice(format!("{:08x}", seq).as_bytes());
            tree.insert(key, serde_json::to_vec(e)?)?;
        }
        Ok(())
    }

    /// Последние `tail` строк контейнера в хронологическом порядке.
    pub fn query(&self, node: &str, container: &str, tail: usize) -> Result<Vec<LogEntry>> {
        let tree = self.db.open_tree(TREE)?;
        let prefix = key_prefix(node, container);
        let mut out: Vec<LogEntry> = Vec::with_capacity(tail);
        for item in tree.scan_prefix(&prefix).rev().take(tail) {
            let (_, v) = item?;
            if let Ok(e) = serde_json::from_slice::<LogEntry>(&v) {
                out.push(e);
            }
        }
        out.reverse();
        Ok(out)
    }

    /// Список пар (node, container), по которым есть логи.
    pub fn list_containers(&self) -> Result<Vec<(String, String)>> {
        let tree = self.db.open_tree(TREE)?;
        let mut out: Vec<(String, String)> = Vec::new();
        for item in tree.iter() {
            let (k, _) = item?;
            let mut parts = k.splitn(3, |b| *b == 0);
            let node = String::from_utf8_lossy(parts.next().unwrap_or_default()).to_string();
            let container = String::from_utf8_lossy(parts.next().unwrap_or_default()).to_string();
            if out.last() != Some(&(node.clone(), container.clone())) {
                out.push((node, container));
            }
        }
        out.dedup();
        Ok(out)
    }

    /// Удаляет записи старше `max_age_secs`. Возвращает число удалённых.
    pub fn prune(&self, max_age_secs: u64) -> Result<usize> {
        let cutoff_ms = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_sub(max_age_secs)) * 1000;

        let tree = self.db.open_tree(TREE)?;
        let mut removed = 0;
        for item in tree.iter() {
            let (k, v) = item?;
            let old = serde_json::from_slice::<LogEntry>(&v)
                .map(|e| e.ts_ms < cutoff_ms)
                .unwrap_or(true); // нечитаемые записи тоже чистим
            if old {
                tree.remove(k)?;
                removed += 1;
            }
        }
        if removed > 0 {
            self.db.flush()?;
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(node: &str, container: &str, ts_ms: u64, line: &str) -> LogEntry {
        LogEntry {
            node: node.to_string(),
            container: container.to_string(),
            ts_ms,
            stream: "stdout".to_string(),
            line: line.to_string(),
        }
    }

    #[test]
    fn append_query_prune() {
        let dir = tempfile::tempdir().unwrap();
        let store = LogStore::open(dir.path()).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;

        store.append(&[
            entry("n1", "c1", now - 1000, "first"),
            entry("n1", "c1", now, "second"),
            entry("n1", "c2", now, "other-container"),
            entry("n2", "c1", now, "other-node"),
        ]).unwrap();

        // tail с хронологическим порядком, без чужих контейнеров
        let got = store.query("n1", "c1", 10).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].line, "first");
        assert_eq!(got[1].line, "second");

        // tail меньше количества строк — берём последние
        let got = store.query("n1", "c1", 1).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].line, "second");

        let containers = store.list_containers().unwrap();
        assert_eq!(containers.len(), 3);

        // prune: старая запись уходит, свежие остаются
        store.append(&[entry("n1", "c1", now - 10 * 24 * 3600 * 1000, "ancient")]).unwrap();
        let removed = store.prune(24 * 3600).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(store.query("n1", "c1", 10).unwrap().len(), 2);
    }
}
