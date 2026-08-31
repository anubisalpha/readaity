//! axum router + handlers for the share server.

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};

use axum::{
    body::Body,
    extract::{ConnectInfo, Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use tauri::{AppHandle, Manager};
use tower_http::set_header::SetResponseHeaderLayer;

use super::{auth, guard, ids};
use crate::db::{self, AppDb, ShareBook};

const INDEX_HTML: &str = include_str!("assets/index.html");
const TRUST_HELP_HTML: &str = include_str!("assets/trust-help.html");

#[derive(Clone)]
pub struct Ctx {
    pub app: AppHandle,
    pub session_key: [u8; 32],
    pub pin_hash: String,
    pub allowlist: Arc<Vec<guard::Rule>>,
    pub audit: bool,
    pub name: String,
    pub cert_pem: String,
    pub fingerprint: String,
    pub guards: Arc<Mutex<guard::Guards>>,
    /// Permits for concurrent downloads (a large pool when unlimited).
    pub downloads: Arc<tokio::sync::Semaphore>,
    /// Per-download bandwidth ceiling, KB/s (0 = unlimited).
    pub rate_kbps: u32,
}

pub fn router(ctx: Ctx) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/healthz", get(healthz))
        .route("/trust", get(trust_cert))
        .route("/trust/help", get(trust_help))
        .route("/api/auth", post(api_auth))
        .route("/api/manifest", get(api_manifest))
        .route("/api/books", get(api_books))
        .route("/api/cover/{id}", get(api_cover))
        .route("/api/download/{id}", get(api_download))
        .fallback(fallback)
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .with_state(ctx)
}

// ---------- guards ----------

fn deny(code: StatusCode, msg: &str) -> Response {
    (code, msg.to_string()).into_response()
}

fn precheck(ctx: &Ctx, ip: IpAddr) -> Result<(), Response> {
    if !guard::is_private(ip) {
        return Err(deny(StatusCode::FORBIDDEN, "Readaity Share is LAN-only."));
    }
    if !guard::allowed(&ctx.allowlist, ip) {
        return Err(deny(StatusCode::FORBIDDEN, "This device is not on the allowlist."));
    }
    let mut g = ctx.guards.lock().unwrap();
    if !g.rate_ok(ip) {
        return Err(deny(StatusCode::TOO_MANY_REQUESTS, "Too many requests — slow down."));
    }
    Ok(())
}

fn cookie(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|kv| {
        kv.trim()
            .strip_prefix("readaity_share=")
            .map(|v| v.to_string())
    })
}

fn require_auth(ctx: &Ctx, headers: &HeaderMap, ip: IpAddr) -> Result<(), Response> {
    let ok = cookie(headers)
        .map(|c| auth::check_cookie(&ctx.session_key, &ip.to_string(), &c))
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        Err(deny(StatusCode::UNAUTHORIZED, "PIN required."))
    }
}

fn audit(ctx: &Ctx, ip: IpAddr, event: &str, detail: Option<&str>) {
    if !ctx.audit {
        return;
    }
    if let Some(dbs) = ctx.app.try_state::<AppDb>() {
        if let Ok(conn) = dbs.0.lock() {
            let _ = db::add_audit(&conn, &ip.to_string(), event, detail);
        }
    }
}

fn ready_books(ctx: &Ctx, library: &str) -> Vec<ShareBook> {
    ctx.app
        .try_state::<AppDb>()
        .and_then(|dbs| {
            dbs.0
                .lock()
                .ok()
                .and_then(|conn| db::share_list(&conn, library).ok())
        })
        .unwrap_or_default()
}

fn all_ready(ctx: &Ctx) -> Vec<ShareBook> {
    let mut v = ready_books(ctx, "comics");
    v.extend(ready_books(ctx, "ebooks"));
    v
}

fn sanitise(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        "book".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------- handlers ----------

async fn index(State(ctx): State<Ctx>, ci: ConnectInfo<SocketAddr>) -> Response {
    let ip = ci.0.ip();
    if let Err(r) = precheck(&ctx, ip) {
        return r;
    }
    (
        [(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; \
             script-src 'self' 'unsafe-inline'; connect-src 'self'"
                .to_string(),
        )],
        Html(INDEX_HTML),
    )
        .into_response()
}

async fn healthz(State(ctx): State<Ctx>, ci: ConnectInfo<SocketAddr>) -> Response {
    let ip = ci.0.ip();
    if let Err(r) = precheck(&ctx, ip) {
        return r;
    }
    Json(serde_json::json!({
        "app": "readaity",
        "version": env!("CARGO_PKG_VERSION"),
        "fingerprint": ctx.fingerprint,
    }))
    .into_response()
}

async fn trust_cert(State(ctx): State<Ctx>, ci: ConnectInfo<SocketAddr>) -> Response {
    let ip = ci.0.ip();
    if let Err(r) = precheck(&ctx, ip) {
        return r;
    }
    let fname = format!("readaity-{}.pem", sanitise(&ctx.name));
    (
        [
            (
                header::CONTENT_TYPE,
                "application/x-pem-file".to_string(),
            ),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{fname}\""),
            ),
        ],
        ctx.cert_pem.clone(),
    )
        .into_response()
}

