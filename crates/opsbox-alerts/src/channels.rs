//! Outbound delivery. Errors intentionally omit transport details: URLs and SMTP
//! responses may contain passwords or bearer tokens. No redirects, plaintext TLS,
//! shell commands, or system key access.
use hmac::{Hmac, KeyInit, Mac};
use lettre::{Message, SmtpTransport, Transport, transport::smtp::authentication::Credentials};
use opsbox_proto::Secret;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use std::time::Duration;
use url::Url;

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Settings {
    Email {
        host: String,
        port: u16,
        username: String,
        password: Secret,
        from: String,
        to: String,
    },
    Slack {
        url: Secret,
    },
    Telegram {
        token: Secret,
        chat: String,
    },
    Webhook {
        url: Secret,
        secret: Secret,
    },
}
impl Settings {
    pub fn id(&self) -> &'static str {
        match self {
            Self::Email { .. } => "c1",
            Self::Slack { .. } => "c2",
            Self::Telegram { .. } => "c3",
            Self::Webhook { .. } => "c4",
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Email {
                host,
                port,
                username,
                password,
                from,
                to,
            } => {
                if host.is_empty()
                    || host.len() > 253
                    || host
                        .bytes()
                        .any(|b| !b.is_ascii_alphanumeric() && !matches!(b, b'.' | b'-'))
                    || ![465, 587].contains(port)
                    || username.is_empty()
                    || password.expose().is_empty()
                    || from.parse::<lettre::message::Mailbox>().is_err()
                    || to.parse::<lettre::message::Mailbox>().is_err()
                {
                    return Err(
                        "Enter a valid SMTP host, port 465 or 587, credentials, and sender and recipient addresses.",
                    );
                }
            }
            Self::Slack { url } => {
                let u = secure_url(url.expose())?;
                if !matches!(
                    u.host_str(),
                    Some("hooks.slack.com" | "hooks.slack-gov.com")
                ) || !u.path().starts_with("/services/")
                {
                    return Err("Enter a Slack incoming webhook URL.");
                }
            }
            Self::Telegram { token, chat } => {
                if token.expose().len() < 10
                    || !token
                        .expose()
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-'))
                    || chat.is_empty()
                    || chat.len() > 64
                    || !chat
                        .trim_start_matches('-')
                        .bytes()
                        .all(|b| b.is_ascii_digit())
                {
                    return Err("Enter a Telegram bot token and numeric chat ID.");
                }
            }
            Self::Webhook { url, secret } => {
                secure_url(url.expose())?;
                if secret.expose().len() < 16 {
                    return Err("Use a webhook signing secret of at least 16 characters.");
                }
            }
        }
        Ok(())
    }
}
fn secure_url(raw: &str) -> Result<Url, &'static str> {
    let u = Url::parse(raw).map_err(|_| "Enter a valid HTTPS URL.")?;
    if u.scheme() != "https"
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
    {
        return Err("Use an HTTPS URL without user information or a fragment.");
    }
    Ok(u)
}

/// Signature authenticates the timestamp as well as the exact wire body.
pub fn signature(secret: &str, timestamp: i64, body: &[u8]) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    let bytes = mac.finalize().into_bytes();
    let mut hex = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(hex, "{b:02x}");
    }
    format!("sha256={hex}")
}

pub fn send(settings: Settings, payload: Value, now: i64) -> Result<(), &'static str> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .max_redirects(0)
        .build()
        .new_agent();
    send_with_agent(settings, payload, now, agent)
}

