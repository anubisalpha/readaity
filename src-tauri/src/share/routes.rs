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
    let file = match tokio::fs::File::open(&book.path).await {
        Ok(f) => f,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);
    let fname = format!("{}.{}", sanitise(&book.title), book.format);
    audit(&ctx, ip, "download", Some(&book.title));
    (
        [
            (
                header::CONTENT_TYPE,
                "application/octet-stream".to_string(),
            ),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{fname}\""),
            ),
            (header::CONTENT_LENGTH, book.size.to_string()),
        ],
        body,
    )
        .into_response()
}

async fn fallback() -> Response {
    StatusCode::NOT_FOUND.into_response()
}
