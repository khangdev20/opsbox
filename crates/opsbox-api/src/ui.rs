//! The web UI, embedded in the binary (docs/BACKEND_SPEC.md section 7.19).
//!
//! Release builds compile `web/` into the executable. Debug builds read it from disk
//! on every request (rust-embed's default), so editing `web/` needs no rebuild.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

use crate::HttpError;

#[derive(RustEmbed)]
#[folder = "../../web/"]
// Dev tooling that lives next to the UI but is not part of it.
#[exclude = "*.py"]
#[exclude = ".DS_Store"]
struct Assets;

/// Long-lived, for files that change only when the UI is rebuilt against new
/// vendored versions. Updating a vendored file in place means users keep the old one
/// for up to a year, so rename it or version its directory when upgrading.
const CACHE_LONG: &str = "public, max-age=31536000, immutable";
/// Revalidate every time (cheap: the ETag usually answers 304).
const CACHE_REVALIDATE: &str = "no-cache";

/// Sanity check for `opsbox selftest`: the entry points of the UI are present.
/// Returns the number of embedded files.
pub(crate) fn check_embedded() -> Result<usize, String> {
    for required in ["index.html", "js/main.js", "js/api.js", "css/app.css"] {
        if Assets::get(required).is_none() {
            return Err(format!("{required} is missing from the embedded UI"));
        }
    }
    Ok(Assets::iter().count())
}

/// Serves the UI for everything the API routes did not match.
pub(crate) async fn fallback(req: Request) -> Response {
    let path = req.uri().path();

    // Unknown API paths are API errors, not the UI.
    if path == "/api" || path.starts_with("/api/") {
        return HttpError::not_found().into_response();
    }
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return crate::error::method_not_allowed().await.into_response();
    }

    match lookup(path) {
        Some(asset_path) => match Assets::get(&asset_path) {
            Some(file) => respond(
                &asset_path,
                file,
                req.headers(),
                req.method() == Method::HEAD,
            ),
            None => HttpError::not_found().into_response(),
        },
        None => HttpError::not_found().into_response(),
    }
}

/// Map a request path to an embedded file path, or `None` if it is not acceptable.
fn lookup(request_path: &str) -> Option<String> {
    // Exactly one leading slash: `//x` has an empty segment and is refused below.
    let trimmed = request_path.strip_prefix('/')?;
    let candidate = if trimmed.is_empty() || trimmed.ends_with('/') {
        format!("{trimmed}index.html")
    } else {
        trimmed.to_owned()
    };
    // Never let a request name anything outside the asset root.
    let suspicious = candidate
        .split('/')
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
        || candidate.contains(['\\', '\0']);
    (!suspicious).then_some(candidate)
}

fn cache_control(path: &str) -> &'static str {
    if path.starts_with("fonts/") || path.starts_with("vendor/") {
        CACHE_LONG
    } else {
        CACHE_REVALIDATE
    }
}

fn content_type(mime: &str) -> String {
    let textual = mime.starts_with("text/")
        || matches!(
            mime,
            "application/javascript" | "application/json" | "image/svg+xml"
        );
    if textual {
        format!("{mime}; charset=utf-8")
    } else {
        mime.to_owned()
    }
}

fn etag(hash: [u8; 32]) -> String {
    let mut tag = String::with_capacity(34);
    tag.push('"');
    for byte in &hash[..16] {
        tag.push_str(&format!("{byte:02x}"));
    }
    tag.push('"');
    tag
}

fn matches_etag(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .map(|t| t.trim().trim_start_matches("W/"))
                .any(|t| t == etag || t == "*")
        })
}

fn respond(
    path: &str,
    file: rust_embed::EmbeddedFile,
    req_headers: &HeaderMap,
    head: bool,
) -> Response {
    let etag = etag(file.metadata.sha256_hash());
    let mut builder = Response::builder()
        .header(header::ETAG, &etag)
        .header(header::CACHE_CONTROL, cache_control(path))
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff");

    if matches_etag(req_headers, &etag) {
        return builder
            .status(StatusCode::NOT_MODIFIED)
            .body(Body::empty())
            .expect("static response parts are valid");
    }

    let len = file.data.len();
    builder = builder
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_str(&content_type(file.metadata.mimetype()))
                .unwrap_or(HeaderValue::from_static("application/octet-stream")),
        )
        .header(header::CONTENT_LENGTH, len);
    let body = if head {
        Body::empty()
    } else {
        Body::from(file.data.into_owned())
    };
    builder.body(body).expect("static response parts are valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_map_to_assets() {
        assert_eq!(lookup("/").as_deref(), Some("index.html"));
        assert_eq!(lookup("/js/main.js").as_deref(), Some("js/main.js"));
        assert_eq!(lookup("/js/").as_deref(), Some("js/index.html"));
    }

    #[test]
    fn traversal_and_oddities_are_refused() {
        for bad in [
            "/../Cargo.toml",
            "/js/../../x",
            "//etc/passwd",
            "/a//b",
            "/a\\b",
            "/./x",
            "no-leading-slash",
        ] {
            assert!(lookup(bad).is_none(), "{bad}");
        }
        // Percent-escapes are not decoded, so an encoded dot-dot is just an unknown name.
        assert_eq!(lookup("/%2e%2e/x").as_deref(), Some("%2e%2e/x"));
    }

    #[test]
    fn cache_policy_follows_the_spec() {
        assert_eq!(cache_control("fonts/Geist.woff2"), CACHE_LONG);
        assert_eq!(cache_control("vendor/monaco/vs/loader.js"), CACHE_LONG);
        assert_eq!(cache_control("index.html"), CACHE_REVALIDATE);
        assert_eq!(cache_control("js/api.js"), CACHE_REVALIDATE);
        assert_eq!(cache_control("css/app.css"), CACHE_REVALIDATE);
    }

    #[test]
    fn etag_matching() {
        let mut h = HeaderMap::new();
        h.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_static("\"a\", W/\"b\""),
        );
        assert!(matches_etag(&h, "\"a\""));
        assert!(matches_etag(&h, "\"b\""));
        assert!(!matches_etag(&h, "\"c\""));
    }
}
