//! DNS checks (`hickory-resolver`) and server public-IP detection.

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::Duration;

use hickory_resolver::Resolver;
use hickory_resolver::config::{
    CLOUDFLARE, ConnectionConfig, GOOGLE, LookupIpStrategy, NameServerConfig, ResolveHosts,
    ResolverConfig, ResolverOpts,
};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::proto::rr::{RData, RecordType};
use serde::{Deserialize, Serialize};

use crate::domain::normalize_domain;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnsStatus {
    PointsHere,
    Waiting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsRecordHint {
    #[serde(rename = "type")]
    pub record_type: String,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DnsCheckResult {
    pub domain: String,
    pub status: DnsStatus,
    pub points_here: bool,
    pub message: String,
    pub resolved_ips: Vec<IpAddr>,
    pub server_ips: Vec<IpAddr>,
    pub cname: Option<String>,
    pub records: Vec<DnsRecordHint>,
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !v4.is_loopback()
                && !v4.is_private()
                && !v4.is_link_local()
                && !v4.is_broadcast()
                && !v4.is_documentation()
                && !v4.is_unspecified()
                && !v4.is_multicast()
                // CGNAT 100.64.0.0/10
                && !(v4.octets()[0] == 100 && (v4.octets()[1] & 0b1100_0000) == 64)
        }
        IpAddr::V6(v6) => {
            !v6.is_loopback()
                && !v6.is_unspecified()
                && !v6.is_multicast()
                // Unique local fc00::/7
                && (v6.segments()[0] & 0xfe00) != 0xfc00
                // Link-local fe80::/10
                && (v6.segments()[0] & 0xffc0) != 0xfe80
        }
    }
}

fn push_unique(out: &mut Vec<IpAddr>, ip: IpAddr) {
    if !out.contains(&ip) {
        out.push(ip);
    }
}

fn metadata_agent() -> ureq::Agent {
    let cfg = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(500)))
        .build();
    cfg.into()
}

fn parse_ip_body(resp: ureq::http::Response<ureq::Body>) -> Option<IpAddr> {
    let text = resp.into_body().read_to_string().ok()?;
    text.trim().parse::<IpAddr>().ok()
}

/// Detect the server's public IPv4 / IPv6 addresses from cloud metadata (with short
/// timeouts), outbound interface addresses, or optional explicit overrides.
pub fn detect_public_ips(overrides: &[IpAddr]) -> Vec<IpAddr> {
    if !overrides.is_empty() {
        return overrides.to_vec();
    }
    if let Ok(raw) = std::env::var("OPSBOX_PUBLIC_IPS") {
        let parsed: Vec<IpAddr> = raw
            .split(',')
            .filter_map(|s| s.trim().parse::<IpAddr>().ok())
            .collect();
        if !parsed.is_empty() {
            return parsed;
        }
    }

    let mut ips = Vec::new();
    if std::env::var_os("OPSBOX_NO_IMDS").is_none() {
        let agent = metadata_agent();

        // AWS IMDSv2
        if let Ok(tok_resp) = agent
            .put("http://169.254.169.254/latest/api/token")
            .header("X-aws-ec2-metadata-token-ttl-seconds", "60")
            .send_empty()
            && let Ok(token) = tok_resp.into_body().read_to_string()
        {
            let tok = token.trim();
            if let Ok(ip_resp) = agent
                .get("http://169.254.169.254/latest/meta-data/public-ipv4")
                .header("X-aws-ec2-metadata-token", tok)
                .call()
                && let Some(ip) = parse_ip_body(ip_resp)
                && is_public_ip(ip)
            {
                push_unique(&mut ips, ip);
            }
            if let Ok(ip6_resp) = agent
                .get("http://169.254.169.254/latest/meta-data/ipv6")
                .header("X-aws-ec2-metadata-token", tok)
                .call()
                && let Some(ip) = parse_ip_body(ip6_resp)
                && is_public_ip(ip)
            {
                push_unique(&mut ips, ip);
            }
        }

        // GCP
        if ips.is_empty()
            && let Ok(resp) = agent
                .get("http://169.254.169.254/computeMetadata/v1/instance/network-interfaces/0/access-configs/0/external-ip")
                .header("Metadata-Flavor", "Google")
                .call()
            && let Some(ip) = parse_ip_body(resp)
            && is_public_ip(ip)
        {
            push_unique(&mut ips, ip);
        }

        // Azure
        if ips.is_empty()
            && let Ok(resp) = agent
                .get("http://169.254.169.254/metadata/instance/network/interface/0/ipv4/ipAddress/0/publicIpAddress?api-version=2021-02-01&format=text")
                .header("Metadata", "true")
                .call()
            && let Some(ip) = parse_ip_body(resp)
            && is_public_ip(ip)
        {
            push_unique(&mut ips, ip);
        }

        // DigitalOcean
        if ips.is_empty()
            && let Ok(resp) = agent
                .get("http://169.254.169.254/metadata/v1/interfaces/public/0/ipv4/address")
                .call()
            && let Some(ip) = parse_ip_body(resp)
            && is_public_ip(ip)
        {
            push_unique(&mut ips, ip);
        }

        // Hetzner
        if ips.is_empty()
            && let Ok(resp) = agent
                .get("http://169.254.169.254/hetzner/v1/metadata/public-ipv4")
                .call()
            && let Some(ip) = parse_ip_body(resp)
            && is_public_ip(ip)
        {
            push_unique(&mut ips, ip);
        }
    }

    // Primary outbound IPv4 / IPv6 interface addresses (UDP connect sends no packet).
    if let Ok(sock) = UdpSocket::bind("0.0.0.0:0")
        && sock.connect("1.1.1.1:80").is_ok()
        && let Ok(addr) = sock.local_addr()
        && is_public_ip(addr.ip())
    {
        push_unique(&mut ips, addr.ip());
    }
    if let Ok(sock) = UdpSocket::bind("[::]:0")
        && sock.connect("[2606:4700:4700::1111]:80").is_ok()
        && let Ok(addr) = sock.local_addr()
        && is_public_ip(addr.ip())
    {
        push_unique(&mut ips, addr.ip());
    }

    // Fallback for NATed VMs where IMDS is unavailable and interface IP is RFC1918.
    if ips.is_empty() && std::env::var_os("OPSBOX_NO_IMDS").is_none() {
        let cfg = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(1200)))
            .build();
        let agent: ureq::Agent = cfg.into();
        if let Ok(resp) = agent.get("https://api.ipify.org").call()
            && let Some(ip) = parse_ip_body(resp)
            && is_public_ip(ip)
        {
            push_unique(&mut ips, ip);
        }
    }

    ips
}

