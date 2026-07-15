use crate::MetricPoint;
use anyhow::Result;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

const METRICS_TREE: &str = "metrics";

/// Отдельный sled-инстанс телеметрии на мастере (история метрик нод).
/// Логи контейнеров живут в ClickHouse, здесь их больше нет.
#[derive(Clone)]
pub struct LogStore {
    db: sled::Db,
    seq: Arc<AtomicU32>,
}

impl LogStore {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let db = sled::open(path)?;
        Ok(Self { db, seq: Arc::new(AtomicU32::new(0)) })
    }

    /// Точка истории метрик ноды. Ключ: `{node}\0{ts_ms:016x}{seq:08x}`.
    pub fn append_metric(&self, p: &MetricPoint) -> Result<()> {
        let tree = self.db.open_tree(METRICS_TREE)?;
        let mut key = Vec::with_capacity(p.node.len() + 25);
        key.extend_from_slice(p.node.as_bytes());
        key.push(0);
        key.extend_from_slice(format!("{:016x}", p.ts_ms).as_bytes());
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        key.extend_from_slice(format!("{:08x}", seq).as_bytes());
        tree.insert(key, serde_json::to_vec(p)?)?;
        Ok(())
    }

    /// Последние `tail` точек метрик ноды в хронологическом порядке.
    pub fn query_metrics(&self, node: &str, tail: usize) -> Result<Vec<MetricPoint>> {
        let tree = self.db.open_tree(METRICS_TREE)?;
        let mut prefix = Vec::with_capacity(node.len() + 1);
        prefix.extend_from_slice(node.as_bytes());
        prefix.push(0);
        let mut out: Vec<MetricPoint> = Vec::with_capacity(tail);
        for item in tree.scan_prefix(&prefix).rev().take(tail) {
            let (_, v) = item?;
            if let Ok(p) = serde_json::from_slice::<MetricPoint>(&v) {
                out.push(p);
            }
        }
        out.reverse();
        Ok(out)
    }

    /// Удаляет точки метрик старше `max_age_secs`. Возвращает число удалённых.
    pub fn prune_metrics(&self, max_age_secs: u64) -> Result<usize> {
        let cutoff_ms = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_sub(max_age_secs)) * 1000;

        let tree = self.db.open_tree(METRICS_TREE)?;
        let mut removed = 0;
        for item in tree.iter() {
            let (k, v) = item?;
            let old = serde_json::from_slice::<MetricPoint>(&v)
                .map(|p| p.ts_ms < cutoff_ms)
                .unwrap_or(true);
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

    fn point(node: &str, ts_ms: u64, cpu: f32) -> MetricPoint {
        MetricPoint {
            node: node.to_string(),
            ts_ms,
            cpu_percent: cpu,
            ram_used_mb: 100,
            ram_total_mb: 1000,
            vram_used_mb: None,
            vram_total_mb: None,
        }
    }

    #[test]
    fn metrics_append_query_prune() {
        let dir = tempfile::tempdir().unwrap();
        let store = LogStore::open(dir.path()).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;

        store.append_metric(&point("n1", now - 1000, 10.0)).unwrap();
        store.append_metric(&point("n1", now, 20.0)).unwrap();
        store.append_metric(&point("n2", now, 30.0)).unwrap();

        // хронологический порядок, без чужих нод
        let got = store.query_metrics("n1", 10).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].cpu_percent, 10.0);
        assert_eq!(got[1].cpu_percent, 20.0);

        // tail меньше количества точек — берём последние
        let got = store.query_metrics("n1", 1).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].cpu_percent, 20.0);

        // prune: старая точка уходит, свежие остаются
        store.append_metric(&point("n1", now - 10 * 24 * 3600 * 1000, 5.0)).unwrap();
        let removed = store.prune_metrics(24 * 3600).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(store.query_metrics("n1", 10).unwrap().len(), 2);
    }
}
