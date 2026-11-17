//! The Files page (docs/BACKEND_SPEC.md 7.10): browse, edit, upload, download, delete.
//!
//! Every operation is a `priv` operation. Admins can reach the whole machine (except the
//! denied system areas); deployers are confined to the apps directory by a *jail* that
//! `priv` enforces after resolving symbolic links; viewers have no file access.

use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use rustix::fs::{AtFlags, Mode, OFlags, fchmod, open, openat, unlinkat};

use axum::Json;
use axum::body::Body;
use axum::extract::{Extension, Query, State};
use axum::http::{HeaderValue, header};
use axum::response::Response;
use futures_util::StreamExt;
use opsbox_auth::Principal;
use opsbox_proto::{ErrorCode, FileKind, Role};
use opsbox_store::{ActorKind, AuditResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::json::ApiJson;
use crate::session::{Client, Deployer};
use crate::{AppState, HttpError, RequestId};

/// Where a deployer's files live. Everything outside is invisible to them.
pub const APPS_DIR: &str = "/var/lib/opsbox/apps";

/// Settings the file endpoints need from the configuration.
#[derive(Debug, Clone)]
pub struct FilesSettings {
    /// The only place deployers can see.
    pub apps_dir: PathBuf,
    /// Staging area for uploads and downloads; created here, read by `priv`.
    pub xfer_dir: PathBuf,
    /// Largest accepted upload, in bytes.
    pub max_upload: u64,
}

impl Default for FilesSettings {
    fn default() -> Self {
        Self {
            apps_dir: PathBuf::from(APPS_DIR),
            xfer_dir: PathBuf::from(opsbox_priv::DEFAULT_XFER_DIR),
            max_upload: 2 << 30,
        }
    }
}

/// The folder a request is confined to, from the caller's role.
pub fn jail_for(who: &Principal, files: &FilesSettings) -> Option<String> {
    (who.user.role != Role::Admin).then(|| files.apps_dir.to_string_lossy().into_owned())
}

fn invalid(field: &'static str, msg: &str) -> HttpError {
    HttpError(
        opsbox_proto::ApiError::new(ErrorCode::Validation, msg).with_field(field),
        None,
    )
}

fn audit(
    state: &AppState,
    who: &Principal,
    ip: &str,
    action: &str,
    target: Option<&str>,
    detail: Value,
) {
    state.auth.audit(
        &who.user.name,
        ActorKind::User,
        Some(ip),
        action,
        target,
        AuditResult::Ok,
        detail,
    );
}

#[derive(Deserialize)]
pub(crate) struct PathQuery {
    path: Option<String>,
}

fn need_path(q: &PathQuery) -> Result<&str, HttpError> {
    q.path
        .as_deref()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| invalid("path", "Choose a file or folder."))
}

fn type_char(k: FileKind) -> &'static str {
    match k {
        FileKind::Dir => "d",
        FileKind::File | FileKind::BrokenLink => "f",
    }
}

pub(crate) async fn list(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    Extension(RequestId(rid)): Extension<RequestId>,
    Query(q): Query<PathQuery>,
) -> Result<Json<Value>, HttpError> {
    let l = state
        .priv_client
        .file_list(
            &rid,
            need_path(&q)?,
            jail_for(&who, &state.files).as_deref(),
        )
        .await?;
    Ok(Json(json!({
        "path": l.path,
        "entries": l.entries.iter().map(|e| json!({
            "name": e.name, "path": e.path, "type": type_char(e.kind), "mode": e.mode,
            "mtime": e.mtime, "size": e.size, "link": e.is_link, "brokenLink": e.kind == FileKind::BrokenLink, "warn": e.warn,
        })).collect::<Vec<_>>(),
    })))
}

pub(crate) async fn read(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    Extension(RequestId(rid)): Extension<RequestId>,
    Query(q): Query<PathQuery>,
) -> Result<Json<Value>, HttpError> {
    let c = state
        .priv_client
        .file_read(
            &rid,
            need_path(&q)?,
            jail_for(&who, &state.files).as_deref(),
        )
        .await?;
    Ok(Json(
        json!({ "path": c.path, "content": c.content, "mode": c.mode, "size": c.size, "mtime": c.mtime }),
    ))
}

