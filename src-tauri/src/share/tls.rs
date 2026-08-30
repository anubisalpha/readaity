//! rustls config for the share server: TLS 1.3 only, AEAD suites, aws-lc-rs.

use std::sync::Arc;

use rustls::crypto::aws_lc_rs;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::ServerConfig;

pub fn server_config(cert_pem: &[u8], key_pem: &[u8]) -> Result<ServerConfig, String> {
    // Harmless if another part of the app already installed one.
    let _ = aws_lc_rs::default_provider().install_default();

    let certs = parse_certs(cert_pem)?;
    let key = parse_key(key_pem)?;

    // AEAD-only, strongest first.
    let cipher_suites = vec![
        aws_lc_rs::cipher_suite::TLS13_AES_256_GCM_SHA384,
        aws_lc_rs::cipher_suite::TLS13_CHACHA20_POLY1305_SHA256,
        aws_lc_rs::cipher_suite::TLS13_AES_128_GCM_SHA256,
    ];
    let provider = Arc::new(rustls::crypto::CryptoProvider {
        cipher_suites,
        ..aws_lc_rs::default_provider()
    });

    let mut config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| e.to_string())?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

fn parse_certs(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>, String> {
    let mut rd = pem;
    rustls_pemfile::certs(&mut rd)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn parse_key(pem: &[u8]) -> Result<PrivateKeyDer<'static>, String> {
    let mut rd = pem;
    rustls_pemfile::private_key(&mut rd)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no private key in stored PEM".to_string())
}
