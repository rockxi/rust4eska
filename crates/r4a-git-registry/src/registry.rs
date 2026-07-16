use anyhow::{anyhow, bail, Context, Result};
use argon2::{password_hash::PasswordHash, Argon2, PasswordVerifier};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{
        header::{self, HeaderName, HeaderValue},
        HeaderMap, Method, Request, Response, StatusCode,
    },
    response::IntoResponse,
};
use base64::Engine;
use http_body_util::BodyExt;
use r4a_core::models::{Resource, Token, User, Verb};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path as StdPath, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
};
use tokio_util::io::ReaderStream;
use tracing::warn;
use uuid::Uuid;

use crate::RegistryState;

const DIST_API_VERSION: &str = "registry/2.0";
const META_TREE: &str = "registry_meta";
const DEFAULT_MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ManifestRecord {
    repo: String,
    digest: String,
    media_type: String,
    size: u64,
    created_at: u64,
    body: Vec<u8>,
}

#[derive(Debug, Serialize)]
struct TagsListResponse {
    name: String,
    tags: Vec<String>,
}

#[derive(Clone)]
enum AuthIdentity {
    Token(Token),
    User(String),
}

pub async fn registry_handler(
    State(state): State<RegistryState>,
    method: Method,
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    req: Request<Body>,
) -> impl IntoResponse {
    let normalized = path.trim_matches('/').to_string();
    match handle_request(state, method, normalized, query, headers, req).await {
        Ok(resp) => resp,
        Err(err) => {
            warn!("registry error: {err:#}");
            if let Some(status) = err.downcast_ref::<StatusCode>() {
                return match *status {
                    StatusCode::UNAUTHORIZED => unauthorized_response(),
                    s => error_response(s, "registry error"),
                };
            }
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "registry error")
        }
    }
}

pub async fn registry_root_handler(
    State(state): State<RegistryState>,
    method: Method,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    req: Request<Body>,
) -> impl IntoResponse {
    match handle_request(state, method, String::new(), query, headers, req).await {
        Ok(resp) => resp,
        Err(err) => {
            warn!("registry error: {err:#}");
            if let Some(status) = err.downcast_ref::<StatusCode>() {
                return match *status {
                    StatusCode::UNAUTHORIZED => unauthorized_response(),
                    s => error_response(s, "registry error"),
                };
            }
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "registry error")
        }
    }
}

async fn handle_request(
    state: RegistryState,
    method: Method,
    path: String,
    query: HashMap<String, String>,
    headers: HeaderMap,
    req: Request<Body>,
) -> Result<Response<Body>> {
    ensure_layout(&state.root).await?;

    if path.is_empty() {
        let auth = match authenticate(&state, &headers)? {
            Some(auth) => auth,
            None => return Ok(unauthorized_response()),
        };
        if !matches!(method, Method::GET | Method::HEAD) {
            ensure_write_allowed(&state, &auth)?;
        }
        return match method {
            Method::GET | Method::HEAD => Ok(with_standard_headers(
                Response::builder()
                    .status(StatusCode::OK)
                    .body(Body::empty())?,
            )),
            _ => Ok(error_response(
                StatusCode::METHOD_NOT_ALLOWED,
                "method not allowed",
            )),
        };
    }

    let auth = match authenticate(&state, &headers)? {
        Some(auth) => auth,
        None => return Ok(unauthorized_response()),
    };

    if !matches!(method, Method::GET | Method::HEAD) {
        ensure_write_allowed(&state, &auth)?;
    }

    if let Some(repo) = path.strip_suffix("/tags/list") {
        return match method {
            Method::GET => tags_list(&state, repo).await,
            _ => Ok(error_response(
                StatusCode::METHOD_NOT_ALLOWED,
                "method not allowed",
            )),
        };
    }

    if path.ends_with("/blobs/uploads") {
        let repo = path
            .strip_suffix("/blobs/uploads")
            .ok_or_else(|| anyhow!("invalid upload path"))?;
        return match method {
            Method::POST => start_upload(&state, repo).await,
            _ => Ok(error_response(
                StatusCode::METHOD_NOT_ALLOWED,
                "method not allowed",
            )),
        };
    }

    if let Some((repo, upload_id)) = path.split_once("/blobs/uploads/") {
        return match method {
            Method::PATCH => patch_upload(&state, repo, upload_id, req).await,
            Method::PUT => complete_upload(&state, repo, upload_id, query.get("digest"), req).await,
            _ => Ok(error_response(
                StatusCode::METHOD_NOT_ALLOWED,
                "method not allowed",
            )),
        };
    }

    if let Some((repo, digest)) = path.split_once("/blobs/") {
        return match method {
            Method::HEAD => head_blob(&state, repo, digest).await,
            Method::GET => get_blob(&state, repo, digest).await,
            _ => Ok(error_response(
                StatusCode::METHOD_NOT_ALLOWED,
                "method not allowed",
            )),
        };
    }

    if let Some((repo, reference)) = path.split_once("/manifests/") {
        return match method {
            Method::GET => get_manifest(&state, repo, reference).await,
            Method::PUT => put_manifest(&state, repo, reference, &headers, req).await,
            Method::DELETE => delete_manifest(&state, repo, reference).await,
            _ => Ok(error_response(
                StatusCode::METHOD_NOT_ALLOWED,
                "method not allowed",
            )),
        };
    }

    Ok(error_response(StatusCode::NOT_FOUND, "not found"))
}