#[derive(Deserialize)]
pub(crate) struct WriteBody {
    path: String,
    content: String,
    /// The modification time the editor saw; a different one on disk means someone else saved.
    mtime: Option<i64>,
}

pub(crate) async fn write(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(b): ApiJson<WriteBody>,
) -> Result<Json<Value>, HttpError> {
    let mtime = state
        .priv_client
        .file_write(
            &rid,
            &who.user.name,
            &b.path,
            &b.content,
            b.mtime,
            jail_for(&who, &state.files).as_deref(),
        )
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        "files.write",
        Some(&b.path),
        json!({ "bytes": b.content.len() }),
    );
    Ok(Json(json!({ "mtime": mtime })))
}

#[derive(Deserialize)]
pub(crate) struct ChmodBody {
    path: String,
    mode: String,
}

pub(crate) async fn chmod(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(b): ApiJson<ChmodBody>,
) -> Result<Json<Value>, HttpError> {
    state
        .priv_client
        .file_chmod(
            &rid,
            &who.user.name,
            &b.path,
            b.mode.trim(),
            jail_for(&who, &state.files).as_deref(),
        )
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        "files.chmod",
        Some(&b.path),
        json!({ "mode": b.mode.trim() }),
    );
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub(crate) struct PathBody {
    path: String,
}

pub(crate) async fn mkdir(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(b): ApiJson<PathBody>,
) -> Result<Json<Value>, HttpError> {
    state
        .priv_client
        .file_mkdir(
            &rid,
            &who.user.name,
            &b.path,
            jail_for(&who, &state.files).as_deref(),
        )
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        "files.mkdir",
        Some(&b.path),
        json!({}),
    );
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub(crate) struct DeleteBody {
    paths: Vec<String>,
    #[serde(default)]
    permanent: bool,
}

pub(crate) async fn delete(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(b): ApiJson<DeleteBody>,
) -> Result<Json<Value>, HttpError> {
    let n = state
        .priv_client
        .file_remove(
            &rid,
            &who.user.name,
            b.paths.clone(),
            b.permanent,
            jail_for(&who, &state.files).as_deref(),
        )
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        if b.permanent {
            "files.delete"
        } else {
            "files.trash"
        },
        Some(&b.paths.join(", ")),
        json!({ "count": n }),
    );
    Ok(Json(json!({ "removed": n, "permanent": b.permanent })))
}

// ------------------------------------------------------------------ transfers

#[derive(Deserialize)]
pub(crate) struct UploadQuery {
    dir: String,
    name: String,
    #[serde(default)]
    overwrite: bool,
}

/// Make sure the staging folder exists and only the opsbox user can use it, returning its fd.
async fn ensure_xfer(dir: &Path) -> Result<OwnedFd, HttpError> {
    tokio::fs::create_dir_all(dir).await.map_err(|e| {
        tracing::error!(error = %e, dir = %dir.display(), "cannot create the transfer folder");
        HttpError::internal()
    })?;
    let xfer_fd = open(
        dir,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| {
        tracing::error!(error = %e, dir = %dir.display(), "cannot open the transfer folder");
        HttpError::internal()
    })?;
    let _ = fchmod(&xfer_fd, Mode::from_raw_mode(0o700));
    Ok(xfer_fd)
}