async fn trust_help(State(ctx): State<Ctx>, ci: ConnectInfo<SocketAddr>) -> Response {
    let ip = ci.0.ip();
    if let Err(r) = precheck(&ctx, ip) {
        return r;
    }
    (
        [(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'self'; style-src 'unsafe-inline'".to_string(),
        )],
        Html(TRUST_HELP_HTML),
    )
        .into_response()
}

#[derive(Deserialize)]
struct AuthBody {
    pin: String,
}

async fn api_auth(
    State(ctx): State<Ctx>,
    ci: ConnectInfo<SocketAddr>,
    Json(body): Json<AuthBody>,
) -> Response {
    let ip = ci.0.ip();
    if let Err(r) = precheck(&ctx, ip) {
        return r;
    }
    {
        let mut g = ctx.guards.lock().unwrap();
        if g.is_locked(ip) {
            drop(g);
            audit(&ctx, ip, "auth-locked", None);
            return deny(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many wrong PINs — locked out for 15 minutes.",
            );
        }
        if !g.auth_throttle_ok() {
            return deny(StatusCode::TOO_MANY_REQUESTS, "Slow down.");
        }
    }

    if auth::verify_pin(&body.pin, &ctx.pin_hash) {
        ctx.guards.lock().unwrap().record_success(ip);
        audit(&ctx, ip, "auth-ok", None);
        let value = auth::mint_cookie(&ctx.session_key, &ip.to_string(), 12 * 3600);
        let set_cookie = format!(
            "readaity_share={value}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={}",
            12 * 3600
        );
        return (
            StatusCode::OK,
            [(header::SET_COOKIE, set_cookie)],
            Json(serde_json::json!({ "ok": true })),
        )
            .into_response();
    }

    let locked = ctx.guards.lock().unwrap().record_fail(ip);
    audit(
        &ctx,
        ip,
        if locked { "auth-fail-lockout" } else { "auth-fail" },
        None,
    );
    deny(StatusCode::UNAUTHORIZED, "Wrong PIN.")
}

async fn api_manifest(
    State(ctx): State<Ctx>,
    ci: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let ip = ci.0.ip();
    if let Err(r) = precheck(&ctx, ip) {
        return r;
    }
    if let Err(r) = require_auth(&ctx, &headers, ip) {
        return r;
    }
    Json(serde_json::json!({
        "name": ctx.name,
        "version": env!("CARGO_PKG_VERSION"),
        "libraries": {
            "comics": ready_books(&ctx, "comics").len(),
            "ebooks": ready_books(&ctx, "ebooks").len(),
        },
        "generated_at": now_unix(),
    }))
    .into_response()
}

#[derive(Deserialize)]
struct LibQ {
    library: Option<String>,
}

async fn api_books(
    State(ctx): State<Ctx>,
    ci: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<LibQ>,
) -> Response {
    let ip = ci.0.ip();
    if let Err(r) = precheck(&ctx, ip) {
        return r;
    }
    if let Err(r) = require_auth(&ctx, &headers, ip) {
        return r;
    }
    let library = match q.library.as_deref() {
        Some("comics") => "comics",
        Some("ebooks") => "ebooks",
        _ => return deny(StatusCode::BAD_REQUEST, "library must be comics or ebooks"),
    };
    let out: Vec<_> = ready_books(&ctx, library)
        .iter()
        .map(|b| {
            serde_json::json!({
                "id": ids::book_id(&ctx.session_key, &b.path),
                "title": b.title,
                "format": b.format,
                "size": b.size,
                "page_count": b.page_count,
                "md5": b.md5,
                "has_cover": b.has_cover,
            })
        })
        .collect();
    Json(out).into_response()
}