fn authenticate(state: &RegistryState, headers: &HeaderMap) -> Result<Option<AuthIdentity>> {
    if let Some(auth_header) = headers.get(header::AUTHORIZATION) {
        let auth_str = match auth_header.to_str() {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };
        if let Some(token_id) = auth_str.strip_prefix("Bearer ") {
            return Ok(state.store.get_token(token_id)?.map(AuthIdentity::Token));
        }
        if let Some(encoded) = auth_str.strip_prefix("Basic ") {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .context("decode basic auth")?;
            let creds = String::from_utf8(decoded).context("basic auth utf8")?;
            let (username, password) = match creds.split_once(':') {
                Some(parts) => parts,
                None => return Ok(None),
            };
            if verify_user_password(&state.store, username, password)? {
                return Ok(Some(AuthIdentity::User(username.to_string())));
            }
            return Ok(None);
        }
    }
    Ok(None)
}

fn verify_user_password(store: &r4a_store::Store, username: &str, password: &str) -> Result<bool> {
    let Some(raw) = store.get("users", username.as_bytes())? else {
        return Ok(false);
    };
    let user: User = serde_json::from_slice(&raw)?;
    let parsed =
        PasswordHash::new(&user.password_hash).map_err(|e| anyhow!("parse password hash: {e}"))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

fn ensure_write_allowed(state: &RegistryState, auth: &AuthIdentity) -> Result<()> {
    let username = match auth {
        AuthIdentity::Token(token) => &token.username,
        AuthIdentity::User(username) => username,
    };
    let allowed = state
        .store
        .can(username, Verb::Create, Resource::Registry, None)
        || state
            .store
            .can(username, Verb::Update, Resource::Registry, None)
        || state
            .store
            .can(username, Verb::Delete, Resource::Registry, None)
        || state
            .store
            .can(username, Verb::All, Resource::Registry, None)
        || state.store.can(username, Verb::All, Resource::All, None);
    if allowed {
        Ok(())
    } else {
        Err(anyhow!(StatusCode::FORBIDDEN))
    }
}

async fn tags_list(state: &RegistryState, repo: &str) -> Result<Response<Body>> {
    let tree = state.store.db.open_tree(META_TREE)?;
    let prefix = tag_prefix(repo);
    let mut tags = Vec::new();
    for item in tree.scan_prefix(prefix.as_bytes()) {
        let (key, _) = item?;
        let key = String::from_utf8_lossy(&key);
        if let Some(tag) = key.strip_prefix(&prefix) {
            tags.push(tag.to_string());
        }
    }
    tags.sort();
    let body = serde_json::to_vec(&TagsListResponse {
        name: repo.to_string(),
        tags,
    })?;
    json_response(StatusCode::OK, body)
}

async fn start_upload(state: &RegistryState, repo: &str) -> Result<Response<Body>> {
    let upload_id = Uuid::new_v4().to_string();
    fs::create_dir_all(uploads_dir(&state.root)).await?;
    fs::write(uploads_dir(&state.root).join(&upload_id), []).await?;
    let location = format!("/v2/{repo}/blobs/uploads/{upload_id}");
    Ok(with_standard_headers(
        Response::builder()
            .status(StatusCode::ACCEPTED)
            .header(header::LOCATION, location)
            .header(header::RANGE, "0-0")
            .header("Docker-Upload-UUID", upload_id)
            .body(Body::empty())?,
    ))
}

async fn patch_upload(
    state: &RegistryState,
    repo: &str,
    upload_id: &str,
    req: Request<Body>,
) -> Result<Response<Body>> {
    let path = uploads_dir(&state.root).join(upload_id);
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await?;
    let mut written = file.metadata().await?.len();
    let mut body = req.into_body();
    while let Some(frame) = body.frame().await {
        let frame = frame?;
        if let Ok(data) = frame.into_data() {
            file.write_all(&data).await?;
            written += data.len() as u64;
        }
    }
    file.flush().await?;
    let location = format!("/v2/{repo}/blobs/uploads/{upload_id}");
    Ok(with_standard_headers(
        Response::builder()
            .status(StatusCode::ACCEPTED)
            .header(header::LOCATION, location)
            .header(header::RANGE, format!("0-{written}"))
            .header("Docker-Upload-UUID", upload_id)
            .body(Body::empty())?,
    ))
}

async fn complete_upload(
    state: &RegistryState,
    repo: &str,
    upload_id: &str,
    digest: Option<&String>,
    req: Request<Body>,
) -> Result<Response<Body>> {
    let Some(expected_digest) = digest else {
        return Ok(error_response(
            StatusCode::BAD_REQUEST,
            "digest query is required",
        ));
    };
    validate_digest(expected_digest)?;

    let upload_path = uploads_dir(&state.root).join(upload_id);
    if !fs::try_exists(&upload_path).await? {
        return Ok(error_response(StatusCode::NOT_FOUND, "upload not found"));
    }

    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&upload_path)
        .await?;
    let mut body = req.into_body();
    while let Some(frame) = body.frame().await {
        let frame = frame?;
        if let Ok(data) = frame.into_data() {
            file.write_all(&data).await?;
        }
    }
    file.flush().await?;
    drop(file);

    let computed = sha256_file(&upload_path).await?;
    if computed != *expected_digest {
        let _ = fs::remove_file(&upload_path).await;
        return Ok(error_response(StatusCode::BAD_REQUEST, "digest mismatch"));
    }

    let final_path = blob_path(&state.root, expected_digest)?;
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent).await?;
    }
    if fs::try_exists(&final_path).await? {
        let _ = fs::remove_file(&upload_path).await;
    } else {
        fs::rename(&upload_path, &final_path).await?;
    }

    Ok(with_standard_headers(
        Response::builder()
            .status(StatusCode::CREATED)
            .header(
                header::LOCATION,
                format!("/v2/{repo}/blobs/{expected_digest}"),
            )
            .header("Docker-Content-Digest", expected_digest)
            .body(Body::empty())?,
    ))
}

