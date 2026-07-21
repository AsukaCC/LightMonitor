use crate::geo;
use crate::models::{Host, HostDomain};
use crate::state::AppState;
use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures_util::future::join_all;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error, SignatureScheme};
use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::{TcpStream, lookup_host};
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use url::Url;
use uuid::Uuid;
use x509_parser::prelude::parse_x509_certificate;

const PROBE_ATTEMPTS: usize = 4;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub struct HostProbeResult {
    pub resolved_ipv4: Vec<String>,
    pub resolved_ipv6: Vec<String>,
    pub latency_ms: Option<f64>,
    pub packet_loss_percent: Option<f64>,
    pub checked_at: DateTime<Utc>,
    pub error: Option<String>,
    pub region: String,
}

#[derive(Debug, Clone)]
pub struct DomainProbeResult {
    pub id: Uuid,
    pub resolved_ipv4: Vec<String>,
    pub resolved_ipv6: Vec<String>,
    pub ssl_expires_at: Option<DateTime<Utc>>,
    pub ssl_status: String,
    pub latency_ms: Option<f64>,
    pub packet_loss_percent: Option<f64>,
    pub checked_at: DateTime<Utc>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainTarget {
    pub domain: String,
    pub port: u16,
}

pub fn parse_domain_target(input: &str, fallback_port: u16) -> Result<DomainTarget> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("domain is required");
    }
    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let url = Url::parse(&candidate).context("invalid domain or URL")?;
    let domain = url
        .host_str()
        .ok_or_else(|| anyhow!("domain is required"))?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if domain.parse::<IpAddr>().is_ok() {
        bail!("an IP address cannot be added as a domain");
    }
    let port = url.port().unwrap_or(fallback_port);
    if port == 0 {
        bail!("domain port must be between 1 and 65535");
    }
    Ok(DomainTarget { domain, port })
}

pub fn domain_from_host_address(address: &str) -> Option<DomainTarget> {
    let target = parse_domain_target(address, 443).ok()?;
    target.domain.contains('.').then_some(target)
}

pub async fn probe_host(host: &Host) -> (HostProbeResult, Vec<DomainProbeResult>) {
    let address = geo::extract_host(&host.address);
    let host_future = async {
        let addresses = resolve_addresses(&address, host.ssh_port).await;
        let region = if geo::region_has_flag(&host.region) {
            String::new()
        } else {
            geo::resolve_region(&host.address).await
        };
        match addresses {
            Ok(addresses) => {
                let (resolved_ipv4, resolved_ipv6) = split_ip_versions(&addresses);
                let (latency_ms, packet_loss_percent) = probe_addresses(&addresses).await;
                HostProbeResult {
                    resolved_ipv4,
                    resolved_ipv6,
                    latency_ms,
                    packet_loss_percent: Some(packet_loss_percent),
                    checked_at: Utc::now(),
                    error: (latency_ms.is_none())
                        .then(|| "all connection probes failed".to_string()),
                    region,
                }
            }
            Err(error) => HostProbeResult {
                resolved_ipv4: Vec::new(),
                resolved_ipv6: Vec::new(),
                latency_ms: None,
                packet_loss_percent: Some(100.0),
                checked_at: Utc::now(),
                error: Some(error.to_string()),
                region,
            },
        }
    };
    let domain_future = join_all(host.domains.iter().map(probe_domain));
    tokio::join!(host_future, domain_future)
}

pub async fn refresh_host(state: &AppState, host: &Host) -> Result<Host> {
    let (host_probe, domain_probes) = probe_host(host).await;
    state
        .db
        .apply_probe_results(host.id, &host_probe, &domain_probes)?
        .context("host disappeared while saving probe results")
}

async fn probe_domain(domain: &HostDomain) -> DomainProbeResult {
    let checked_at = Utc::now();
    let addresses = match resolve_addresses(&domain.domain, domain.port).await {
        Ok(addresses) => addresses,
        Err(error) => {
            return DomainProbeResult {
                id: domain.id,
                resolved_ipv4: Vec::new(),
                resolved_ipv6: Vec::new(),
                ssl_expires_at: None,
                ssl_status: "unavailable".to_string(),
                latency_ms: None,
                packet_loss_percent: Some(100.0),
                checked_at,
                error: Some(error.to_string()),
            };
        }
    };

    let (resolved_ipv4, resolved_ipv6) = split_ip_versions(&addresses);
    let (latency_ms, packet_loss_percent) = probe_addresses(&addresses).await;
    let certificate = fetch_certificate_expiry(&domain.domain, domain.port).await;
    let (ssl_expires_at, ssl_status, certificate_error) = match certificate {
        Ok(expires_at) => {
            let status = if expires_at <= checked_at {
                "expired"
            } else if expires_at <= checked_at + ChronoDuration::days(30) {
                "expiring"
            } else {
                "valid"
            };
            (Some(expires_at), status.to_string(), None)
        }
        Err(error) => (None, "unavailable".to_string(), Some(error.to_string())),
    };
    let error = if latency_ms.is_none() {
        Some("all connection probes failed".to_string())
    } else {
        certificate_error
    };

    DomainProbeResult {
        id: domain.id,
        resolved_ipv4,
        resolved_ipv6,
        ssl_expires_at,
        ssl_status,
        latency_ms,
        packet_loss_percent: Some(packet_loss_percent),
        checked_at,
        error,
    }
}

