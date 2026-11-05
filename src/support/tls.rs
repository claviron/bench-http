//! Startup-only TLS for the native hosts. Without the two variables a host
//! serves cleartext, exactly as before.

use std::io;
use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::TlsAcceptor;

/// Whether the host speaks HTTP/2: `BENCH_HTTP=2`. Absent or `1` is HTTP/1.1.
pub fn http2_from_env() -> io::Result<bool> {
    match std::env::var("BENCH_HTTP").as_deref() {
        Err(std::env::VarError::NotPresent) | Ok("1") => Ok(false),
        Ok("2") => Ok(true),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "BENCH_HTTP must be 1 or 2",
        )),
    }
}

/// `BENCH_TLS_CERT` names the PEM chain and `BENCH_TLS_KEY` its private key.
/// Both or neither: one without the other is a configuration error. The
/// protocol offered in the handshake is the one the host speaks.
pub fn acceptor_from_env(http2: bool) -> io::Result<Option<TlsAcceptor>> {
    let chain = std::env::var_os("BENCH_TLS_CERT");
    let key = std::env::var_os("BENCH_TLS_KEY");
    let (chain, key) = match (chain, key) {
        (None, None) => return Ok(None),
        (Some(chain), Some(key)) => (chain, key),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "BENCH_TLS_CERT and BENCH_TLS_KEY must be set together",
            ));
        }
    };
    let chain = CertificateDer::pem_file_iter(&chain)
        .and_then(|certificates| certificates.collect::<Result<Vec<_>, _>>())
        .map_err(io::Error::other)?;
    let key = PrivateKeyDer::from_pem_file(&key).map_err(io::Error::other)?;
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(io::Error::other)?
    .with_no_client_auth()
    .with_single_cert(chain, key)
    .map_err(io::Error::other)?;
    config.alpn_protocols =
        vec![if http2 { b"h2".to_vec() } else { b"http/1.1".to_vec() }];
    Ok(Some(TlsAcceptor::from(Arc::new(config))))
}
