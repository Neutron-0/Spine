use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum SsrfError {
    #[error("Invalid webhook URL: {0}")]
    InvalidUrl(String),
    #[error("Unsupported URL scheme '{0}', only http and https are permitted")]
    UnsupportedScheme(String),
    #[error("Missing host in webhook URL")]
    MissingHost,
    #[error("Prohibited port '{0}', access to internal control ports is forbidden")]
    ProhibitedPort(u16),
    #[error("Failed to resolve host '{0}': {1}")]
    ResolutionFailed(String, String),
    #[error("Target IP '{0}' is prohibited by SSRF defense policy (loopback/private/link-local/cloud-metadata)")]
    ProhibitedIp(IpAddr),
}

/// SSRF Validator protecting against internal network access, cloud metadata endpoints,
/// DNS rebinding attacks, and dangerous ports.
#[derive(Debug, Clone, Default)]
pub struct SsrfValidator {
    pub allow_localhost: bool,
}

impl SsrfValidator {
    pub fn new(allow_localhost: bool) -> Self {
        Self { allow_localhost }
    }

    /// Validates a URL and resolves its host into a safe, pinned `SocketAddr`.
    /// Returning the pinned `SocketAddr` prevents DNS rebinding TOCTOU vulnerabilities.
    pub async fn validate_and_resolve(
        &self,
        target_url: &str,
    ) -> Result<(Url, SocketAddr), SsrfError> {
        let parsed = Url::parse(target_url).map_err(|e| SsrfError::InvalidUrl(e.to_string()))?;

        let scheme = parsed.scheme().to_lowercase();
        if scheme != "http" && scheme != "https" {
            return Err(SsrfError::UnsupportedScheme(scheme));
        }

        let host = parsed.host_str().ok_or(SsrfError::MissingHost)?;
        let port = parsed
            .port_or_known_default()
            .unwrap_or(if scheme == "https" { 443 } else { 80 });

        // Check prohibited administrative and database ports
        if is_prohibited_port(port) {
            return Err(SsrfError::ProhibitedPort(port));
        }

        // Resolve host to IP addresses
        let host_port = format!("{}:{}", host, port);
        let addrs = tokio::net::lookup_host(&host_port)
            .await
            .map_err(|e| SsrfError::ResolutionFailed(host.to_string(), e.to_string()))?;

        let mut validated_addr = None;
        for addr in addrs {
            self.validate_ip(addr.ip())?;
            if validated_addr.is_none() {
                validated_addr = Some(addr);
            }
        }

        let chosen_addr = validated_addr.ok_or_else(|| {
            SsrfError::ResolutionFailed(host.to_string(), "No IP addresses resolved".to_string())
        })?;

        Ok((parsed, chosen_addr))
    }

    /// Evaluates whether an IP address is within forbidden ranges.
    pub fn validate_ip(&self, ip: IpAddr) -> Result<(), SsrfError> {
        match ip {
            IpAddr::V4(ipv4) => self.validate_ipv4(ipv4),
            IpAddr::V6(ipv6) => {
                // If it's an IPv4-mapped IPv6 address (e.g., ::ffff:127.0.0.1), unwrap it
                if let Some(mapped) = ipv6.to_ipv4_mapped() {
                    self.validate_ipv4(mapped)
                } else {
                    self.validate_ipv6(ipv6)
                }
            }
        }
    }

    fn validate_ipv4(&self, ip: Ipv4Addr) -> Result<(), SsrfError> {
        let octets = ip.octets();

        // 0.0.0.0/8 (Broadcast/Unspecified)
        if octets[0] == 0 {
            return Err(SsrfError::ProhibitedIp(IpAddr::V4(ip)));
        }

        // 127.0.0.0/8 (Loopback)
        if ip.is_loopback() {
            if self.allow_localhost {
                return Ok(());
            }
            return Err(SsrfError::ProhibitedIp(IpAddr::V4(ip)));
        }

        // 169.254.0.0/16 (Link-local / Cloud metadata service e.g. 169.254.169.254)
        if ip.is_link_local() || (octets[0] == 169 && octets[1] == 254) {
            return Err(SsrfError::ProhibitedIp(IpAddr::V4(ip)));
        }

        // RFC 1918 Private ranges:
        // 10.0.0.0/8
        // 172.16.0.0/12 (172.16.0.0 – 172.31.255.255)
        // 192.168.0.0/16
        if ip.is_private() {
            if self.allow_localhost {
                return Ok(());
            }
            return Err(SsrfError::ProhibitedIp(IpAddr::V4(ip)));
        }

        // 100.64.0.0/10 (Carrier-Grade NAT)
        if octets[0] == 100 && (octets[1] >= 64 && octets[1] <= 127) {
            return Err(SsrfError::ProhibitedIp(IpAddr::V4(ip)));
        }

        // Documentation and benchmarking:
        // 192.0.2.0/24 (TEST-NET-1), 198.51.100.0/24 (TEST-NET-2), 203.0.113.0/24 (TEST-NET-3)
        // 198.18.0.0/15 (Benchmarking)
        if (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
            || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
            || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
            || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
        {
            return Err(SsrfError::ProhibitedIp(IpAddr::V4(ip)));
        }

        // 224.0.0.0/4 (Multicast) and 240.0.0.0/4 (Reserved)
        if octets[0] >= 224 {
            return Err(SsrfError::ProhibitedIp(IpAddr::V4(ip)));
        }

        Ok(())
    }

    fn validate_ipv6(&self, ip: Ipv6Addr) -> Result<(), SsrfError> {
        // Loopback ::1
        if ip.is_loopback() {
            if self.allow_localhost {
                return Ok(());
            }
            return Err(SsrfError::ProhibitedIp(IpAddr::V6(ip)));
        }

        // Unspecified ::
        if ip.is_unspecified() {
            return Err(SsrfError::ProhibitedIp(IpAddr::V6(ip)));
        }

        // Unique Local Address (ULA) fc00::/7
        let segments = ip.segments();
        if (segments[0] & 0xfe00) == 0xfc00 {
            if self.allow_localhost {
                return Ok(());
            }
            return Err(SsrfError::ProhibitedIp(IpAddr::V6(ip)));
        }

        // Link-local unicast fe80::/10
        if (segments[0] & 0xffc0) == 0xfe80 {
            return Err(SsrfError::ProhibitedIp(IpAddr::V6(ip)));
        }

        // Multicast ff00::/8
        if (segments[0] & 0xff00) == 0xff00 {
            return Err(SsrfError::ProhibitedIp(IpAddr::V6(ip)));
        }

        Ok(())
    }
}

fn is_prohibited_port(port: u16) -> bool {
    matches!(
        port,
        22    // SSH
        | 23  // Telnet
        | 25  // SMTP
        | 53  // DNS
        | 69  // TFTP
        | 111 // RPC
        | 135 // MS RPC
        | 139 // NetBIOS
        | 445 // SMB
        | 2049 // NFS
        | 2222 // Spine SSH Listener
        | 2379 | 2380 // etcd
        | 3306 // MySQL
        | 5432 // PostgreSQL
        | 6379 // Redis
        | 8400 // Spine Coordinator RPC
        | 8401 // Spine Storage Node RPC
        | 9200 | 9300 // Elasticsearch
        | 11211 // Memcached
        | 27017 // MongoDB
    )
}