/// Compare resolved A/AAAA records against this server's public IPs and produce exact
/// DNS record instructions when waiting.
pub fn evaluate_dns(
    domain: &str,
    resolved_ips: &[IpAddr],
    server_ips: &[IpAddr],
    cname: Option<&str>,
) -> DnsCheckResult {
    let records: Vec<DnsRecordHint> = server_ips
        .iter()
        .map(|ip| DnsRecordHint {
            record_type: if ip.is_ipv4() {
                "A".to_owned()
            } else {
                "AAAA".to_owned()
            },
            name: domain.to_owned(),
            value: ip.to_string(),
        })
        .collect();

    if server_ips.is_empty() {
        return DnsCheckResult {
            domain: domain.to_owned(),
            status: DnsStatus::Waiting,
            points_here: false,
            message: "Could not detect this server's public IP address. Set [proxy] public_ips in /etc/opsbox/config.toml.".to_owned(),
            resolved_ips: resolved_ips.to_vec(),
            server_ips: Vec::new(),
            cname: cname.map(ToOwned::to_owned),
            records,
        };
    }

    if resolved_ips.is_empty() {
        let cname_note = cname.map_or_else(String::new, |c| format!(" (CNAME points to {c})"));
        let first_rec = records.first().map_or_else(String::new, |r| {
            format!(" Add {} {} -> {}.", r.record_type, r.name, r.value)
        });
        return DnsCheckResult {
            domain: domain.to_owned(),
            status: DnsStatus::Waiting,
            points_here: false,
            message: format!(
                "Waiting for DNS: no A or AAAA record found for {domain} yet{cname_note}.{first_rec}"
            ),
            resolved_ips: Vec::new(),
            server_ips: server_ips.to_vec(),
            cname: cname.map(ToOwned::to_owned),
            records,
        };
    }

    let any_match = resolved_ips.iter().any(|ip| server_ips.contains(ip));
    let foreign: Vec<String> = resolved_ips
        .iter()
        .filter(|ip| !server_ips.contains(ip))
        .map(ToString::to_string)
        .collect();

    if any_match && foreign.is_empty() {
        let matched: Vec<String> = resolved_ips.iter().map(ToString::to_string).collect();
        return DnsCheckResult {
            domain: domain.to_owned(),
            status: DnsStatus::PointsHere,
            points_here: true,
            message: format!("Points here ({}).", matched.join(", ")),
            resolved_ips: resolved_ips.to_vec(),
            server_ips: server_ips.to_vec(),
            cname: cname.map(ToOwned::to_owned),
            records,
        };
    }

    if any_match {
        return DnsCheckResult {
            domain: domain.to_owned(),
            status: DnsStatus::Waiting,
            points_here: false,
            message: format!(
                "Waiting for DNS: {domain} also resolves to {} (not this server). Remove extra A/AAAA records.",
                foreign.join(", ")
            ),
            resolved_ips: resolved_ips.to_vec(),
            server_ips: server_ips.to_vec(),
            cname: cname.map(ToOwned::to_owned),
            records,
        };
    }

    let got: Vec<String> = resolved_ips.iter().map(ToString::to_string).collect();
    let expected: Vec<String> = server_ips.iter().map(ToString::to_string).collect();
    DnsCheckResult {
        domain: domain.to_owned(),
        status: DnsStatus::Waiting,
        points_here: false,
        message: format!(
            "Waiting for DNS: {domain} resolves to {}, expected {}.",
            got.join(", "),
            expected.join(" / ")
        ),
        resolved_ips: resolved_ips.to_vec(),
        server_ips: server_ips.to_vec(),
        cname: cname.map(ToOwned::to_owned),
        records,
    }
}

