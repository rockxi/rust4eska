/// Git HTTP Smart Protocol via `git http-backend` CGI forwarding.
///
/// Mount with: .route("/git/*path", any(git_handler).with_state(git_root))
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, Request, Response, StatusCode},
    response::IntoResponse,
};
use bytes::Bytes;
use http_body_util::BodyExt;
use std::{collections::HashMap, path::PathBuf, process::Stdio};
use tokio::io::AsyncWriteExt;
use tracing::warn;

pub async fn git_handler(
    State(git_root): State<PathBuf>,
    method: Method,
    Path(path): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
    req: Request<Body>,
) -> impl IntoResponse {
    let body_bytes = match req.into_body().collect().await {
        Ok(b) => b.to_bytes(),
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "cannot read body"),
    };

    match run_git_backend(&git_root, &method, &path, &params, &headers, body_bytes).await {
        Ok(resp) => resp,
        Err(e) => {
            warn!("git http-backend error: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "git backend error")
        }
    }
}

async fn run_git_backend(
    git_root: &PathBuf,
    method: &Method,
    path: &str,
    params: &HashMap<String, String>,
    headers: &HeaderMap,
    body: Bytes,
) -> anyhow::Result<Response<Body>> {
    let query_string = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&");

    // PATH_INFO должен начинаться с /
    let path_info = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };

    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("http-backend");
    cmd.env("GIT_PROJECT_ROOT", git_root);
    cmd.env("GIT_HTTP_EXPORT_ALL", "1");
    cmd.env("PATH_INFO", &path_info);
    cmd.env("REQUEST_METHOD", method.as_str());
    cmd.env("QUERY_STRING", &query_string);
    cmd.env("CONTENT_TYPE", &content_type);
    cmd.env("CONTENT_LENGTH", body.len().to_string());

    // Передаём HTTP-заголовки как переменные окружения HTTP_*
    for (k, v) in headers.iter() {
        let env_key = format!("HTTP_{}", k.as_str().replace('-', "_").to_uppercase());
        if let Ok(val) = v.to_str() {
            cmd.env(env_key, val);
        }
    }

    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::inherit());

    let mut child = cmd.spawn()?;

    // Пишем тело запроса в stdin
    if !body.is_empty() {
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&body).await?;
        }
    }

    let output = child.wait_with_output().await?;

    if !output.status.success() && output.stdout.is_empty() {
        anyhow::bail!("git http-backend exited with {}", output.status);
    }

    parse_cgi_response(output.stdout)
}

/// Разобрать CGI-ответ: заголовки\r\n\r\nтело
fn parse_cgi_response(raw: Vec<u8>) -> anyhow::Result<Response<Body>> {
    // Ищем разделитель заголовков (\r\n\r\n или \n\n)
    let split_pos = find_header_end(&raw)
        .ok_or_else(|| anyhow::anyhow!("no header separator in CGI response"))?;

    let header_bytes = &raw[..split_pos];
    let body_start = split_pos
        + if raw.get(split_pos) == Some(&b'\r') {
            4
        } else {
            2
        };
    let body_bytes = raw[body_start..].to_vec();

    let header_str = std::str::from_utf8(header_bytes)?;

    let mut status = StatusCode::OK;
    let mut resp = Response::builder();

    for line in header_str.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            let val = v.trim();
            if key.eq_ignore_ascii_case("Status") {
                if let Some(code) = val.split_whitespace().next() {
                    if let Ok(c) = code.parse::<u16>() {
                        status = StatusCode::from_u16(c).unwrap_or(StatusCode::OK);
                    }
                }
            } else {
                resp = resp.header(key, val);
            }
        }
    }

    Ok(resp.status(status).body(Body::from(body_bytes))?)
}

fn find_header_end(data: &[u8]) -> Option<usize> {
    // ищем \r\n\r\n
    for i in 0..data.len().saturating_sub(3) {
        if &data[i..i + 4] == b"\r\n\r\n" {
            return Some(i);
        }
    }
    // или \n\n
    for i in 0..data.len().saturating_sub(1) {
        if &data[i..i + 2] == b"\n\n" {
            return Some(i);
        }
    }
    None
}

fn error_response(status: StatusCode, msg: &'static str) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(Body::from(msg))
        .unwrap()
}
