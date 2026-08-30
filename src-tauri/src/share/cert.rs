//! Self-signed certificate for the share server. See docs/NETWORK_SHARING.md.

use std::net::{IpAddr, Ipv4Addr};

use base64::Engine as _;
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, SanType};
use sha2::{Digest, Sha256};

/// Make a fresh self-signed cert. Returns `(cert_pem, key_pem)`.
pub fn generate() -> Result<(String, String), String> {
    let mut params = CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "Readaity");
    params.distinguished_name = dn;
    params.subject_alt_names = san_list();

    let key_pair = KeyPair::generate().map_err(|e| e.to_string())?;
    let cert = params.self_signed(&key_pair).map_err(|e| e.to_string())?;
    Ok((cert.pem(), key_pair.serialize_pem()))
}

fn san_list() -> Vec<SanType> {
    let mut v = vec![
        SanType::DnsName("localhost".try_into().unwrap()),
        SanType::DnsName("readaity.local".try_into().unwrap()),
        SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)),
    ];
    for ip in super::lan_ips() {
        v.push(SanType::IpAddress(ip));
    }
    v
}

/// `AB:CD:…` SHA-256 of the certificate's DER, for out-of-band verification.
pub fn fingerprint_from_pem(cert_pem: &str) -> Result<String, String> {
    let der = pem_to_der(cert_pem)?;
    let digest = Sha256::digest(der);
    Ok(digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":"))
}

fn pem_to_der(pem: &str) -> Result<Vec<u8>, String> {
    let b64: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|e| e.to_string())
}
