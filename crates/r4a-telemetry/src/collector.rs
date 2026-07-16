use crate::{LogEntry, LogsChTarget};
use bollard::container::{ListContainersOptions, LogOutput, LogsOptions};
use bollard::Docker;
use futures_util::StreamExt;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::{debug, info, warn};

const CONFIG_POLL_SECS: u64 = 30;
const SCAN_INTERVAL_SECS: u64 = 15;
const FLUSH_INTERVAL_SECS: u64 = 2;
const MAX_BATCH: usize = 200;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Последний отгруженный в ClickHouse docker-timestamp по каждому контейнеру.
/// Персистится в файл, чтобы после рестарта агента продолжить с места остановки
/// (история доезжает, дублей нет).
struct ShipState {
    path: PathBuf,
    last_ts: Mutex<HashMap<String, u64>>,
}

impl ShipState {
    fn load(path: PathBuf) -> Self {
        let last_ts = std::fs::read(&path)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();
        Self {
            path,
            last_ts: Mutex::new(last_ts),
        }
    }

    fn get(&self, container: &str) -> u64 {
        *self.last_ts.lock().unwrap().get(container).unwrap_or(&0)
    }

    fn advance(&self, container: &str, ts_ms: u64) {
        let mut map = self.last_ts.lock().unwrap();
        let cur = map.entry(container.to_string()).or_insert(0);
        if ts_ms <= *cur {
            return;
        }
        *cur = ts_ms;
        if let Ok(data) = serde_json::to_vec(&*map) {
            if let Err(e) = std::fs::write(&self.path, data) {
                debug!("Telemetry: failed to persist logs-state: {}", e);
            }
        }
    }
}

/// Запускается на агенте: ждёт, пока на мастере настроят ClickHouse
/// (GET /api/logs/agent-config), затем следит за r4a-контейнерами
/// (label `r4a.node=<node>`) и шипит их логи напрямую в ClickHouse.
/// Никогда не возвращается — вызывать в tokio::spawn.
pub async fn run_collector(
    node_name: String,
    master_base: String,
    cluster_secret: String,
    state_path: PathBuf,
) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    // Логи опциональны: пока пользователь не задеплоил ClickHouse, мастер отвечает 404
    let target = loop {
        match client
            .get(format!("{}/api/logs/agent-config", master_base))
            .header("X-R4A-Secret", &cluster_secret)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => match resp.json::<LogsChTarget>().await {
                Ok(t) => break t,
                Err(e) => debug!("Telemetry: bad agent-config: {}", e),
            },
            Ok(_) => {} // 404 — логи не настроены
            Err(e) => debug!("Telemetry: agent-config poll failed: {}", e),
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(CONFIG_POLL_SECS)).await;
    };

    info!(
        "Telemetry collector started (node={}, clickhouse={})",
        node_name, target.endpoint
    );

    let docker = match Docker::connect_with_local_defaults() {
        Ok(d) => d,
        Err(e) => {
            warn!(
                "Telemetry collector: Docker connect failed, collector disabled: {}",
                e
            );
            return;
        }
    };

    let state = Arc::new(ShipState::load(state_path));
    let target = Arc::new(Mutex::new(target));

    // Config can change when the user redeploys ClickHouse (new node/password).
    // Keep following existing Docker streams, but ship to the latest target.
    let refresh_client = client.clone();
    let refresh_master_base = master_base.clone();
    let refresh_secret = cluster_secret.clone();
    let refresh_target = target.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(CONFIG_POLL_SECS)).await;
            match refresh_client
                .get(format!("{}/api/logs/agent-config", refresh_master_base))
                .header("X-R4A-Secret", &refresh_secret)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => match resp.json::<LogsChTarget>().await {
                    Ok(next) => {
                        let mut cur = refresh_target.lock().unwrap();
                        if cur.endpoint != next.endpoint || cur.password != next.password {
                            info!("Telemetry: ClickHouse target updated ({})", next.endpoint);
                            *cur = next;
                        }
                    }
                    Err(e) => debug!("Telemetry: bad refreshed agent-config: {}", e),
                },
                Ok(resp) => debug!("Telemetry: agent-config refresh HTTP {}", resp.status()),
                Err(e) => debug!("Telemetry: agent-config refresh failed: {}", e),
            }
        }
    });

    // Контейнеры, за которыми уже следим (id). Follow-задача сама удаляет
    // свой id при завершении стрима (контейнер остановлен/удалён).
    let tracked: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    loop {
        let mut filters = HashMap::new();
        filters.insert("label".to_string(), vec![format!("r4a.node={}", node_name)]);
        let opts = ListContainersOptions {
            all: false,
            filters,
            ..Default::default()
        };

        match docker.list_containers(Some(opts)).await {
            Ok(containers) => {
                for c in containers {
                    let id = match c.id.clone() {
                        Some(id) => id,
                        None => continue,
                    };
                    let name = c
                        .names
                        .as_ref()
                        .and_then(|ns| ns.first())
                        .map(|n| n.trim_start_matches('/').to_string())
                        .unwrap_or_else(|| id.clone());

                    let is_new = tracked.lock().unwrap().insert(id.clone());
                    if !is_new {
                        continue;
                    }

                    info!("Telemetry: following logs of container {}", name);
                    tokio::spawn(follow_container(
                        docker.clone(),
                        client.clone(),
                        tracked.clone(),
                        state.clone(),
                        target.clone(),
                        id,
                        name,
                        node_name.clone(),
                    ));
                }
            }
            Err(e) => debug!("Telemetry: list_containers failed: {}", e),
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(SCAN_INTERVAL_SECS)).await;
    }
}