async fn head_blob(state: &RegistryState, _repo: &str, digest: &str) -> Result<Response<Body>> {
    validate_digest(digest)?;
    let path = blob_path(&state.root, digest)?;
    if !fs::try_exists(&path).await? {
        return Ok(error_response(StatusCode::NOT_FOUND, "blob not found"));
    }
    let size = fs::metadata(path).await?.len();
    Ok(with_standard_headers(
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_LENGTH, size.to_string())
            .header("Docker-Content-Digest", digest)
            .body(Body::empty())?,
    ))
}

async fn get_blob(state: &RegistryState, _repo: &str, digest: &str) -> Result<Response<Body>> {
    validate_digest(digest)?;
    let path = blob_path(&state.root, digest)?;
    if !fs::try_exists(&path).await? {
        return Ok(error_response(StatusCode::NOT_FOUND, "blob not found"));
    }
    let size = fs::metadata(&path).await?.len();
    let file = fs::File::open(path).await?;
    let stream = ReaderStream::new(file);
    Ok(with_standard_headers(
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(header::CONTENT_LENGTH, size.to_string())
            .header("Docker-Content-Digest", digest)
            .body(Body::from_stream(stream))?,
    ))
}

async fn get_manifest(
    state: &RegistryState,
    repo: &str,
    reference: &str,
) -> Result<Response<Body>> {
    let record = resolve_manifest(state, repo, reference)?;
    Ok(with_standard_headers(
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, record.media_type)
            .header(header::CONTENT_LENGTH, record.size.to_string())
            .header("Docker-Content-Digest", record.digest)
            .body(Body::from(record.body))?,
    ))
}

async fn put_manifest(
    state: &RegistryState,
    repo: &str,
    reference: &str,
    headers: &HeaderMap,
    req: Request<Body>,
) -> Result<Response<Body>> {
    let media_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or(DEFAULT_MANIFEST_MEDIA_TYPE)
        .to_string();
    let body = req.into_body().collect().await?.to_bytes().to_vec();
    let digest = format!("sha256:{:x}", Sha256::digest(&body));

    let tree = state.store.db.open_tree(META_TREE)?;
    let record = ManifestRecord {
        repo: repo.to_string(),
        digest: digest.clone(),
        media_type,
        size: body.len() as u64,
        created_at: now_ts(),
        body,
    };
    tree.insert(
        manifest_key(repo, &digest).as_bytes(),
        serde_json::to_vec(&record)?,
    )?;
    tree.insert(tag_key(repo, reference).as_bytes(), digest.as_bytes())?;
    state.store.db.flush_async().await?;

    Ok(with_standard_headers(
        Response::builder()
            .status(StatusCode::CREATED)
            .header("Docker-Content-Digest", digest)
            .header(
                header::LOCATION,
                format!("/v2/{repo}/manifests/{reference}"),
            )
            .body(Body::empty())?,
    ))
}

