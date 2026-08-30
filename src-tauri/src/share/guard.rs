//! Client-IP gating: private-range check, allowlist, per-IP lockout + rate limit,
//! global auth throttle. All in-memory — reset when the server restarts.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

const LOCK_AFTER: u32 = 5;
const LOCK_FOR: Duration = Duration::from_secs(15 * 60);
const FAIL_WINDOW: Duration = Duration::from_secs(15 * 60);
const RATE_WINDOW: Duration = Duration::from_secs(10);
const RATE_MAX: u32 = 120;
const AUTH_MIN_GAP: Duration = Duration::from_millis(200);

#[derive(Default)]
pub struct Guards {
    fails: HashMap<IpAddr, (u32, Instant)>,
    locked: HashMap<IpAddr, Instant>,
    hits: HashMap<IpAddr, (u32, Instant)>,
    last_auth: Option<Instant>,
}

impl Guards {
    pub fn is_locked(&mut self, ip: IpAddr) -> bool {
        if let Some(&until) = self.locked.get(&ip) {
            if Instant::now() < until {
                return true;
            }
            self.locked.remove(&ip);
        }
        false
    }

    /// Record a failed PIN attempt. Returns `true` if this attempt tripped the lockout.
    pub fn record_fail(&mut self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let e = self.fails.entry(ip).or_insert((0, now));
        if now.duration_since(e.1) > FAIL_WINDOW {
            *e = (0, now);
        }
        e.0 += 1;
        if e.0 >= LOCK_AFTER {
            self.locked.insert(ip, now + LOCK_FOR);
            self.fails.remove(&ip);
            true
        } else {
            false
        }
    }

    pub fn record_success(&mut self, ip: IpAddr) {
        self.fails.remove(&ip);
    }

    /// Per-IP request rate limit (all routes).
    pub fn rate_ok(&mut self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let e = self.hits.entry(ip).or_insert((0, now));
        if now.duration_since(e.1) > RATE_WINDOW {
            *e = (0, now);
        }
        e.0 += 1;
        e.0 <= RATE_MAX
    }

    /// Global throttle across all IPs for `/api/auth`.
    pub fn auth_throttle_ok(&mut self) -> bool {
        let now = Instant::now();
        if let Some(t) = self.last_auth {
            if now.duration_since(t) < AUTH_MIN_GAP {
                return false;
            }
        }
        self.last_auth = Some(now);
        true
    }
}

pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // unique local
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // link local
        }
    }
}

#[derive(Clone)]
pub enum Rule {
    Ip(IpAddr),
    /// IPv4 network + prefix length.
    V4Cidr(u32, u32),
}

pub fn parse_allowlist(s: &str) -> Vec<Rule> {
    let mut out = Vec::new();
    for tok in s.split([',', ' ', '\n', '\r', '\t']) {
        let tok = tok.trim();
        if tok.is_empty() {
            continue;
        }
        if let Some((net, bits)) = tok.split_once('/') {
            if let (Ok(ip), Ok(bits)) = (net.parse::<std::net::Ipv4Addr>(), bits.parse::<u32>()) {
                if bits <= 32 {
                    out.push(Rule::V4Cidr(u32::from(ip), bits));
                    continue;
                }
            }
        }
        if let Ok(ip) = tok.parse::<IpAddr>() {
            out.push(Rule::Ip(ip));
        }
    }
    out
}

pub fn allowed(rules: &[Rule], ip: IpAddr) -> bool {
    if rules.is_empty() {
        return true;
    }
    rules.iter().any(|r| match r {
        Rule::Ip(a) => *a == ip,
        Rule::V4Cidr(net, bits) => match ip {
            IpAddr::V4(v4) => {
                let mask = if *bits == 0 { 0 } else { u32::MAX << (32 - bits) };
                (u32::from(v4) & mask) == (net & mask)
            }
            IpAddr::V6(_) => false,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn private_ranges() {
        for p in ["192.168.1.1", "10.0.0.1", "172.16.5.9", "127.0.0.1", "169.254.1.1"] {
            assert!(is_private(ip(p)), "{p}");
        }
        for p in ["8.8.8.8", "1.1.1.1", "203.0.113.7"] {
            assert!(!is_private(ip(p)), "{p}");
        }
    }

    #[test]
    fn allowlist_matches_ip_and_cidr() {
        let rules = parse_allowlist("192.168.1.50, 10.0.0.0/8");
        assert!(allowed(&rules, ip("192.168.1.50")));
        assert!(!allowed(&rules, ip("192.168.1.51")));
        assert!(allowed(&rules, ip("10.9.9.9")));
        assert!(!allowed(&rules, ip("11.0.0.1")));
        // empty allowlist = allow all
        assert!(allowed(&[], ip("8.8.8.8")));
    }

    #[test]
    fn lockout_after_five_fails() {
        let mut g = Guards::default();
        let a = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 9));
        for _ in 0..4 {
            assert!(!g.record_fail(a));
            assert!(!g.is_locked(a));
        }
        assert!(g.record_fail(a)); // 5th trips it
        assert!(g.is_locked(a));
        // a different IP is unaffected
        assert!(!g.is_locked(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10))));
    }

    #[test]
    fn success_clears_fail_count() {
        let mut g = Guards::default();
        let a = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        g.record_fail(a);
        g.record_fail(a);
        g.record_success(a);
        for _ in 0..4 {
            assert!(!g.record_fail(a));
        }
    }
}