/// "2026-07-13T12:34:56.789012345Z <текст>" → (ts_ms, текст).
/// Docker с timestamps=true префиксует так каждую запись.
fn split_docker_ts(chunk: &str) -> (Option<u64>, &str) {
    if let Some(space) = chunk.find(' ') {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&chunk[..space]) {
            return (Some(dt.timestamp_millis() as u64), &chunk[space + 1..]);
        }
    }
    (None, chunk)
}

#[allow(clippy::too_many_arguments)]
async fn follow_container(
    docker: Docker,
    client: reqwest::Client,
    tracked: Arc<Mutex<HashSet<String>>>,
    state: Arc<ShipState>,
    target: Arc<Mutex<LogsChTarget>>,
    id: String,
    name: String,
    node: String,
) {
    let last_shipped = state.get(&name);
    let opts = LogsOptions::<String> {
        follow: true,
        stdout: true,
        stderr: true,
        timestamps: true,
        // С последнего отгруженного места (0 = вся история docker'а).
        // Секундная гранулярность since → точная дедупликация ниже по ts_ms.
        since: (last_shipped / 1000) as i64,
        ..Default::default()
    };

    let mut stream = docker.logs(&id, Some(opts));
    let mut buf: Vec<LogEntry> = Vec::new();
    let mut flush_tick =
        tokio::time::interval(tokio::time::Duration::from_secs(FLUSH_INTERVAL_SECS));

    loop {
        tokio::select! {
            item = stream.next() => {
                match item {
                    Some(Ok(output)) => {
                        let (stream_name, bytes) = match &output {
                            LogOutput::StdOut { message } => ("stdout", message),
                            LogOutput::StdErr { message } => ("stderr", message),
                            LogOutput::Console { message } => ("stdout", message),
                            LogOutput::StdIn { message } => ("stdin", message),
                        };
                        let chunk = String::from_utf8_lossy(bytes);
                        let (ts, text) = split_docker_ts(&chunk);
                        let ts_ms = ts.unwrap_or_else(now_ms);
                        if ts_ms <= last_shipped {
                            continue; // уже в ClickHouse с прошлого запуска
                        }
                        for line in text.split('\n') {
                            let line = line.trim_end_matches('\r');
                            if line.is_empty() {
                                continue;
                            }
                            buf.push(LogEntry {
                                node: node.clone(),
                                container: name.clone(),
                                ts_ms,
                                stream: stream_name.to_string(),
                                line: line.to_string(),
                            });
                        }
                        if buf.len() >= MAX_BATCH {
                            ship(&client, &target, &state, &name, &mut buf).await;
                        }
                    }
                    Some(Err(e)) => {
                        debug!("Telemetry: log stream error for {}: {}", name, e);
                        break;
                    }
                    None => break, // контейнер остановлен
                }
            }
            _ = flush_tick.tick() => {
                if !buf.is_empty() {
                    ship(&client, &target, &state, &name, &mut buf).await;
                }
            }
        }
    }

    ship(&client, &target, &state, &name, &mut buf).await;
    tracked.lock().unwrap().remove(&id);
    info!("Telemetry: stopped following {} (stream ended)", name);
}

async fn ship(
    client: &reqwest::Client,
    target: &Mutex<LogsChTarget>,
    state: &ShipState,
    container: &str,
    buf: &mut Vec<LogEntry>,
) {
    if buf.is_empty() {
        return;
    }
    let entries = std::mem::take(buf);
    let mut body = String::with_capacity(entries.len() * 128);
    for e in &entries {
        if let Ok(json) = serde_json::to_string(e) {
            body.push_str(&json);
            body.push('\n');
        }
    }
    let target = target.lock().unwrap().clone();
    let res = client
        .post(format!(
            "{}/?query=INSERT%20INTO%20r4a.logs%20FORMAT%20JSONEachRow",
            target.endpoint
        ))
        .basic_auth("default", Some(&target.password))
        .body(body)
        .send()
        .await;
    match res {
        Ok(resp) if resp.status().is_success() => {
            if let Some(max_ts) = entries.iter().map(|e| e.ts_ms).max() {
                state.advance(container, max_ts);
            }
        }
        Ok(resp) => {
            debug!(
                "Telemetry: ClickHouse rejected batch: HTTP {}",
                resp.status()
            );
            // строки уже взяты из buf — при ошибке возвращаем, чтобы не терять
            buf.extend(entries);
        }
        Err(e) => {
            debug!("Telemetry: ship failed: {}", e);
            buf.extend(entries);
        }
    }
    // Защита от бесконечного роста при недоступном ClickHouse
    if buf.len() > 10 * MAX_BATCH {
        let drop_n = buf.len() - 10 * MAX_BATCH;
        buf.drain(..drop_n);
    }
}