async fn api_cover(
    State(ctx): State<Ctx>,
    ci: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let ip = ci.0.ip();
    if let Err(r) = precheck(&ctx, ip) {
        return r;
    }
    if let Err(r) = require_auth(&ctx, &headers, ip) {
        return r;
    }
    let books = all_ready(&ctx);
    let Some(book) = ids::find(&ctx.session_key, &books, &id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let cover = ctx.app.try_state::<AppDb>().and_then(|dbs| {
        dbs.0
            .lock()
            .ok()
            .and_then(|conn| db::get_cover(&conn, &book.path).ok().flatten())
    });
    match cover {
        Some(bytes) => (
            [(header::CONTENT_TYPE, "image/jpeg".to_string())],
            bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Parse a single-range `Range: bytes=…` header against a known total length.
/// Returns `Some((start, end_inclusive))`, or `None` when there's no Range
/// header. `Err(())` means the range is syntactically present but unsatisfiable.
fn parse_range(headers: &HeaderMap, total: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(raw) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) else {
        return Ok(None);
    };
    let spec = raw.trim().strip_prefix("bytes=").ok_or(())?;
    if spec.contains(',') {
        return Err(()); // multi-range not supported
    }
    let (a, b) = spec.split_once('-').ok_or(())?;
    let (start, end) = match (a.trim(), b.trim()) {
        ("", "") => return Err(()),
        ("", suf) => {
            let n: u64 = suf.parse().map_err(|_| ())?;
            (total.saturating_sub(n), total.saturating_sub(1))
        }
        (s, "") => (s.parse().map_err(|_| ())?, total.saturating_sub(1)),
        (s, e) => (s.parse().map_err(|_| ())?, e.parse().map_err(|_| ())?),
    };
    if total == 0 || start > end || start >= total {
        return Err(());
    }
    Ok(Some((start, end.min(total - 1))))
}

async fn api_download(
    State(ctx): State<Ctx>,
    ci: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let ip = ci.0.ip();
    if let Err(r) = precheck(&ctx, ip) {
        return r;
    }
    if let Err(r) = require_auth(&ctx, &headers, ip) {
        return r;
    }
    let books = all_ready(&ctx);
    let Some(book) = ids::find(&ctx.session_key, &books, &id).cloned() else {
        return StatusCode::NOT_FOUND.into_response();
    };

    // Cap concurrent downloads.
    let Ok(permit) = ctx.downloads.clone().try_acquire_owned() else {
        return deny(
            StatusCode::SERVICE_UNAVAILABLE,
            "Too many downloads in progress — try again shortly.",
        );
    };

    let total = book.size.max(0) as u64;
    let (start, end, status) = match parse_range(&headers, total) {
        Ok(Some((s, e))) => (s, e, StatusCode::PARTIAL_CONTENT),
        Ok(None) => (0, total.saturating_sub(1), StatusCode::OK),
        Err(()) => {
            return (
                StatusCode::RANGE_NOT_SATISFIABLE,
                [(header::CONTENT_RANGE, format!("bytes */{total}"))],
                "Requested range not satisfiable",
            )
                .into_response();
        }
    };
    let span = if total == 0 { 0 } else { end - start + 1 };

    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let mut file = match tokio::fs::File::open(&book.path).await {
        Ok(f) => f,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    if start > 0 && file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Stream the (bounded) slice through a bandwidth-paced channel; the permit
    // rides along and is released when the transfer ends.
    let rate_bps = ctx.rate_kbps as u64 * 1024;
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(4);
    tokio::spawn(async move {
        let _permit = permit;
        let mut remaining = span;
        let mut buf = vec![0u8; 64 * 1024];
        let began = std::time::Instant::now();
        let mut sent: u64 = 0;
        while remaining > 0 {
            let want = remaining.min(buf.len() as u64) as usize;
            let n = match file.read(&mut buf[..want]).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    break;
                }
            };
            remaining -= n as u64;
            sent += n as u64;
            if tx
                .send(Ok(axum::body::Bytes::copy_from_slice(&buf[..n])))
                .await
                .is_err()
            {
                break; // client hung up
            }
            if rate_bps > 0 {
                let expected = std::time::Duration::from_secs_f64(sent as f64 / rate_bps as f64);
                let elapsed = began.elapsed();
                if expected > elapsed {
                    tokio::time::sleep(expected - elapsed).await;
                }
            }
        }
    });
    let body = Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx));

    let fname = format!("{}.{}", sanitise(&book.title), book.format);
    audit(&ctx, ip, "download", Some(&book.title));
    let mut resp = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{fname}\""))
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, span.to_string());
    if status == StatusCode::PARTIAL_CONTENT {
        resp = resp.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{total}"));
    }
    resp.body(body).unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn fallback() -> Response {
    StatusCode::NOT_FOUND.into_response()
}

#[cfg(test)]
mod tests {
    use super::parse_range;
    use axum::http::{header, HeaderMap, HeaderValue};

    fn h(v: &str) -> HeaderMap {
        let mut m = HeaderMap::new();
        m.insert(header::RANGE, HeaderValue::from_str(v).unwrap());
        m
    }

    #[test]
    fn range_parsing() {
        assert_eq!(parse_range(&HeaderMap::new(), 1000), Ok(None));
        assert_eq!(parse_range(&h("bytes=0-99"), 1000), Ok(Some((0, 99))));
        assert_eq!(parse_range(&h("bytes=500-"), 1000), Ok(Some((500, 999))));
        assert_eq!(parse_range(&h("bytes=-100"), 1000), Ok(Some((900, 999))));
        // end past EOF is clamped
        assert_eq!(parse_range(&h("bytes=900-5000"), 1000), Ok(Some((900, 999))));
        // unsatisfiable / malformed
        assert_eq!(parse_range(&h("bytes=1000-1001"), 1000), Err(()));
        assert_eq!(parse_range(&h("bytes=50-10"), 1000), Err(()));
        assert_eq!(parse_range(&h("bytes=0-10,20-30"), 1000), Err(()));
        assert_eq!(parse_range(&h("chunks=0-10"), 1000), Err(()));
    }
}
