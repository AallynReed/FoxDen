use crate::error::{Error, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Accepts SteelSeries' self-signed certificates, but only from this computer, and only the
/// pinned certificate when one is known.
#[derive(Debug)]
struct LoopbackVerifier {
    pinned: Option<Vec<u8>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for LoopbackVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let local = match server_name {
            ServerName::IpAddress(ip) => std::net::IpAddr::from(*ip).is_loopback(),
            ServerName::DnsName(d) => d.as_ref().eq_ignore_ascii_case("localhost"),
            _ => false,
        };
        if !local {
            return Err(rustls::Error::General("not a loopback server".into()));
        }
        if let Some(pin) = &self.pinned
            && pin.as_slice() != end_entity.as_ref()
        {
            return Err(rustls::Error::General("certificate does not match the pinned one".into()));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// A small HTTP client limited to loopback addresses.
/// Last write per route, shared by every client in the process.
fn last_writes() -> &'static Mutex<HashMap<String, Instant>> {
    static LAST: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    LAST.get_or_init(Mutex::default)
}

#[derive(Clone)]
pub struct Http {
    agent: ureq::Agent,
    min_interval: Duration,
}

impl Http {
    /// `pinned_der` pins the server certificate (Engine publishes its own through GG).
    pub fn new(pinned_der: Option<Vec<u8>>, timeout: Duration) -> Self {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = Arc::new(LoopbackVerifier { pinned: pinned_der, provider: provider.clone() });
        let tls = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("ring supports the default TLS versions")
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();
        let agent = ureq::AgentBuilder::new()
            .tls_config(Arc::new(tls))
            .timeout_connect(timeout)
            .timeout(timeout * 2)
            .build();
        Self { agent, min_interval: Duration::from_millis(170) }
    }

    pub fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        let text = self.send("GET", url, None)?;
        serde_json::from_str(&text).map_err(|e| Error::Parse(format!("{url}: {e}")))
    }

    pub fn send_json<T: DeserializeOwned>(&self, method: &str, url: &str, body: Option<&Value>) -> Result<T> {
        let text = self.send(method, url, body)?;
        let text = if text.trim().is_empty() { "null" } else { &text };
        serde_json::from_str(text).map_err(|e| Error::Parse(format!("{url}: {e}")))
    }

    /// Writes to the same route closer than ~170 ms apart are spaced out (Sonar observes 3 per 500 ms).
    pub fn send(&self, method: &str, url: &str, body: Option<&Value>) -> Result<String> {
        check_loopback(url)?;
        if method != "GET" {
            self.pace(method, url);
        }
        let req = self.agent.request(method, url);
        let res = match body {
            Some(b) => req.send_json(b.clone()),
            None if method == "GET" => req.call(),
            None => req.send_string(""),
        };
        match res {
            Ok(r) => r.into_string().map_err(|e| Error::Parse(e.to_string())),
            Err(ureq::Error::Status(code, r)) => Err(Error::Status { code, body: r.into_string().unwrap_or_default() }),
            Err(ureq::Error::Transport(t)) => Err(Error::Offline(t.to_string())),
        }
    }

    fn pace(&self, method: &str, url: &str) {
        let key = format!("{method} {}", url.split('?').next().unwrap_or(url));
        let route: String = key.rsplitn(2, '/').last().unwrap_or(&key).to_string();
        let wait = {
            let mut map = last_writes().lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            let next = map.get(&route).map(|t| *t + self.min_interval).unwrap_or(now);
            let at = next.max(now);
            map.insert(route, at);
            at - now
        };
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
    }
}

pub fn check_loopback(url: &str) -> Result<()> {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let host_port = rest.split('/').next().unwrap_or(rest);
    let host = if let Some(h) = host_port.strip_prefix('[') {
        h.split(']').next().unwrap_or(h)
    } else {
        host_port.rsplit_once(':').map(|(h, _)| h).unwrap_or(host_port)
    };
    let ok = host.eq_ignore_ascii_case("localhost")
        || host.parse::<std::net::IpAddr>().map(|ip| ip.is_loopback()).unwrap_or(false);
    if ok { Ok(()) } else { Err(Error::NotLoopback(host.to_string())) }
}

/// Decodes the first PEM certificate block to DER.
pub fn pem_to_der(pem: &str) -> Option<Vec<u8>> {
    let body: String = pem
        .lines()
        .skip_while(|l| !l.starts_with("-----BEGIN"))
        .skip(1)
        .take_while(|l| !l.starts_with("-----END"))
        .collect();
    base64_decode(body.trim())
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut buf, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\r' | b'\n' | b' ' => continue,
            _ => return None,
        } as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_only() {
        assert!(check_loopback("https://127.0.0.1:6327/subApps").is_ok());
        assert!(check_loopback("http://localhost:1/x").is_ok());
        assert!(check_loopback("http://[::1]:5/x").is_ok());
        assert!(check_loopback("https://example.com/x").is_err());
        assert!(check_loopback("http://192.168.1.2:80/").is_err());
    }

    #[test]
    fn base64() {
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
    }
}
