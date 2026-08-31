//! b4 — embedded HTTPS server that shares the libraries on the LAN.
//! Full design: docs/NETWORK_SHARING.md.

mod auth;
mod cert;
mod guard;
mod ids;
mod routes;
mod tls;
mod tray;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use rand::RngCore;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::db::AppDb;

pub use tray::TrayState;

/// Managed Tauri state: the running server, if any.
#[derive(Default)]
pub struct ShareState(pub Mutex<Option<Running>>);

pub struct Running {
    handle: axum_server::Handle,
    port: u16,
    fingerprint: String,
}

#[derive(Serialize, Clone)]
pub struct ShareConfig {
    pub enabled: bool,
    pub port: u16,
    pub name: String,
    pub pin_set: bool,
    pub allowlist: String,
    pub audit: bool,
    /// Max simultaneous downloads (0 = unlimited).
    pub max_conn: u32,
    /// Per-download bandwidth ceiling in KB/s (0 = unlimited).
    pub rate_kbps: u32,
}

/// QR code for a URL as an inline SVG string.
pub fn qr_svg(url: &str) -> Result<String, String> {
    use qrcode::{render::svg, QrCode};
    let code = QrCode::new(url.as_bytes()).map_err(|e| e.to_string())?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(180, 180)
        .quiet_zone(true)
        .dark_color(svg::Color("#111111"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

#[derive(Serialize, Clone)]
pub struct ShareStatus {
    pub running: bool,
    pub port: u16,
    pub urls: Vec<String>,
    pub fingerprint: String,
    pub pin_set: bool,
}

// ---------- settings helpers ----------

fn get(app: &AppHandle, key: &str) -> Option<String> {
    let db = app.state::<AppDb>();
    let conn = db.0.lock().ok()?;
    crate::db::get_setting(&conn, key).ok().flatten()
}

fn put(app: &AppHandle, key: &str, val: &str) -> Result<(), String> {
    let db = app.state::<AppDb>();
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    crate::db::set_setting(&conn, key, val)
}

fn default_name() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Readaity".to_string())
}

pub fn load_config(app: &AppHandle) -> ShareConfig {
    ShareConfig {
        enabled: get(app, "share_enabled").as_deref() == Some("true"),
        port: get(app, "share_port")
            .and_then(|s| s.parse().ok())
            .unwrap_or(8787),
        name: get(app, "share_name").unwrap_or_else(default_name),
        pin_set: get(app, "share_pin_hash").is_some(),
        allowlist: get(app, "share_allowlist").unwrap_or_default(),
        audit: get(app, "share_audit").as_deref() != Some("false"),
        max_conn: get(app, "share_max_conn").and_then(|s| s.parse().ok()).unwrap_or(0),
        rate_kbps: get(app, "share_rate_kbps").and_then(|s| s.parse().ok()).unwrap_or(0),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn save_config(
    app: &AppHandle,
    port: u16,
    name: &str,
    allowlist: &str,
    audit: bool,
    max_conn: u32,
    rate_kbps: u32,
) -> Result<(), String> {
    if !(1024..=65535).contains(&port) {
        return Err("Port must be between 1024 and 65535.".into());
    }
    put(app, "share_port", &port.to_string())?;
    put(app, "share_name", name.trim())?;
    put(app, "share_allowlist", allowlist.trim())?;
    put(app, "share_audit", if audit { "true" } else { "false" })?;
    put(app, "share_max_conn", &max_conn.to_string())?;
    put(app, "share_rate_kbps", &rate_kbps.to_string())?;
    Ok(())
}

pub fn set_pin(app: &AppHandle, pin: &str) -> Result<(), String> {
    let len = pin.chars().count();
    if !(6..=10).contains(&len) || !pin.chars().all(|c| c.is_ascii_digit()) {
        return Err("PIN must be 6 to 10 digits.".into());
    }
    let hash = auth::hash_pin(pin)?;
    put(app, "share_pin_hash", &hash)
}

pub fn generate_pin(app: &AppHandle) -> Result<String, String> {
    let mut buf = [0u8; 6];
    rand::thread_rng().fill_bytes(&mut buf);
    let pin: String = buf.iter().map(|b| char::from(b'0' + (b % 10))).collect();
    set_pin(app, &pin)?;
    Ok(pin)
}

// ---------- certificate ----------

fn ensure_cert(app: &AppHandle) -> Result<(Vec<u8>, Vec<u8>, String), String> {
    if let (Some(c), Some(k)) = (get(app, "share_cert_pem"), get(app, "share_key_pem")) {
        let fp = cert::fingerprint_from_pem(&c)?;
        return Ok((c.into_bytes(), k.into_bytes(), fp));
    }
    let (c, k) = cert::generate()?;
    put(app, "share_cert_pem", &c)?;
    put(app, "share_key_pem", &k)?;
    let fp = cert::fingerprint_from_pem(&c)?;
    Ok((c.into_bytes(), k.into_bytes(), fp))
}

pub fn regenerate_cert(app: &AppHandle) -> Result<String, String> {
    let (c, k) = cert::generate()?;
    put(app, "share_cert_pem", &c)?;
    put(app, "share_key_pem", &k)?;
    cert::fingerprint_from_pem(&c)
}

// ---------- LAN addresses ----------

/// Rank a LAN address by how likely it is the one a person wants to hand out:
/// a home/office `192.168.x` first, then `10.x`, then `172.16–31.x`, then any
/// other private range, with APIPA `169.254.x` link-local last.
fn ip_rank(ip: &IpAddr) -> u8 {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            match (o[0], o[1]) {
                (192, 168) => 0,
                (10, _) => 1,
                (172, b) if (16..=31).contains(&b) => 2,
                (169, 254) => 4,
                _ => 3,
            }
        }
        IpAddr::V6(_) => 5,
    }
}

pub(crate) fn lan_ips() -> Vec<IpAddr> {
    let mut v = Vec::new();
    if let Ok(list) = local_ip_address::list_afinet_netifas() {
        for (_name, ip) in list {
            if ip.is_ipv4() && !ip.is_loopback() && guard::is_private(ip) {
                v.push(ip);
            }
        }
    }
    v.sort_by_key(ip_rank);
    v.dedup();
    if v.is_empty() {
        v.push(IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
    v
}

// ---------- lifecycle ----------

pub fn start(app: &AppHandle) -> Result<ShareStatus, String> {
    {
        let state = app.state::<ShareState>();
        let guard = state.0.lock().map_err(|e| e.to_string())?;
        if guard.is_some() {
            drop(guard);
            return Ok(status(app));
        }
    }

    let cfg = load_config(app);
    let Some(pin_hash) = get(app, "share_pin_hash") else {
        return Err("Set a PIN before starting sharing.".into());
    };

    let (cert_pem, key_pem, fingerprint) = ensure_cert(app)?;
    let tls_config = tls::server_config(&cert_pem, &key_pem)?;

    let mut session_key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut session_key);

    // 0 = unlimited: model as a very large permit pool.
    let permits = if cfg.max_conn == 0 { 1024 } else { cfg.max_conn as usize };
    let ctx = routes::Ctx {
        app: app.clone(),
        session_key,
        pin_hash,
        allowlist: Arc::new(guard::parse_allowlist(&cfg.allowlist)),
        audit: cfg.audit,
        name: cfg.name.clone(),
        cert_pem: String::from_utf8_lossy(&cert_pem).into_owned(),
        fingerprint: fingerprint.clone(),
        guards: Arc::new(Mutex::new(guard::Guards::default())),
        downloads: Arc::new(tokio::sync::Semaphore::new(permits)),
        rate_kbps: cfg.rate_kbps,
    };
    let router = routes::router(ctx);

    let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, cfg.port));
    let handle = axum_server::Handle::new();
    let serve_handle = handle.clone();
    let acceptor = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(tls_config));

    tauri::async_runtime::spawn(async move {
        let result = axum_server::bind_rustls(addr, acceptor)
            .handle(serve_handle)
            .serve(router.into_make_service_with_connect_info::<SocketAddr>())
            .await;
        if let Err(e) = result {
            eprintln!("[share] server stopped: {e}");
        }
    });

    {
        let state = app.state::<ShareState>();
        let mut guard = state.0.lock().map_err(|e| e.to_string())?;
        *guard = Some(Running {
            handle,
            port: cfg.port,
            fingerprint,
        });
    }
    put(app, "share_enabled", "true")?;
    tray::refresh(app);
    Ok(status(app))
}