/// `POST /api/v1/files/upload?dir=&name=[&overwrite=true]` with the file as the request body.
/// The body is streamed to the staging folder (never held in memory) and `priv` moves it into place.
pub(crate) async fn upload(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Query(q): Query<UploadQuery>,
    headers: axum::http::HeaderMap,
    body: Body,
) -> Result<Json<Value>, HttpError> {
    let max = state.files.max_upload;
    let too_big = || {
        HttpError::new(
            ErrorCode::Validation,
            format!(
                "That file is larger than the upload limit ({} MB).",
                max >> 20
            ),
        )
    };
    if headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|n| n > max)
    {
        return Err(too_big());
    }
    let xfer_fd = ensure_xfer(&state.files.xfer_dir).await?;
    let part_name = format!("{}.part", ulid::Ulid::generate());
    let staging = state.files.xfer_dir.join(&part_name);
    let stream_to_disk = async {
        let fd = openat(
            &xfer_fd,
            &part_name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|_| HttpError::internal())?;
        let mut file = tokio::fs::File::from_std(std::fs::File::from(fd));
        let mut total = 0u64;
        let mut stream = body.into_data_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| {
                HttpError::new(ErrorCode::Validation, "The upload was interrupted.")
            })?;
            total += chunk.len() as u64;
            if total > max {
                return Err(too_big());
            }
            file.write_all(&chunk).await.map_err(|_| {
                HttpError::new(
                    ErrorCode::Internal,
                    "The server ran out of space for the upload.",
                )
            })?;
        }
        file.sync_all().await.map_err(|_| HttpError::internal())?;
        Ok(total)
    };
    let total = match stream_to_disk.await {
        Ok(t) => t,
        Err(e) => {
            let _ = unlinkat(&xfer_fd, &part_name, AtFlags::empty());
            return Err(e);
        }
    };
    let installed = state
        .priv_client
        .clone()
        .with_timeout(std::time::Duration::from_secs(30 * 60))
        .file_install_upload(
            &rid,
            &who.user.name,
            opsbox_priv::InstallUpload {
                staging: &staging.to_string_lossy(),
                dir: &q.dir,
                name: &q.name,
                overwrite: q.overwrite,
            },
            jail_for(&who, &state.files).as_deref(),
        )
        .await;
    // Whether or not it was installed, the staged copy is no longer needed (a refused one is
    // sent again by the page if the person chooses to overwrite).
    let _ = unlinkat(&xfer_fd, &part_name, AtFlags::empty());
    installed?;
    let target = format!("{}/{}", q.dir.trim_end_matches('/'), q.name);
    audit(
        &state,
        &who,
        &client.ip,
        "files.upload",
        Some(&target),
        json!({ "bytes": total }),
    );
    Ok(Json(json!({ "ok": true, "bytes": total, "path": target })))
}

/// A file name for `Content-Disposition`: an ASCII fallback plus the real name, percent-encoded.
pub fn content_disposition(name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && !matches!(c, '"' | '\\' | '%' | ';') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

/// `GET /api/v1/files/download?path=`
pub(crate) async fn download(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Query(q): Query<PathQuery>,
) -> Result<Response, HttpError> {
    let path = need_path(&q)?.to_owned();
    let xfer_fd = ensure_xfer(&state.files.xfer_dir).await?;
    let staged = state
        .priv_client
        .clone()
        .with_timeout(std::time::Duration::from_secs(30 * 60))
        .file_stage_download(
            &rid,
            &who.user.name,
            &path,
            jail_for(&who, &state.files).as_deref(),
        )
        .await?;
    let staged_name = Path::new(&staged.staging)
        .file_name()
        .ok_or_else(HttpError::internal)?;
    let fd = openat(
        &xfer_fd,
        staged_name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| HttpError::internal())?;
    // The open file keeps the data alive; unlinking it now means nothing is left behind
    // whether the download finishes, is cancelled or the connection drops.
    let _ = unlinkat(&xfer_fd, staged_name, AtFlags::empty());
    let file = tokio::fs::File::from_std(std::fs::File::from(fd));
    audit(
        &state,
        &who,
        &client.ip,
        "files.download",
        Some(&path),
        json!({ "bytes": staged.size }),
    );
    let mut resp = Response::new(Body::from_stream(
        tokio_util::io::ReaderStream::with_capacity(file, 64 * 1024),
    ));
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(staged.size));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    if let Ok(v) = HeaderValue::from_str(&content_disposition(&staged.name)) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_names_are_safe_in_headers() {
        assert_eq!(
            content_disposition("report.pdf"),
            "attachment; filename=\"report.pdf\"; filename*=UTF-8''report.pdf"
        );
        let weird = content_disposition("a\"b\r\nX-Evil: 1;%.txt");
        assert!(!weird.contains('\n') && !weird.contains('\r'), "{weird}");
        assert!(
            weird.starts_with("attachment; filename=\"a_b__X-Evil:_1__.txt\""),
            "{weird}"
        );
        assert!(
            content_disposition("héllo wörld.txt")
                .contains("filename*=UTF-8''h%C3%A9llo%20w%C3%B6rld.txt")
        );
        assert!(
            HeaderValue::from_str(&weird).is_ok(),
            "always a valid header value"
        );
    }
}