fn send_with_agent(
    settings: Settings,
    payload: Value,
    now: i64,
    agent: ureq::Agent,
) -> Result<(), &'static str> {
    settings.validate()?;
    let text = format!(
        "opsbox: {} — {}\n{}\n{}",
        payload["name"].as_str().unwrap_or("Alert"),
        payload["state"].as_str().unwrap_or("test"),
        payload["message"].as_str().unwrap_or("Test notification"),
        payload["link"].as_str().unwrap_or("#/alerts")
    );
    if let Settings::Email {
        host,
        port,
        username,
        password,
        from,
        to,
    } = settings
    {
        let message = Message::builder()
            .from(from.parse().map_err(|_| "Invalid sender.")?)
            .to(to.parse().map_err(|_| "Invalid recipient.")?)
            .subject("opsbox alert")
            .body(text)
            .map_err(|_| "Could not build email.")?;
        let builder = if port == 465 {
            SmtpTransport::relay(&host)
        } else {
            SmtpTransport::starttls_relay(&host)
        }
        .map_err(|_| "Could not configure SMTP TLS.")?;
        let transport = builder
            .port(port)
            .timeout(Some(Duration::from_secs(10)))
            .credentials(Credentials::new(username, password.expose().to_owned()))
            .build();
        transport
            .send(&message)
            .map_err(|_| "SMTP delivery failed. Check the host, credentials and recipient.")?;
        return Ok(());
    }
    let (url, body, key) = match settings {
        Settings::Slack { url } => (url, json!({"text":text}), None),
        Settings::Telegram { token, chat } => (
            Secret::new(format!(
                "https://api.telegram.org/bot{}/sendMessage",
                token.expose()
            )),
            json!({"chat_id":chat,"text":text}),
            None,
        ),
        Settings::Webhook { url, secret } => (url, payload, Some(secret)),
        Settings::Email { .. } => unreachable!(),
    };
    let body = serde_json::to_vec(&body).map_err(|_| "Could not build notification.")?;
    let mut request = agent
        .post(url.expose())
        .header("Content-Type", "application/json");
    if let Some(key) = key {
        request = request
            .header("X-Opsbox-Timestamp", now.to_string())
            .header("X-Opsbox-Signature", signature(key.expose(), now, &body));
    }
    let response = request
        .send(&body)
        .map_err(|_| "Delivery failed. Check the channel address and credentials.")?;
    if !response.status().is_success() {
        return Err("The channel refused the notification.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::{Arc, mpsc};

    fn receiver(
        status: &str,
    ) -> (
        String,
        ureq::Agent,
        mpsc::Receiver<String>,
        std::thread::JoinHandle<()>,
    ) {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.cert.der().clone()],
                rustls::pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()).into(),
            )
            .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "https://localhost:{}/notify",
            listener.local_addr().unwrap().port()
        );
        let roots = ureq::tls::RootCerts::new_with_certs(&[ureq::tls::Certificate::from_der(
            cert.cert.der().as_ref(),
        )
        .to_owned()]);
        let agent = ureq::Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(5)))
            .tls_config(ureq::tls::TlsConfig::builder().root_certs(roots).build())
            .build()
            .new_agent();
        let (tx, rx) = mpsc::channel();
        let status = status.to_owned();
        let handle = std::thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut stream = rustls::StreamOwned::new(
                rustls::ServerConnection::new(Arc::new(config)).unwrap(),
                socket,
            );
            let mut data = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = stream.read(&mut chunk).unwrap();
                assert!(n > 0);
                data.extend_from_slice(&chunk[..n]);
                if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&data[..pos]);
                    let size: usize = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|s| s.trim().parse().unwrap())
                        })
                        .unwrap();
                    if data.len() >= pos + 4 + size {
                        break;
                    }
                }
            }
            tx.send(String::from_utf8(data).unwrap()).unwrap();
            let body = b"ok";
            write!(stream,"HTTP/1.1 {status}\r\nContent-Length: {}\r\nLocation: https://example.com/stolen\r\nConnection: close\r\n\r\n",body.len()).unwrap();
            stream.write_all(body).unwrap();
            stream.flush().unwrap();
        });
        (url, agent, rx, handle)
    }

    #[test]
    fn signed_webhook_uses_exact_body_timestamp_and_rejects_redirect_and_failure() {
        for (status, success) in [
            ("200 OK", true),
            ("302 Found", false),
            ("503 Unavailable", false),
        ] {
            let (url, agent, rx, handle) = receiver(status);
            let payload =
                json!({"id":"disk:/:2","state":"firing","message":"Disk needs space: café"});
            let secret = "webhook-secret-value";
            let result = send_with_agent(
                Settings::Webhook {
                    url: Secret::new(url),
                    secret: Secret::new(secret),
                },
                payload.clone(),
                12345,
                agent,
            );
            assert_eq!(result.is_ok(), success);
            let raw = rx.recv().unwrap();
            handle.join().unwrap();
            let (head, body) = raw.split_once("\r\n\r\n").unwrap();
            assert_eq!(serde_json::from_str::<Value>(body).unwrap(), payload);
            let expected = signature(secret, 12345, body.as_bytes());
            assert!(
                head.to_ascii_lowercase()
                    .contains(&format!("x-opsbox-signature: {expected}"))
            );
            assert!(
                head.to_ascii_lowercase()
                    .contains("x-opsbox-timestamp: 12345")
            );
            assert_ne!(signature(secret, 12346, body.as_bytes()), expected);
            assert_ne!(signature(secret, 12345, b"changed"), expected);
            assert!(!raw.contains(secret));
        }
    }
    #[test]
    fn invalid_channel_settings_refuse_before_network_and_errors_never_include_secrets() {
        for url in [
            "http://example.com",
            "https://user:password@example.com",
            "https://example.com/#secret",
            "file:///etc/passwd",
        ] {
            let result = send(
                Settings::Webhook {
                    url: Secret::new(url),
                    secret: Secret::new("secret-value-12345"),
                },
                json!({}),
                0,
            );
            assert!(result.is_err());
            assert!(!result.unwrap_err().contains("secret-value"));
        }
        assert!(
            Settings::Email {
                host: "smtp.example.com".into(),
                port: 25,
                username: "user".into(),
                password: Secret::new("secret"),
                from: "a@example.com".into(),
                to: "b@example.com".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            Settings::Slack {
                url: Secret::new("https://evil.example/services/x")
            }
            .validate()
            .is_err()
        );
    }
}