async fn resolve_addresses(host: &str, port: u16) -> Result<Vec<SocketAddr>> {
    let addresses = timeout(CONNECT_TIMEOUT, lookup_host((host, port)))
        .await
        .context("DNS lookup timed out")??
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        bail!("DNS lookup returned no addresses");
    }
    Ok(addresses)
}

fn split_ip_versions(addresses: &[SocketAddr]) -> (Vec<String>, Vec<String>) {
    let mut ipv4 = BTreeSet::new();
    let mut ipv6 = BTreeSet::new();
    for address in addresses {
        match address.ip() {
            IpAddr::V4(value) => {
                ipv4.insert(value.to_string());
            }
            IpAddr::V6(value) => {
                ipv6.insert(value.to_string());
            }
        }
    }
    (ipv4.into_iter().collect(), ipv6.into_iter().collect())
}

async fn probe_addresses(addresses: &[SocketAddr]) -> (Option<f64>, f64) {
    let mut elapsed = Vec::new();
    for attempt in 0..PROBE_ATTEMPTS {
        let address = addresses[attempt % addresses.len()];
        let started = Instant::now();
        if timeout(CONNECT_TIMEOUT, TcpStream::connect(address))
            .await
            .is_ok_and(|result| result.is_ok())
        {
            elapsed.push(started.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let successes = elapsed.len();
    let latency_ms = (successes > 0).then(|| elapsed.iter().sum::<f64>() / successes as f64);
    let packet_loss_percent = (PROBE_ATTEMPTS - successes) as f64 / PROBE_ATTEMPTS as f64 * 100.0;
    (latency_ms, packet_loss_percent)
}

async fn fetch_certificate_expiry(domain: &str, port: u16) -> Result<DateTime<Utc>> {
    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyCertificate))
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));
    let server_name =
        ServerName::try_from(domain.to_string()).context("invalid TLS server name")?;
    let tcp = timeout(CONNECT_TIMEOUT, TcpStream::connect((domain, port)))
        .await
        .context("TLS connection timed out")??;
    let stream = timeout(CONNECT_TIMEOUT, connector.connect(server_name, tcp))
        .await
        .context("TLS handshake timed out")??;
    let certificate = stream
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|certificates| certificates.first())
        .ok_or_else(|| anyhow!("server did not provide a certificate"))?;
    let (_, parsed) = parse_x509_certificate(certificate.as_ref())
        .map_err(|error| anyhow!("invalid X.509 certificate: {error}"))?;
    DateTime::from_timestamp(parsed.validity().not_after.timestamp(), 0)
        .ok_or_else(|| anyhow!("certificate expiry is outside the supported date range"))
}

#[derive(Debug)]
struct AcceptAnyCertificate;

impl ServerCertVerifier for AcceptAnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_domains_and_urls() {
        assert_eq!(
            parse_domain_target("https://Example.COM/path", 443).unwrap(),
            DomainTarget {
                domain: "example.com".to_string(),
                port: 443,
            }
        );
        assert_eq!(
            parse_domain_target("status.example.com:8443", 443).unwrap(),
            DomainTarget {
                domain: "status.example.com".to_string(),
                port: 8443,
            }
        );
        assert!(parse_domain_target("192.0.2.1", 443).is_err());
    }

    #[test]
    fn detects_host_address_domains() {
        assert_eq!(
            domain_from_host_address("panel.example.com")
                .unwrap()
                .domain,
            "panel.example.com"
        );
        assert!(domain_from_host_address("192.0.2.1").is_none());
    }

    #[test]
    fn separates_ipv4_and_ipv6_results() {
        let addresses = vec![
            "192.0.2.10:443".parse().unwrap(),
            "[2001:db8::10]:443".parse().unwrap(),
        ];
        let (ipv4, ipv6) = split_ip_versions(&addresses);
        assert_eq!(ipv4, vec!["192.0.2.10"]);
        assert_eq!(ipv6, vec!["2001:db8::10"]);
    }
}
