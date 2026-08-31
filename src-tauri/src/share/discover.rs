//! mDNS / DNS-SD: advertise this machine's share server and browse for peers.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;

const SERVICE: &str = "_readaity._tcp.local.";

/// A live mDNS registration — unregisters itself when dropped.
pub struct Advert {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Drop for Advert {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

fn host_label(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "readaity".to_string()
    } else {
        trimmed.chars().take(40).collect()
    }
}

/// Advertise `_readaity._tcp` for the running share server. Returns `None` if
/// mDNS can't start (firewalled, no multicast) — sharing still works by URL.
pub fn advertise(name: &str, port: u16, version: &str) -> Option<Advert> {
    let daemon = ServiceDaemon::new().ok()?;
    let host = format!("{}.local.", host_label(name));
    let props: [(&str, &str); 2] = [("v", version), ("pin", "required")];
    let info = ServiceInfo::new(SERVICE, name, &host, "", port, &props[..])
        .ok()?
        .enable_addr_auto();
    let fullname = info.get_fullname().to_string();
    daemon.register(info).ok()?;
    Some(Advert { daemon, fullname })
}

/// One discovered peer running Readaity Share.
#[derive(Serialize, Clone)]
pub struct Peer {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub addr: String,
    pub version: String,
}

/// Browse the LAN for `secs` seconds and return the peers found (deduped by
/// address:port). Blocking — call from a background task.
pub fn browse(secs: u64) -> Vec<Peer> {
    let Ok(daemon) = ServiceDaemon::new() else {
        return Vec::new();
    };
    let Ok(rx) = daemon.browse(SERVICE) else {
        let _ = daemon.shutdown();
        return Vec::new();
    };

    let deadline = Instant::now() + Duration::from_secs(secs.clamp(1, 15));
    let mut peers: HashMap<String, Peer> = HashMap::new();

    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        match rx.recv_timeout(left) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let addr = info
                    .get_addresses()
                    .iter()
                    .find(|a| a.is_ipv4())
                    .or_else(|| info.get_addresses().iter().next())
                    .map(|a| a.to_string())
                    .unwrap_or_default();
                if addr.is_empty() {
                    continue;
                }
                let port = info.get_port();
                let name = info
                    .get_fullname()
                    .split('.')
                    .next()
                    .unwrap_or("Readaity")
                    .replace('\\', "");
                let version = info
                    .get_property_val_str("v")
                    .unwrap_or("?")
                    .to_string();
                peers.insert(
                    format!("{addr}:{port}"),
                    Peer {
                        name,
                        host: info.get_hostname().trim_end_matches('.').to_string(),
                        port,
                        addr,
                        version,
                    },
                );
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }

    let _ = daemon.shutdown();
    let mut out: Vec<Peer> = peers.into_values().collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}
