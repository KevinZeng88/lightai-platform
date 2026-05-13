//! Self-signed CA and server certificate generation using rcgen (pure Rust, no openssl).

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, SanType};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

pub struct CaOutput {
    pub ca_cert_pem: String,
    pub ca_key_pem: String,
    pub ca_fingerprint: String,
}

pub struct ServerCertOutput {
    pub cert_pem: String,
    pub key_pem: String,
}

/// Generate a self-signed CA certificate + key.
pub fn generate_ca() -> anyhow::Result<CaOutput> {
    let key = KeyPair::generate()?;
    let mut params = CertificateParams::new(vec!["LightAI Internal CA".to_string()])?;
    params
        .distinguished_name
        .push(DnType::CommonName, "LightAI Internal CA");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.not_before =
        time::OffsetDateTime::now_utc().replace_offset(time::UtcOffset::from_hms(0, 0, 0)?);
    params.not_after = params
        .not_before
        .replace_year(params.not_before.year() + 10)?;

    let cert = params.self_signed(&key)?;
    let cert_der = cert.der().to_vec();
    let fingerprint = sha256_hex(&cert_der);

    Ok(CaOutput {
        ca_cert_pem: cert.pem(),
        ca_key_pem: key.serialize_pem(),
        ca_fingerprint: fingerprint,
    })
}

/// Generate a server certificate signed by the CA.
/// `hosts`: DNS names; `ips`: IPv4/IPv6 addresses as strings.
pub fn generate_server_cert(
    ca_cert_pem: &str,
    ca_key_pem: &str,
    hosts: &[String],
    ips: &[String],
) -> anyhow::Result<ServerCertOutput> {
    let ca_key = KeyPair::from_pem(ca_key_pem)?;
    let ca_params = CertificateParams::from_ca_cert_pem(ca_cert_pem)?;
    // Reconstruct the CA Certificate object needed for signed_by.
    let ca_cert = ca_params.self_signed(&ca_key)?;

    let key = KeyPair::generate()?;
    let mut params = CertificateParams::new(hosts.to_vec())?;
    params.distinguished_name.push(
        DnType::CommonName,
        hosts.first().map(String::as_str).unwrap_or("localhost"),
    );

    for host in hosts {
        params
            .subject_alt_names
            .push(SanType::DnsName(host.clone().try_into()?));
    }
    for ip in ips {
        if let Ok(addr) = ip.parse::<std::net::IpAddr>() {
            params.subject_alt_names.push(SanType::IpAddress(addr));
        }
    }
    if !hosts.iter().any(|h| h == "localhost") {
        params
            .subject_alt_names
            .push(SanType::DnsName("localhost".try_into()?));
    }
    if !ips.iter().any(|i| i == "127.0.0.1") {
        params
            .subject_alt_names
            .push(SanType::IpAddress(std::net::IpAddr::V4(
                std::net::Ipv4Addr::new(127, 0, 0, 1),
            )));
    }

    params.not_before =
        time::OffsetDateTime::now_utc().replace_offset(time::UtcOffset::from_hms(0, 0, 0)?);
    params.not_after = params
        .not_before
        .replace_year(params.not_before.year() + 1)?;

    let cert = params.signed_by(&key, &ca_cert, &ca_key)?;

    Ok(ServerCertOutput {
        cert_pem: cert.pem(),
        key_pem: key.serialize_pem(),
    })
}

/// SHA-256 fingerprint of a DER certificate.
pub fn sha256_hex(der: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(der);
    format!("{:x}", hasher.finalize())
}

/// Write file with mode 600 on Unix.
pub fn write_private(path: &Path, content: &str) -> std::io::Result<()> {
    fs::write(path, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