pub fn stop(app: &AppHandle) -> Result<(), String> {
    let running = {
        let state = app.state::<ShareState>();
        let mut guard = state.0.lock().map_err(|e| e.to_string())?;
        guard.take()
    };
    if let Some(r) = running {
        r.handle
            .graceful_shutdown(Some(std::time::Duration::from_secs(2)));
    }
    put(app, "share_enabled", "false")?;
    tray::refresh(app);
    Ok(())
}

pub fn status(app: &AppHandle) -> ShareStatus {
    let cfg = load_config(app);
    let state = app.state::<ShareState>();
    let guard = state.0.lock().ok();
    let running = guard.as_ref().and_then(|g| g.as_ref());

    let port = running.map(|r| r.port).unwrap_or(cfg.port);
    let fingerprint = running
        .map(|r| r.fingerprint.clone())
        .or_else(|| get(app, "share_cert_pem").and_then(|p| cert::fingerprint_from_pem(&p).ok()))
        .unwrap_or_default();
    let urls = if running.is_some() {
        lan_ips()
            .into_iter()
            .map(|ip| format!("https://{ip}:{port}"))
            .collect()
    } else {
        Vec::new()
    };

    ShareStatus {
        running: running.is_some(),
        port,
        urls,
        fingerprint,
        pin_set: cfg.pin_set,
    }
}

#[cfg(test)]
mod tests {
    use super::ip_rank;
    use std::net::IpAddr;

    #[test]
    fn lan_addresses_rank_192_then_10_then_link_local() {
        let mut ips: Vec<IpAddr> = ["169.254.5.5", "10.0.0.3", "192.168.1.10", "172.20.1.1"]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        ips.sort_by_key(ip_rank);
        let ordered: Vec<String> = ips.iter().map(|i| i.to_string()).collect();
        assert_eq!(
            ordered,
            ["192.168.1.10", "10.0.0.3", "172.20.1.1", "169.254.5.5"]
        );
    }
}

/// Start on launch only if the user previously enabled it and a PIN is set.
pub fn autostart(app: &AppHandle) {
    let cfg = load_config(app);
    if cfg.enabled && cfg.pin_set {
        if let Err(e) = start(app) {
            eprintln!("[share] autostart failed: {e}");
        }
    }
}