/// Resolve `domain` against public resolvers (`1.1.1.1`, `8.8.8.8`, etc.) using
/// `hickory-resolver` and compare with `server_ips`.
pub async fn check_dns(
    domain_input: &str,
    server_ips: &[IpAddr],
) -> Result<DnsCheckResult, String> {
    check_dns_with_nameservers(domain_input, server_ips, None).await
}

/// Same as [`check_dns`], with optional explicit nameserver socket addresses (for tests).
pub async fn check_dns_with_nameservers(
    domain_input: &str,
    server_ips: &[IpAddr],
    custom_nameservers: Option<&[SocketAddr]>,
) -> Result<DnsCheckResult, String> {
    let domain = normalize_domain(domain_input).map_err(|e| e.0)?;

    let cfg = if let Some(addrs) = custom_nameservers {
        let servers = addrs
            .iter()
            .map(|sa| {
                let mut conn = ConnectionConfig::udp();
                conn.port = sa.port();
                NameServerConfig::new(sa.ip(), true, vec![conn])
            })
            .collect();
        ResolverConfig::from_name_servers(servers)
    } else {
        let has_v6 = server_ips.iter().any(IpAddr::is_ipv6);
        let servers: Vec<NameServerConfig> = CLOUDFLARE
            .udp_and_tcp()
            .chain(GOOGLE.udp_and_tcp())
            .filter(|ns| has_v6 || ns.ip.is_ipv4())
            .collect();
        ResolverConfig::from_name_servers(servers)
    };

    let mut builder = Resolver::builder_with_config(cfg, TokioRuntimeProvider::default());
    {
        let opts: &mut ResolverOpts = builder.options_mut();
        opts.ip_strategy = LookupIpStrategy::Ipv4AndIpv6;
        opts.use_hosts_file = ResolveHosts::Never;
        opts.timeout = Duration::from_secs(3);
        opts.attempts = 2;
        opts.cache_size = 0;
    }
    let resolver = builder
        .build()
        .map_err(|e| format!("DNS resolver setup failed: {e}"))?;

    let fqdn = format!("{domain}.");
    let mut resolved = Vec::new();
    match resolver.lookup_ip(fqdn.as_str()).await {
        Ok(lookup) => {
            for ip in lookup.iter() {
                push_unique(&mut resolved, ip);
            }
        }
        Err(e) if e.is_no_records_found() || e.is_nx_domain() => {}
        Err(e) => {
            return Err(format!("DNS lookup for {domain} failed: {e}"));
        }
    }

    let cname = match resolver.lookup(fqdn.as_str(), RecordType::CNAME).await {
        Ok(lookup) => lookup.answers().iter().find_map(|rec| match &rec.data {
            RData::CNAME(c) => Some(c.to_string().trim_end_matches('.').to_owned()),
            _ => None,
        }),
        Err(_) => None,
    };

    Ok(evaluate_dns(
        &domain,
        &resolved,
        server_ips,
        cname.as_deref(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};
    use tokio::net::UdpSocket as TokioUdpSocket;

    #[test]
    fn evaluates_matching_and_mismatched_dns() {
        let srv = [IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10))];

        let ok = evaluate_dns("panel.example.com", &srv, &srv, None);
        assert!(ok.points_here);
        assert_eq!(ok.status, DnsStatus::PointsHere);
        assert_eq!(ok.records.len(), 1);
        assert_eq!(ok.records[0].record_type, "A");
        assert_eq!(ok.records[0].value, "203.0.113.10");

        let empty = evaluate_dns("panel.example.com", &[], &srv, None);
        assert!(!empty.points_here);
        assert_eq!(empty.status, DnsStatus::Waiting);
        assert!(empty.message.contains("Waiting for DNS"));

        let wrong = evaluate_dns(
            "panel.example.com",
            &[IpAddr::V4(Ipv4Addr::new(198, 51, 100, 5))],
            &srv,
            None,
        );
        assert!(!wrong.points_here);
        assert!(wrong.message.contains("198.51.100.5"));
        assert!(wrong.message.contains("203.0.113.10"));

        let mixed = evaluate_dns(
            "panel.example.com",
            &[
                IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)),
                IpAddr::V4(Ipv4Addr::new(198, 51, 100, 5)),
            ],
            &srv,
            None,
        );
        assert!(!mixed.points_here);
        assert!(mixed.message.contains("198.51.100.5"));

        // Dual-stack server with single matching A record points here.
        let dual = [
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)),
            IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
        ];
        let v4_only = evaluate_dns("panel.example.com", &srv, &dual, None);
        assert!(v4_only.points_here);
        assert_eq!(v4_only.records.len(), 2);
    }

    #[test]
    fn filters_private_and_loopback_ips() {
        assert!(!is_public_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(!is_public_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5))));
        assert!(!is_public_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        assert!(!is_public_ip(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
        assert!(!is_public_ip(IpAddr::V4(Ipv4Addr::new(100, 64, 1, 1))));
        assert!(!is_public_ip(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)))); // RFC 5737 TEST-NET-3
        assert!(is_public_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
        assert!(!is_public_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(!is_public_ip(IpAddr::V6(Ipv6Addr::new(
            0xfc00, 0, 0, 0, 0, 0, 0, 1
        ))));
        assert!(is_public_ip(IpAddr::V6(Ipv6Addr::new(
            0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888
        ))));
    }

    /// Minimal hermetic UDP DNS server responding with an A record for `panel.example.com.`.
    async fn spawn_test_dns(ip_to_return: Option<Ipv4Addr>) -> SocketAddr {
        let sock = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = sock.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            for _ in 0..8 {
                let Ok((len, peer)) = sock.recv_from(&mut buf).await else {
                    break;
                };
                if len < 12 {
                    continue;
                }
                // Parse question end and QTYPE.
                let mut pos = 12;
                while pos < len && buf[pos] != 0 {
                    let l = buf[pos] as usize;
                    pos += 1 + l;
                }
                if pos + 5 > len {
                    continue;
                }
                let qend = pos + 5;
                let qtype = u16::from_be_bytes([buf[pos + 1], buf[pos + 2]]);

                let mut resp = Vec::new();
                resp.extend_from_slice(&buf[0..2]); // ID
                let rcode = if ip_to_return.is_none() && qtype == 1 {
                    3u8 // NXDOMAIN
                } else {
                    0u8
                };
                resp.push(0x81); // QR=1, RD=1
                resp.push(0x80 | rcode); // RA=1, RCODE
                resp.extend_from_slice(&[0, 1]); // QDCOUNT = 1
                if qtype == 1 && ip_to_return.is_some() {
                    resp.extend_from_slice(&[0, 1]); // ANCOUNT = 1
                } else {
                    resp.extend_from_slice(&[0, 0]); // ANCOUNT = 0
                }
                resp.extend_from_slice(&[0, 0, 0, 0]); // NSCOUNT, ARCOUNT
                resp.extend_from_slice(&buf[12..qend]); // Question
                if qtype == 1
                    && let Some(ip) = ip_to_return
                {
                    resp.extend_from_slice(&[0xc0, 0x0c]); // Name pointer
                    resp.extend_from_slice(&1u16.to_be_bytes()); // A
                    resp.extend_from_slice(&1u16.to_be_bytes()); // IN
                    resp.extend_from_slice(&60u32.to_be_bytes()); // TTL
                    resp.extend_from_slice(&4u16.to_be_bytes()); // RDLENGTH
                    resp.extend_from_slice(&ip.octets());
                }
                let _ = sock.send_to(&resp, peer).await;
            }
        });
        addr
    }

    #[tokio::test]
    async fn hickory_resolver_checks_a_record_against_local_dns_server() {
        let server_ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 42));

        let ok_dns = spawn_test_dns(Some(Ipv4Addr::new(203, 0, 113, 42))).await;
        let ok = check_dns_with_nameservers("panel.example.com", &[server_ip], Some(&[ok_dns]))
            .await
            .unwrap();
        assert!(ok.points_here, "{ok:?}");
        assert_eq!(ok.status, DnsStatus::PointsHere);

        let nx_dns = spawn_test_dns(None).await;
        let wait = check_dns_with_nameservers("panel.example.com", &[server_ip], Some(&[nx_dns]))
            .await
            .unwrap();
        assert!(!wait.points_here, "{wait:?}");
        assert_eq!(wait.status, DnsStatus::Waiting);
        assert_eq!(wait.records[0].value, "203.0.113.42");
    }
}
