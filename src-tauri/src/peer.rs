//! Client half of LAN sharing: talk to another Readaity instance's share
//! server over HTTPS, verifying its self-signed certificate by a pinned
//! SHA-256 fingerprint (trust-on-first-use). See docs/NETWORK_SHARING.md.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// `AB:CD:…` SHA-256 of a DER cert — matches `share::cert::fingerprint_from_pem`.
fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Accepts the peer's cert iff its fingerprint matches `expected` (when set);
/// with `expected == None` it accepts any cert but records what it saw so the
/// caller can ask the user to confirm (trust-on-first-use). The handshake
/// signature is always checked cryptographically, so a matching fingerprint
/// still proves the peer holds the matching private key.
#[derive(Debug)]
struct PinnedVerifier {
    expected: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fp = fingerprint(end_entity);
        *self.seen.lock().unwrap() = Some(fp.clone());
        match &self.expected {
            Some(want) if want.eq_ignore_ascii_case(&fp) => Ok(ServerCertVerified::assertion()),
            Some(_) => Err(rustls::Error::General(
                "the device's certificate fingerprint has changed — not connecting".into(),
            )),
            None => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn build_agent(expected_fp: Option<String>) -> (ureq::Agent, Arc<Mutex<Option<String>>>) {
    let seen = Arc::new(Mutex::new(None));
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let verifier = Arc::new(PinnedVerifier {
        expected: expected_fp,
        seen: seen.clone(),
        provider: provider.clone(),
    });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("rustls protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    let agent = ureq::AgentBuilder::new()
        .tls_config(Arc::new(config))
        .timeout_connect(Duration::from_secs(6))
        .timeout_read(Duration::from_secs(60))
        .build();
    (agent, seen)
}

fn friendly(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(401, _) => "Wrong PIN for that device.".to_string(),
        ureq::Error::Status(403, _) => {
            "That device refused the connection (not on its allowlist?).".to_string()
        }
        ureq::Error::Status(code, _) => format!("The device replied with an error ({code})."),
        ureq::Error::Transport(t) => {
            let m = t.to_string();
            if m.contains("fingerprint has changed") {
                "This device's security fingerprint has changed. If you didn't reset it, don't connect.".to_string()
            } else {
                format!("Couldn't reach the device: {m}")
            }
        }
    }
}

/// Open a connection just far enough to read the peer's cert fingerprint.
/// `expected` enforces a pin; `None` is trust-on-first-use.
pub fn probe(host: &str, port: u16, expected: Option<String>) -> Result<String, String> {
    let (agent, seen) = build_agent(expected);
    agent
        .get(&format!("https://{host}:{port}/healthz"))
        .call()
        .map_err(friendly)?;
    let fp = seen.lock().unwrap().clone();
    fp.ok_or_else(|| "No certificate was presented by the device.".to_string())
}

/// A connected + authenticated session with a peer.
pub struct Session {
    agent: ureq::Agent,
    base: String,
    cookie: String,
}

pub fn connect(host: &str, port: u16, pin: &str, expected: String) -> Result<Session, String> {
    let (agent, _seen) = build_agent(Some(expected));
    let base = format!("https://{host}:{port}");
    let resp = agent
        .post(&format!("{base}/api/auth"))
        .send_json(ureq::json!({ "pin": pin }))
        .map_err(friendly)?;
    let cookie = resp
        .header("set-cookie")
        .and_then(|c| c.split(';').next())
        .map(str::to_string)
        .ok_or_else(|| "The device didn't return a session.".to_string())?;
    Ok(Session { agent, base, cookie })
}

/// One book on a peer, as returned by `/api/books`.
#[derive(Deserialize, Serialize, Clone)]
pub struct PeerBook {
    pub id: String,
    pub title: String,
    pub format: String,
    pub size: i64,
    pub md5: Option<String>,
    pub has_cover: bool,
}

impl Session {
    pub fn books(&self, library: &str) -> Result<Vec<PeerBook>, String> {
        let resp = self
            .agent
            .get(&format!("{}/api/books?library={library}", self.base))
            .set("Cookie", &self.cookie)
            .call()
            .map_err(friendly)?;
        resp.into_json::<Vec<PeerBook>>().map_err(|e| e.to_string())
    }

    /// Stream one book to `dest_file`.
    pub fn download(&self, id: &str, dest_file: &std::path::Path) -> Result<(), String> {
        let resp = self
            .agent
            .get(&format!("{}/api/download/{id}", self.base))
            .set("Cookie", &self.cookie)
            .call()
            .map_err(friendly)?;
        let mut reader = resp.into_reader();
        let mut file = std::fs::File::create(dest_file).map_err(|e| e.to_string())?;
        std::io::copy(&mut reader, &mut file).map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// A settings-safe key for remembering a peer's trusted fingerprint.
pub fn trust_key(host: &str) -> String {
    let h: String = host
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("peer_trust_{h}")
}
