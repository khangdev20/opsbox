//! Bounded, non-consuming ClientHello/HTTP peeks and raw TCP forwarding.
use std::{
    io,
    time::{Duration, Instant},
};
use tokio::{io::copy_bidirectional, net::TcpStream};

#[derive(Debug, PartialEq, Eq)]
pub enum Hello {
    Incomplete,
    Complete(Option<String>),
    Invalid,
}
fn u16_at(bytes: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as usize)
}
/// TLS 1.2 and 1.3 share the legacy ClientHello framing. Handshakes may span records.
pub fn client_hello(bytes: &[u8]) -> Hello {
    let mut handshake = Vec::new();
    let mut at = 0;
    loop {
        if bytes.len() < at + 5 {
            return Hello::Incomplete;
        }
        if bytes[at] != 22 || bytes[at + 1] != 3 {
            return Hello::Invalid;
        }
        let n = u16_at(bytes, at + 3).unwrap_or(0);
        if n == 0 || n > 18432 {
            return Hello::Invalid;
        }
        if bytes.len() < at + 5 + n {
            return Hello::Incomplete;
        }
        handshake.extend_from_slice(&bytes[at + 5..at + 5 + n]);
        if handshake.len() >= 4 {
            if handshake[0] != 1 {
                return Hello::Invalid;
            }
            let len = ((handshake[1] as usize) << 16)
                | ((handshake[2] as usize) << 8)
                | handshake[3] as usize;
            if len > 65532 {
                return Hello::Invalid;
            }
            if handshake.len() >= len + 4 {
                return parse_hello(&handshake[4..len + 4]).unwrap_or(Hello::Invalid);
            }
        }
        at += 5 + n;
        if at > 65536 {
            return Hello::Invalid;
        }
    }
}
fn parse_hello(b: &[u8]) -> Option<Hello> {
    if b.get(..2)? != [3, 3] {
        return None;
    }
    let mut at = 35 + *b.get(34)? as usize;
    at += 2 + u16_at(b, at)?;
    at += 1 + *b.get(at)? as usize;
    if at == b.len() {
        return Some(Hello::Complete(None));
    }
    let end = at + 2 + u16_at(b, at)?;
    if end != b.len() {
        return None;
    }
    at += 2;
    let mut sni = None;
    while at < end {
        let kind = u16_at(b, at)?;
        let len = u16_at(b, at + 2)?;
        let ext = b.get(at + 4..at + 4 + len)?;
        if kind == 0 {
            if sni.is_some() || u16_at(ext, 0)? + 2 != ext.len() {
                return None;
            }
            let mut p = 2;
            while p < ext.len() {
                let typ = *ext.get(p)?;
                let n = u16_at(ext, p + 1)?;
                let name = ext.get(p + 3..p + 3 + n)?;
                if typ == 0 {
                    if sni.is_some() {
                        return None;
                    }
                    let name = std::str::from_utf8(name).ok()?.to_ascii_lowercase();
                    crate::validate_domain(&name).ok()?;
                    sni = Some(name);
                }
                p += 3 + n;
            }
        }
        at += 4 + len;
    }
    Some(Hello::Complete(sni))
}
pub async fn peek_sni(stream: &TcpStream) -> io::Result<Option<String>> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut buf = vec![0; 65536];
        loop {
            let n = stream.peek(&mut buf).await?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            match client_hello(&buf[..n]) {
                Hello::Complete(sni) => return Ok(sni),
                Hello::Invalid => return Err(io::ErrorKind::InvalidData.into()),
                Hello::Incomplete if n == buf.len() => {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                Hello::Incomplete => tokio::time::sleep(Duration::from_millis(2)).await,
            }
        }
    })
    .await
    .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
}
pub async fn peek_host(stream: &TcpStream) -> io::Result<Option<String>> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut buf = [0; 16384];
        loop {
            let n = stream.peek(&mut buf).await?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            if let Some(end) = buf[..n].windows(4).position(|w| w == b"\r\n\r\n") {
                let text =
                    std::str::from_utf8(&buf[..end]).map_err(|_| io::ErrorKind::InvalidData)?;
                return Ok(text.lines().skip(1).find_map(|line| {
                    let (key, val) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("host").then(|| {
                        val.trim()
                            .split(':')
                            .next()
                            .unwrap_or("")
                            .to_ascii_lowercase()
                    })
                }));
            }
            if n == buf.len() {
                return Err(io::ErrorKind::InvalidData.into());
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
}
pub async fn forward(mut stream: TcpStream, port: u16, domain: &str) {
    let started = Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(7200), async {
        let mut upstream = tokio::time::timeout(
            Duration::from_secs(3),
            TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)),
        )
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
        copy_bidirectional(&mut stream, &mut upstream).await
    })
    .await;
    let (sent, received) = result
        .as_ref()
        .ok()
        .and_then(|r| r.as_ref().ok())
        .copied()
        .unwrap_or_default();
    tracing::info!(
        domain,
        sent,
        received,
        duration_ms = started.elapsed().as_millis() as u64,
        ok = matches!(result, Ok(Ok(_))),
        "adopted access"
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    fn hello(sni: Option<&str>, tls13: bool, fragmented: bool) -> Vec<u8> {
        let mut b = vec![3, 3];
        b.extend([0; 32]);
        b.extend([0, 0, 2, 0x13, 1, 1, 0]);
        let mut ex = Vec::new();
        if let Some(name) = sni {
            let mut e = Vec::new();
            e.extend(((name.len() + 3) as u16).to_be_bytes());
            e.push(0);
            e.extend((name.len() as u16).to_be_bytes());
            e.extend(name.as_bytes());
            ex.extend([0, 0]);
            ex.extend((e.len() as u16).to_be_bytes());
            ex.extend(e);
        }
        if tls13 {
            ex.extend([0, 43, 0, 3, 2, 3, 4]);
        }
        b.extend((ex.len() as u16).to_be_bytes());
        b.extend(ex);
        let mut hs = vec![1, 0, (b.len() >> 8) as u8, b.len() as u8];
        hs.extend(b);
        let mut out = Vec::new();
        for chunk in hs.chunks(if fragmented { 7 } else { 65535 }) {
            out.extend([22, 3, 1]);
            out.extend((chunk.len() as u16).to_be_bytes());
            out.extend(chunk);
        }
        out
    }
    #[test]
    fn versions_sni_missing_fragmentation_and_truncation() {
        for tls13 in [false, true] {
            for fragmented in [false, true] {
                for name in [Some("old.test"), None] {
                    let b = hello(name, tls13, fragmented);
                    assert_eq!(client_hello(&b), Hello::Complete(name.map(str::to_owned)));
                    for i in 0..b.len() {
                        assert_eq!(client_hello(&b[..i]), Hello::Incomplete);
                    }
                }
            }
        }
        assert_eq!(client_hello(b"GET / HTTP/1.1"), Hello::Invalid);
    }
    #[tokio::test]
    async fn peek_and_forward_preserve_every_byte_both_directions() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = backend.local_addr().unwrap().port();
        let front = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let front_addr = front.local_addr().unwrap();
        let bytes = hello(Some("old.test"), true, true);
        let expected = bytes.clone();
        let old = tokio::spawn(async move {
            let (mut s, _) = backend.accept().await.unwrap();
            let mut b = Vec::new();
            s.read_to_end(&mut b).await.unwrap();
            assert_eq!(b, expected);
            s.write_all(b"old server response").await.unwrap();
        });
        let proxy = tokio::spawn(async move {
            let (s, _) = front.accept().await.unwrap();
            assert_eq!(peek_sni(&s).await.unwrap().as_deref(), Some("old.test"));
            forward(s, port, "old.test").await;
        });
        let mut client = TcpStream::connect(front_addr).await.unwrap();
        for chunk in bytes.chunks(3) {
            client.write_all(chunk).await.unwrap();
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        client.shutdown().await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        assert_eq!(response, b"old server response");
        old.await.unwrap();
        proxy.await.unwrap();
    }
}
