use crate::{LogBatch, LogEntry};
use bollard::container::{ListContainersOptions, LogOutput, LogsOptions};
use bollard::Docker;
use futures_util::StreamExt;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tracing::{debug, info, warn};

const SCAN_INTERVAL_SECS: u64 = 15;
const FLUSH_INTERVAL_SECS: u64 = 2;
const MAX_BATCH: usize = 200;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Запускается на агенте: следит за r4a-контейнерами (label `r4a.node=<node>`),
/// стримит их логи и батчами отправляет на мастер (`POST /api/logs`).
/// Никогда не возвращается — вызывать в tokio::spawn.
pub async fn run_collector(node_name: String, master_base: String, cluster_secret: String) {
    let docker = match Docker::connect_with_local_defaults() {
        Ok(d) => d,
        Err(e) => {
            warn!("Telemetry collector: Docker connect failed, collector disabled: {}", e);
            return;
        }
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    // Контейнеры, за которыми уже следим (id). Follow-задача сама удаляет
    // свой id при завершении стрима (контейнер остановлен/удалён).
    let tracked: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    info!("Telemetry collector started (node={})", node_name);

    loop {
        let mut filters = HashMap::new();
        filters.insert("label".to_string(), vec![format!("r4a.node={}", node_name)]);
        let opts = ListContainersOptions { all: false, filters, ..Default::default() };

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
                        id,
                        name,
                        node_name.clone(),
                        master_base.clone(),
                        cluster_secret.clone(),
                    ));
                }
            }
            Err(e) => debug!("Telemetry: list_containers failed: {}", e),
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(SCAN_INTERVAL_SECS)).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn follow_container(
    docker: Docker,
    client: reqwest::Client,
    tracked: Arc<Mutex<HashSet<String>>>,
    id: String,
    name: String,
    node: String,
    master_base: String,
    secret: String,
) {
    let opts = LogsOptions::<String> {
        follow: true,
        stdout: true,
        stderr: true,
        // Только новые строки: история и так лежит в store с прошлого follow,
        // а после рестарта агента не заливаем дубли.
        since: (now_ms() / 1000) as i64,
        ..Default::default()
    };

    let mut stream = docker.logs(&id, Some(opts));
    let mut buf: Vec<LogEntry> = Vec::new();
    let mut flush_tick = tokio::time::interval(tokio::time::Duration::from_secs(FLUSH_INTERVAL_SECS));

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
                        for line in String::from_utf8_lossy(bytes).split('\n') {
                            let line = line.trim_end_matches('\r');
                            if line.is_empty() {
                                continue;
                            }
                            buf.push(LogEntry {
                                node: node.clone(),
                                container: name.clone(),
                                ts_ms: now_ms(),
                                stream: stream_name.to_string(),
                                line: line.to_string(),
                            });
                        }
                        if buf.len() >= MAX_BATCH {
                            ship(&client, &master_base, &secret, &mut buf).await;
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
                    ship(&client, &master_base, &secret, &mut buf).await;
                }
            }
        }
    }

    ship(&client, &master_base, &secret, &mut buf).await;
    tracked.lock().unwrap().remove(&id);
    info!("Telemetry: stopped following {} (stream ended)", name);
}

async fn ship(client: &reqwest::Client, master_base: &str, secret: &str, buf: &mut Vec<LogEntry>) {
    if buf.is_empty() {
        return;
    }
    let batch = LogBatch { entries: std::mem::take(buf) };
    let res = client
        .post(format!("{}/api/logs", master_base))
        .header("X-R4A-Secret", secret)
        .json(&batch)
        .send()
        .await;
    match res {
        Ok(resp) if resp.status().is_success() => {}
        Ok(resp) => {
            debug!("Telemetry: master rejected batch: HTTP {}", resp.status());
            // строки уже взяты из buf — при ошибке возвращаем, чтобы не терять
            buf.extend(batch.entries);
        }
        Err(e) => {
            debug!("Telemetry: ship failed: {}", e);
            buf.extend(batch.entries);
        }
    }
    // Защита от бесконечного роста при недоступном мастере
    if buf.len() > 10 * MAX_BATCH {
        let drop_n = buf.len() - 10 * MAX_BATCH;
        buf.drain(..drop_n);
    }
}
