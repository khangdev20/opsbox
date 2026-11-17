//! The panel's allowed-address list (Security page): when it is not empty, requests from
//! addresses outside it are refused. Loopback is always allowed, because reaching the
//! panel through an SSH tunnel must never be what a mistyped list takes away.

use std::net::IpAddr;
use std::sync::{Arc, RwLock};

/// One entry: an address and how many leading bits count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    addr: IpAddr,
    bits: u8,
}

impl Cidr {
    pub fn parse(text: &str) -> Result<Self, String> {
        let t = text.trim();
        let bad = || format!("“{t}” is not a valid IP address or range (like 203.0.113.0/24).");
        let (ip, bits) = match t.split_once('/') {
            Some((ip, b)) => (ip, Some(b.parse::<u8>().map_err(|_| bad())?)),
            None => (t, None),
        };
        let addr: IpAddr = ip.parse().map_err(|_| bad())?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let bits = bits.unwrap_or(max);
        if bits > max {
            return Err(bad());
        }
        Ok(Self { addr, bits })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, ip) {
            (IpAddr::V4(a), IpAddr::V4(b)) => prefix_eq(&a.octets(), &b.octets(), self.bits),
            (IpAddr::V6(a), IpAddr::V6(b)) => prefix_eq(&a.octets(), &b.octets(), self.bits),
            // An IPv4 address that arrives mapped into IPv6 counts as the IPv4 one.
            (IpAddr::V4(_), IpAddr::V6(b)) => b
                .to_ipv4_mapped()
                .is_some_and(|m| self.contains(IpAddr::V4(m))),
            _ => false,
        }
    }
}

impl std::fmt::Display for Cidr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let max = if self.addr.is_ipv4() { 32 } else { 128 };
        if self.bits == max {
            write!(f, "{}", self.addr)
        } else {
            write!(f, "{}/{}", self.addr, self.bits)
        }
    }
}

fn prefix_eq(a: &[u8], b: &[u8], bits: u8) -> bool {
    let (full, rest) = (usize::from(bits / 8), bits % 8);
    if a[..full] != b[..full] {
        return false;
    }
    rest == 0 || (a[full] ^ b[full]) >> (8 - rest) == 0
}

/// Parse a comma- or space-separated list. An empty text is an empty list (no restriction).
pub fn parse_list(text: &str) -> Result<Vec<Cidr>, String> {
    text.split([',', ' ', '\n'])
        .filter(|s| !s.trim().is_empty())
        .map(Cidr::parse)
        .collect()
}

pub fn render(list: &[Cidr]) -> String {
    list.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Shared, cheap to clone.
#[derive(Clone, Default)]
pub struct AllowList(Arc<RwLock<Vec<Cidr>>>);

impl AllowList {
    pub fn set(&self, list: Vec<Cidr>) {
        *self
            .0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = list;
    }

    pub fn get(&self) -> Vec<Cidr> {
        self.0
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Whether a request from `ip` may reach the panel.
    pub fn permits(&self, ip: IpAddr) -> bool {
        let list = self
            .0
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        list.is_empty() || ip.is_loopback() || list.iter().any(|c| c.contains(ip))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn ranges_match_by_prefix() {
        let c = Cidr::parse("203.0.113.0/24").unwrap();
        assert!(c.contains(ip("203.0.113.77")) && c.contains(ip("203.0.113.255")));
        assert!(!c.contains(ip("203.0.114.1")) && !c.contains(ip("10.0.0.1")));
        let odd = Cidr::parse("10.1.64.0/20").unwrap();
        assert!(odd.contains(ip("10.1.79.9")) && !odd.contains(ip("10.1.80.0")));
        assert!(Cidr::parse("1.2.3.4").unwrap().contains(ip("1.2.3.4")));
        assert!(!Cidr::parse("1.2.3.4").unwrap().contains(ip("1.2.3.5")));
        assert!(
            Cidr::parse("0.0.0.0/0")
                .unwrap()
                .contains(ip("198.51.100.1"))
        );
        let v6 = Cidr::parse("2001:db8::/32").unwrap();
        assert!(v6.contains(ip("2001:db8:1::5")) && !v6.contains(ip("2001:db9::1")));
        assert!(!v6.contains(ip("1.2.3.4")), "families do not mix");
        assert!(
            Cidr::parse("1.2.3.4")
                .unwrap()
                .contains(ip("::ffff:1.2.3.4")),
            "mapped IPv4"
        );
    }

    #[test]
    fn bad_entries_say_so() {
        for bad in [
            "",
            "abc",
            "1.2.3",
            "1.2.3.4/33",
            "1.2.3.4/x",
            "::1/129",
            "1.2.3.4/-1",
            "999.1.1.1",
        ] {
            let e = Cidr::parse(bad).unwrap_err();
            assert!(e.contains("not a valid"), "{bad}: {e}");
        }
        assert!(parse_list("1.2.3.4, nope").is_err());
        assert_eq!(parse_list("").unwrap(), vec![]);
        assert_eq!(
            parse_list(" 1.2.3.4 ,10.0.0.0/8\n2001:db8::/32 ")
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            render(&parse_list("1.2.3.4,10.0.0.0/8").unwrap()),
            "1.2.3.4, 10.0.0.0/8"
        );
    }

    #[test]
    fn an_empty_list_allows_everyone_and_loopback_is_always_allowed() {
        let a = AllowList::default();
        assert!(a.permits(ip("198.51.100.1")));
        a.set(parse_list("203.0.113.0/24").unwrap());
        assert!(a.permits(ip("203.0.113.9")));
        assert!(!a.permits(ip("198.51.100.1")));
        assert!(
            a.permits(ip("127.0.0.1")) && a.permits(ip("::1")),
            "a tunnel always works"
        );
    }
}
