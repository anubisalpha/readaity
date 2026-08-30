//! PIN hashing (Argon2id) and signed session cookies (HMAC-SHA256).

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;

type HmacSha = Hmac<Sha256>;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn hash_pin(pin: &str) -> Result<String, String> {
    let mut salt_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt_bytes);
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|e| e.to_string())?;
    Argon2::default()
        .hash_password(pin.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

pub fn verify_pin(pin: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(ph) => Argon2::default()
            .verify_password(pin.as_bytes(), &ph)
            .is_ok(),
        Err(_) => false,
    }
}

/// `<expiry>|<ip>|<hexmac>` — HMAC covers `<expiry>|<ip>`. A `|` separator is
/// used because both IPv4 (`.`) and IPv6 (`:`) addresses contain the obvious ones.
pub fn mint_cookie(key: &[u8; 32], ip: &str, ttl_secs: i64) -> String {
    let exp = now() + ttl_secs;
    let payload = format!("{exp}|{ip}");
    let mut mac = HmacSha::new_from_slice(key).expect("hmac key");
    mac.update(payload.as_bytes());
    format!("{payload}|{}", hex::encode(mac.finalize().into_bytes()))
}

pub fn check_cookie(key: &[u8; 32], ip: &str, cookie: &str) -> bool {
    let parts: Vec<&str> = cookie.split('|').collect();
    if parts.len() != 3 {
        return false;
    }
    let (exp, cookie_ip, tag) = (parts[0], parts[1], parts[2]);
    if cookie_ip != ip {
        return false;
    }
    match exp.parse::<i64>() {
        Ok(e) if e >= now() => {}
        _ => return false,
    }
    let Ok(tag_bytes) = hex::decode(tag) else {
        return false;
    };
    let mut mac = HmacSha::new_from_slice(key).expect("hmac key");
    mac.update(format!("{exp}|{cookie_ip}").as_bytes());
    mac.verify_slice(&tag_bytes).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_roundtrip() {
        let h = hash_pin("814052").unwrap();
        assert!(verify_pin("814052", &h));
        assert!(!verify_pin("814053", &h));
        assert!(!verify_pin("814052", "not-a-hash"));
    }

    #[test]
    fn cookie_roundtrip_and_binding() {
        let key = [7u8; 32];
        // IPv4 (dots) and IPv6 (colons) both round-trip.
        for ip in ["192.168.1.20", "127.0.0.1", "fd00::1234"] {
            let c = mint_cookie(&key, ip, 3600);
            assert!(check_cookie(&key, ip, &c), "{ip}");
            assert!(!check_cookie(&key, "10.0.0.9", &c), "wrong ip {ip}");
        }
    }

    #[test]
    fn cookie_rejects_tamper_expiry_and_wrong_key() {
        let key = [1u8; 32];
        let ip = "192.168.0.5";
        let good = mint_cookie(&key, ip, 3600);
        assert!(!check_cookie(&[2u8; 32], ip, &good));
        assert!(!check_cookie(&key, ip, &good.replace('a', "b")));
        assert!(!check_cookie(&key, ip, &mint_cookie(&key, ip, -10)));
        assert!(!check_cookie(&key, ip, "garbage"));
    }
}