async fn delete_manifest(
    state: &RegistryState,
    repo: &str,
    reference: &str,
) -> Result<Response<Body>> {
    let tree = state.store.db.open_tree(META_TREE)?;
    if reference.starts_with("sha256:") {
        tree.remove(manifest_key(repo, reference).as_bytes())?;
        let prefix = tag_prefix(repo);
        let mut tags_to_remove = Vec::new();
        for item in tree.scan_prefix(prefix.as_bytes()) {
            let (key, value) = item?;
            if value.as_ref() == reference.as_bytes() {
                tags_to_remove.push(key);
            }
        }
        for key in tags_to_remove {
            tree.remove(key)?;
        }
    } else {
        tree.remove(tag_key(repo, reference).as_bytes())?;
    }
    state.store.db.flush_async().await?;
    Ok(with_standard_headers(
        Response::builder()
            .status(StatusCode::ACCEPTED)
            .body(Body::empty())?,
    ))
}

fn resolve_manifest(state: &RegistryState, repo: &str, reference: &str) -> Result<ManifestRecord> {
    let tree = state.store.db.open_tree(META_TREE)?;
    let digest = if reference.starts_with("sha256:") {
        reference.to_string()
    } else {
        let Some(raw) = tree.get(tag_key(repo, reference).as_bytes())? else {
            return Err(anyhow!(StatusCode::NOT_FOUND));
        };
        String::from_utf8(raw.to_vec())?
    };
    let Some(raw) = tree.get(manifest_key(repo, &digest).as_bytes())? else {
        return Err(anyhow!(StatusCode::NOT_FOUND));
    };
    Ok(serde_json::from_slice(&raw)?)
}

fn json_response(status: StatusCode, body: Vec<u8>) -> Result<Response<Body>> {
    Ok(with_standard_headers(
        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CONTENT_LENGTH, body.len().to_string())
            .body(Body::from(body))?,
    ))
}

fn unauthorized_response() -> Response<Body> {
    with_standard_headers(
        Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header(
                header::WWW_AUTHENTICATE,
                "Basic realm=\"r4a-registry\", charset=\"UTF-8\"",
            )
            .body(Body::from("authentication required"))
            .unwrap(),
    )
}

fn error_response(status: StatusCode, msg: &'static str) -> Response<Body> {
    with_standard_headers(
        Response::builder()
            .status(status)
            .body(Body::from(msg))
            .unwrap(),
    )
}

fn with_standard_headers(mut resp: Response<Body>) -> Response<Body> {
    resp.headers_mut().insert(
        HeaderName::from_static("docker-distribution-api-version"),
        HeaderValue::from_static(DIST_API_VERSION),
    );
    resp
}

async fn ensure_layout(root: &StdPath) -> Result<()> {
    fs::create_dir_all(blobs_root(root)).await?;
    fs::create_dir_all(uploads_dir(root)).await?;
    Ok(())
}

fn blobs_root(root: &StdPath) -> PathBuf {
    root.join("blobs").join("sha256")
}

fn uploads_dir(root: &StdPath) -> PathBuf {
    root.join("_uploads")
}

fn blob_path(root: &StdPath, digest: &str) -> Result<PathBuf> {
    let hex = digest
        .strip_prefix("sha256:")
        .ok_or_else(|| anyhow!("unsupported digest"))?;
    validate_digest(digest)?;
    Ok(blobs_root(root).join(&hex[..2]).join(hex))
}

fn validate_digest(digest: &str) -> Result<()> {
    let hex = digest
        .strip_prefix("sha256:")
        .ok_or_else(|| anyhow!("unsupported digest"))?;
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("invalid digest");
    }
    Ok(())
}

fn tag_prefix(repo: &str) -> String {
    format!("repo:{repo}\0tag:")
}

fn tag_key(repo: &str, tag: &str) -> String {
    format!("repo:{repo}\0tag:{tag}")
}

fn manifest_key(repo: &str, digest: &str) -> String {
    format!("repo:{repo}\0manifest:{digest}")
}

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

async fn sha256_file(path: &StdPath) -> Result<String> {
    let mut file = fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}
