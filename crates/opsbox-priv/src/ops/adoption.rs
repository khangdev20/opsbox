//! Existing websites: read-only planning, durable receipts, bounded cutover and Undo.
//! No caller-supplied paths or command lines are executed.
use super::{Env, exec, network::*};
use opsbox_proto::{
    AdoptedDomain, AdoptedRoutes, ListenerAction, PortTarget, PrivError, PrivErrorCode, PrivOp,
    PrivReply, WebDiff, WebMove, WebPlan,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    net::{Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn refused(message: &str) -> PrivError {
    error(PrivErrorCode::Refused, message)
}
fn valid_domain(name: &str) -> bool {
    name.len() <= 253
        && name.contains('.')
        && name.split('.').all(|s| {
            !s.is_empty()
                && s.len() <= 63
                && !s.starts_with('-')
                && !s.ends_with('-')
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}
fn tool(program: &str, args: &[&str]) -> Result<String, PrivError> {
    let out = exec::run(program, args, Duration::from_secs(15)).ok_or_else(internal)?;
    if !out.ok() {
        return Err(refused(
            "The server configuration did not validate. No cutover was completed.",
        ));
    }
    Ok(out.combined())
}
fn inspect(env: &Env, id: &str) -> Result<Value, PrivError> {
    if !valid_container(id) {
        return Err(invalid());
    }
    let raw = command(&docker_bin(env), &["inspect", id])?;
    let v: Value = serde_json::from_str(&raw).map_err(|_| internal())?;
    let c = v[0].clone();
    if c["Id"] != id {
        return Err(invalid());
    }
    Ok(c)
}
fn revalidate(env: &Env, target: &PortTarget) -> Result<(), PrivError> {
    let owners = holders(env, &[80, 443])?;
    if owners.is_empty()
        || owners.iter().any(|h| match target {
            PortTarget::Container(id) => h.container.as_ref().is_none_or(|c| &c.id != id),
            PortTarget::Unit(unit) => h.container.is_some() || h.unit.as_ref() != Some(unit),
        })
    {
        return Err(invalid());
    }
    Ok(())
}
fn free_pair() -> Result<(u16, u16), PrivError> {
    for n in 0..100 {
        let http = 18080 + n;
        let https = 18443 + n;
        if let (Ok(_h), Ok(_s)) = (
            TcpListener::bind((Ipv4Addr::LOCALHOST, http)),
            TcpListener::bind((Ipv4Addr::LOCALHOST, https)),
        ) {
            return Ok((http, https));
        }
    }
    Err(refused("No private port pair is free."))
}
fn label(c: &Value, name: &str) -> Option<String> {
    c["Config"]["Labels"][name]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}
fn compose_args(plan: &WebPlan, override_file: Option<&str>, tail: &[&str]) -> Vec<String> {
    let mut args = vec![
        "compose".into(),
        "--project-directory".into(),
        plan.working_dir.clone().unwrap_or_default(),
        "-p".into(),
        plan.compose_project.clone().unwrap_or_default(),
    ];
    for file in &plan.config_files {
        args.extend(["-f".into(), file.clone()]);
    }
    if let Some(file) = override_file {
        args.extend(["-f".into(), file.into()]);
    }
    args.extend(tail.iter().map(|s| (*s).into()));
    args
}
fn docker_args(env: &Env, args: &[String]) -> Result<String, PrivError> {
    // Secrets stay in a root-only env file, never process argv, logs or stderr.
    use std::io::Write;
    let mut argv = Vec::new();
    let mut env_file = tempfile::NamedTempFile::new().map_err(|_| internal())?;
    let mut at = 0;
    let mut has_env = false;
    while at < args.len() {
        if args[at] == "--env" {
            let value = args.get(at + 1).ok_or_else(invalid)?;
            if value.contains(['\n', '\r']) {
                return Err(refused(
                    "Containers with multiline environment values need a manual recreation.",
                ));
            }
            writeln!(env_file, "{value}").map_err(|_| internal())?;
            has_env = true;
            at += 2;
        } else {
            argv.push(args[at].clone());
            at += 1;
        }
    }
    if has_env {
        argv.splice(
            1..1,
            [
                "--env-file".into(),
                env_file.path().to_string_lossy().into(),
            ],
        );
    }
    let out = exec::run(
        &docker_bin(env),
        &argv.iter().map(String::as_str).collect::<Vec<_>>(),
        Duration::from_secs(20),
    )
    .ok_or_else(internal)?;
    if !out.ok() {
        return Err(refused(
            "Docker could not apply the move. Review the project configuration; Undo details are retained.",
        ));
    }
    Ok(out.stdout.trim().into())
}

/// Compose !override replaces the port sequence rather than merging the old public bindings.
fn compose_override(service: &str, ports: &[Value], http: u16, https: u16) -> String {
    let mut out = format!(
        "services:\n  {}:\n    ports: !override\n",
        serde_json::to_string(service).expect("string")
    );
    for port in ports {
        let mut p = port.clone();
        match p["published"]
            .as_str()
            .and_then(|s| s.parse::<u16>().ok())
            .or_else(|| p["published"].as_u64().and_then(|n| u16::try_from(n).ok()))
        {
            Some(80) => {
                p["published"] = json!(http.to_string());
                p["host_ip"] = json!("127.0.0.1");
            }
            Some(443) => {
                p["published"] = json!(https.to_string());
                p["host_ip"] = json!("127.0.0.1");
            }
            _ => {}
        }
        out.push_str(&format!("      - {}\n", p));
    }
    out
}
/// Rewrite only actual listen tokens; preserve flags/comments and unrelated lines.
fn rewrite_listens(raw: &str, apache: bool, http: u16, https: u16) -> Result<String, PrivError> {
    let mut out: Vec<String> = Vec::new();
    let mut seen = std::collections::BTreeMap::<(usize, u16), usize>::new();
    let mut scope = vec![0usize];
    let mut next_scope = 0;
    for line in raw.split_inclusive('\n') {
        let trim = line.trim_start();
        let directive = if apache {
            trim.split_whitespace()
                .next()
                .filter(|s| s.eq_ignore_ascii_case("listen"))
                .unwrap_or("Listen")
        } else {
            "listen"
        };
        if let Some(rest) = trim
            .strip_prefix(directive)
            .filter(|s| s.starts_with(char::is_whitespace))
        {
            let token = rest
                .trim_start()
                .split(|c: char| c.is_whitespace() || c == ';')
                .next()
                .unwrap_or("");
            let port = token.rsplit(':').next().unwrap_or(token);
            if let Some(new) = match port {
                "80" => Some(http),
                "443" => Some(https),
                _ => None,
            } {
                let start = line.len() - rest.trim_start().len();
                let mut changed = format!(
                    "{}127.0.0.1:{new}{}",
                    &line[..start],
                    &line[start + token.len()..]
                );
                changed = changed
                    .replace(" ipv6only=on", "")
                    .replace(" ipv6only=off", "");
                let key = (*scope.last().unwrap_or(&0), new);
                if let Some(&index) = seen.get(&key) {
                    if !apache {
                        for flag in ["default_server", "ssl", "http2"] {
                            if changed
                                .split(|c: char| c.is_whitespace() || c == ';')
                                .any(|s| s == flag)
                                && !out[index]
                                    .split(|c: char| c.is_whitespace() || c == ';')
                                    .any(|s| s == flag)
                            {
                                out[index] = out[index].replacen(';', &format!(" {flag};"), 1);
                            }
                        }
                    }
                    continue;
                }
                seen.insert(key, out.len());
                out.push(changed);
                continue;
            }
        }
        // Nginx virtual hosts have independent listen sets. Ignore comment braces.
        if !apache {
            for c in line.split('#').next().unwrap_or("").chars() {
                if c == '{' {
                    next_scope += 1;
                    scope.push(next_scope);
                } else if c == '}' && scope.len() > 1 {
                    scope.pop();
                }
            }
        }
        out.push(line.into());
    }
    Ok(out.concat())
}

fn server_kind(unit: &str) -> &str {
    unit.strip_suffix(".service").unwrap_or(unit)
}
fn unit_config(kind: &str) -> Result<String, PrivError> {
    match kind {
        "nginx" => tool("/usr/sbin/nginx", &["-T"]),
        "apache2" | "httpd" => tool(apache_bin(), &["-S"]),
        "caddy" => fs::read_to_string("/etc/caddy/Caddyfile").map_err(|_| internal()),
        "traefik" => fs::read_to_string("/etc/traefik/traefik.yml")
            .or_else(|_| fs::read_to_string("/etc/traefik/traefik.toml"))
            .map_err(|_| internal()),
        _ => Ok(String::new()),
    }
}
fn apache_bin() -> &'static str {
    if Path::new("/usr/sbin/apachectl").exists() {
        "/usr/sbin/apachectl"
    } else {
        "/usr/sbin/apache2ctl"
    }
}
fn domains_from(text: &str, kind: &str) -> Vec<String> {
    let mut names = BTreeSet::new();
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('#') {
            continue;
        }
        let candidates = if kind == "nginx" {
            l.strip_prefix("server_name ")
                .unwrap_or("")
                .split(';')
                .next()
                .unwrap_or("")
                .to_owned()
        } else if kind == "apache2" || kind == "httpd" {
            l.split_whitespace()
                .collect::<Vec<_>>()
                .windows(2)
                .filter(|w| ["namevhost", "alias", "server"].contains(&w[0]))
                .map(|w| w[1])
                .collect::<Vec<_>>()
                .join(" ")
        } else if kind == "caddy" && l.contains('{') && !l.starts_with('@') {
            l.split('{').next().unwrap_or("").to_owned()
        } else {
            String::new()
        };
        for name in candidates.split(|c: char| c.is_whitespace() || c == ',') {
            let name = name
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .split(':')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if valid_domain(&name) {
                names.insert(name);
            }
        }
    }
    // Traefik rules and Caddy JSON admin responses contain quoted hostnames.
    for part in text.split("Host(").skip(1) {
        for candidate in part.split(')').next().unwrap_or("").split(',') {
            let name = candidate
                .trim()
                .trim_matches(['`', '"', '\''])
                .to_ascii_lowercase();
            if valid_domain(&name) {
                names.insert(name);
            }
        }
    }
    names.into_iter().collect()
}
fn caddy_hosts(v: &Value, names: &mut Vec<String>) {
    if let Some(a) = v.as_array() {
        for v in a {
            caddy_hosts(v, names);
        }
    }
    if let Some(o) = v.as_object() {
        for (k, v) in o {
            if k == "host" {
                for s in v.as_array().into_iter().flatten().filter_map(Value::as_str) {
                    if valid_domain(s) {
                        names.push(s.to_ascii_lowercase());
                    }
                }
            } else {
                caddy_hosts(v, names);
            }
        }
    }
}
fn config_files(raw: &str, kind: &str) -> Result<Vec<PathBuf>, PrivError> {
    let mut files = BTreeSet::new();
    if kind == "nginx" {
        for line in raw
            .lines()
            .filter_map(|l| l.strip_prefix("# configuration file "))
        {
            if let Some(path) = line.strip_suffix(':') {
                files.insert(PathBuf::from(path));
            }
        }
    } else if ["apache2", "httpd"].contains(&kind) {
        let root = if kind == "apache2" {
            "/etc/apache2"
        } else {
            "/etc/httpd"
        };
        collect_files(Path::new(root), &mut files, 0);
        files.retain(|p| p.extension().is_some_and(|s| s == "conf"));
    } else if kind == "caddy" {
        files.insert("/etc/caddy/Caddyfile".into());
    } else if kind == "traefik" {
        files.insert(
            if Path::new("/etc/traefik/traefik.yml").exists() {
                "/etc/traefik/traefik.yml"
            } else {
                "/etc/traefik/traefik.toml"
            }
            .into(),
        );
    }
    let mut result = BTreeSet::new();
    for p in files {
        let p = fs::canonicalize(p).map_err(|_| internal())?;
        if !p.starts_with("/etc")
            || !fs::metadata(&p).is_ok_and(|m| m.is_file() && m.len() < 1024 * 1024)
        {
            return Err(invalid());
        }
        result.insert(p);
    }
    Ok(result.into_iter().collect())
}
fn collect_files(dir: &Path, files: &mut BTreeSet<PathBuf>, depth: usize) {
    if depth > 8 || files.len() >= 2048 {
        return;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() && !entry.file_type().is_ok_and(|t| t.is_symlink()) {
                collect_files(&p, files, depth + 1);
            } else if p.is_file() {
                files.insert(p);
            }
        }
    }
}
fn rewrite_caddy(raw: &str, http: u16, https: u16) -> Result<String, PrivError> {
    // Existing global options are preserved. Add loopback binding to each site so the
    // moved listener can never expose an unauthenticated public alternate port.
    let mut out = String::new();
    let mut depth = 0i32;
    let mut global = false;
    for line in raw.lines() {
        let l = line.trim();
        if l.starts_with('#') {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if depth == 0 && l == "{" {
            global = true;
            out.push_str("{\n");
            out.push_str(&format!(
                "    http_port {http}\n    https_port {https}\n    default_bind 127.0.0.1\n"
            ));
        } else if global
            && depth == 1
            && ["http_port", "https_port", "default_bind"]
                .iter()
                .any(|k| l.split_whitespace().next() == Some(k))
        {
        } else {
            out.push_str(line);
            out.push('\n');
        }
        depth += l.matches('{').count() as i32 - l.matches('}').count() as i32;
    }
    if !global {
        out = format!(
            "{{\n    http_port {http}\n    https_port {https}\n    default_bind 127.0.0.1\n}}\n{out}"
        );
    }
    if raw
        .lines()
        .any(|l| l.trim().starts_with("bind ") || l.trim().starts_with("import "))
    {
        return Err(refused(
            "Review Caddy bind/import directives manually before moving this server.",
        ));
    }
    Ok(out)
}
fn traefik_document(raw: &str) -> Result<Value, PrivError> {
    if raw.trim_start().starts_with('[') {
        toml::from_str(raw).map_err(|_| refused("Traefik static TOML is invalid."))
    } else {
        serde_saphyr::from_str(raw).map_err(|_| refused("Traefik static YAML is invalid."))
    }
}
fn rewrite_traefik(raw: &str, http: u16, https: u16) -> Result<String, PrivError> {
    let before = traefik_document(raw)?;
    let mut expected = before.clone();
    let mut replacements = Vec::new();
    let entries = expected["entryPoints"]
        .as_object_mut()
        .ok_or_else(|| refused("Traefik entryPoints are missing."))?;
    for entry in entries.values_mut() {
        if let Some(address) = entry["address"].as_str() {
            let port = match address.rsplit(':').next() {
                Some("80") => http,
                Some("443") => https,
                _ => continue,
            };
            let replacement = format!("127.0.0.1:{port}");
            replacements.push((address.to_owned(), replacement.clone()));
            entry["address"] = json!(replacement);
        }
    }
    if replacements.len() != 2 {
        return Err(refused(
            "Traefik must have one HTTP and one HTTPS entrypoint. Review other layouts manually.",
        ));
    }
    let mut out = String::new();
    for line in raw.split_inclusive('\n') {
        let trim = line.trim_start();
        let mut next = line.to_owned();
        if trim.starts_with("address:") || trim.starts_with("address =") {
            for (old, new) in &replacements {
                for quote in ['"', '\''] {
                    next = next.replace(
                        &format!("{quote}{old}{quote}"),
                        &format!("{quote}{new}{quote}"),
                    );
                }
                if trim.starts_with("address:") {
                    let token = trim
                        .strip_prefix("address:")
                        .unwrap_or("")
                        .split_whitespace()
                        .next()
                        .unwrap_or("");
                    if token == old {
                        next = line.replacen(token, new, 1);
                    }
                }
            }
        }
        out.push_str(&next);
    }
    if traefik_document(&out)? != expected {
        return Err(refused(
            "Traefik addresses need separate address lines. Review compact or aliased layouts manually.",
        ));
    }
    Ok(out)
}

fn probe(domain: &str, port: u16, tls: bool, seconds: u8) -> Result<u16, PrivError> {
    if !valid_domain(domain) {
        return Err(invalid());
    }
    let scheme = if tls { "https" } else { "http" };
    let resolve = format!("{domain}:{port}:127.0.0.1");
    let url = format!("{scheme}://{domain}:{port}/");
    let out = command(
        "/usr/bin/curl",
        &[
            "--silent",
            "--insecure",
            "--noproxy",
            "*",
            "--max-time",
            &seconds.to_string(),
            "--resolve",
            &resolve,
            "--output",
            "/dev/null",
            "--write-out",
            "%{http_code}",
            "--header",
            &format!("Host: {domain}"),
            &url,
        ],
    )?;
    let status = out.parse::<u16>().map_err(|_| internal())?;
    if !(100..600).contains(&status) {
        return Err(refused(
            "The existing website did not answer the local check.",
        ));
    }
    Ok(status)
}
struct Certificate {
    cert: String,
    key: Option<String>,
    expiry: i64,
    before: i64,
}
fn cert_metadata(cert: &str, key: Option<String>, domain: &str) -> Option<Certificate> {
    use rustls_pki_types::{CertificateDer, pem::PemObject};
    use x509_parser::prelude::*;
    let der = CertificateDer::from_pem_slice(cert.as_bytes()).ok()?;
    let (_, parsed) = X509Certificate::from_der(der.as_ref()).ok()?;
    let names = parsed.subject_alternative_name().ok()??;
    let matches = names.value.general_names.iter().any(|n| {
        if let GeneralName::DNSName(name) = n {
            name.eq_ignore_ascii_case(domain)
                || name.strip_prefix("*.").is_some_and(|suffix| {
                    domain
                        .split_once('.')
                        .is_some_and(|(_, s)| s.eq_ignore_ascii_case(suffix))
                })
        } else {
            false
        }
    });
    matches.then(|| Certificate {
        cert: cert.into(),
        key,
        expiry: parsed.validity().not_after.timestamp(),
        before: parsed.validity().not_before.timestamp(),
    })
}
fn find_cert(env: &Env, target: &PortTarget, domain: &str, with_key: bool) -> Option<Certificate> {
    let mut roots = Vec::new();
    let mut container = None;
    if let PortTarget::Container(id) = target {
        if let Ok(c) = inspect(env, id) {
            for m in c["Mounts"].as_array().into_iter().flatten() {
                let dest = m["Destination"].as_str().unwrap_or("");
                if ["letsencrypt", "caddy", "traefik"]
                    .iter()
                    .any(|s| dest.contains(s))
                    && let Some(p) = m["Source"].as_str()
                {
                    roots.push(PathBuf::from(p));
                }
            }
        }
        container = Some(id);
    }
    if let Some(c) = find_cert_in(domain, with_key, &roots) {
        return Some(c);
    }
    if let Some(id) = container {
        let mut paths = vec![
            (
                format!("/etc/letsencrypt/live/{domain}/fullchain.pem"),
                format!("/etc/letsencrypt/live/{domain}/privkey.pem"),
            ),
            (
                format!("/etc/nginx/{domain}.crt"),
                format!("/etc/nginx/{domain}.key"),
            ),
        ];
        if let Ok(raw) = command(
            &docker_bin(env),
            &[
                "exec",
                id,
                "find",
                "/data/caddy/certificates",
                "-maxdepth",
                "5",
                "-type",
                "f",
                "-name",
                "*.crt",
            ],
        ) {
            for path in raw.lines().take(256).filter(|s| {
                s.starts_with("/data/caddy/certificates/") && !s.contains("..") && s.len() < 1024
            }) {
                paths.push((
                    path.into(),
                    Path::new(path)
                        .with_extension("key")
                        .to_string_lossy()
                        .into(),
                ));
            }
        }
        for (cert, key) in paths {
            if let Ok(pem) = command(&docker_bin(env), &["exec", id, "cat", &cert]) {
                let key = if with_key {
                    command(&docker_bin(env), &["exec", id, "cat", &key]).ok()
                } else {
                    None
                };
                if let Some(c) = cert_metadata(&pem, key, domain) {
                    return Some(c);
                }
            }
        }
    }
    None
}
fn find_cert_in(domain: &str, with_key: bool, extra_roots: &[PathBuf]) -> Option<Certificate> {
    // Search only standard certificate stores. The public API sees expiry, never PEMs.
    let mut files = BTreeSet::new();
    for root in [
        "/etc/letsencrypt/live",
        "/var/lib/caddy/.local/share/caddy",
        "/var/lib/caddy",
        "/etc/caddy",
        "/etc/traefik",
        "/var/lib/traefik",
    ] {
        collect_files(Path::new(root), &mut files, 0);
    }
    for root in extra_roots {
        if root.is_file() {
            files.insert(root.clone());
        } else {
            collect_files(root, &mut files, 0);
        }
    }
    for path in files {
        if !fs::metadata(&path).is_ok_and(|m| m.len() < 4 * 1024 * 1024) {
            continue;
        }
        let name = path.file_name()?.to_str()?;
        if name == "fullchain.pem" || path.extension().is_some_and(|e| e == "crt") {
            let Ok(cert) = fs::read_to_string(&path) else {
                continue;
            };
            let key = if with_key {
                fs::read_to_string(if name == "fullchain.pem" {
                    path.with_file_name("privkey.pem")
                } else {
                    path.with_extension("key")
                })
                .ok()
            } else {
                None
            };
            if let Some(c) = cert_metadata(&cert, key, domain) {
                return Some(c);
            }
        } else if name == "acme.json" {
            use base64::Engine;
            if let Ok(raw) = fs::read_to_string(&path)
                && let Ok(v) = serde_json::from_str::<Value>(&raw)
            {
                for resolver in v.as_object().into_iter().flat_map(|m| m.values()) {
                    for c in resolver["Certificates"].as_array().into_iter().flatten() {
                        let decode = |key: &str| {
                            c[key]
                                .as_str()
                                .and_then(|s| {
                                    base64::engine::general_purpose::STANDARD.decode(s).ok()
                                })
                                .and_then(|b| String::from_utf8(b).ok())
                        };
                        if let Some(cert) = decode("certificate")
                            && let Some(c) = cert_metadata(
                                &cert,
                                if with_key { decode("key") } else { None },
                                domain,
                            )
                        {
                            return Some(c);
                        }
                    }
                }
            }
        }
    }
    None
}
fn discover(env: &Env, target: &PortTarget) -> Result<WebPlan, PrivError> {
    revalidate(env, target)?;
    let (http, https) = free_pair()?;
    let mut plan=WebPlan {target:target.clone(), holder:String::new(), image:None, compose_project:None, working_dir:None, config_files:vec![], routes:AdoptedRoutes {http_port:http,https_port:https,..Default::default()}, changes:vec![], supported:true, explanation:"The old server keeps its certificates. opsbox forwards its HTTP and TLS traffic through private ports. Undo restores the original ports.".into(), confirmation:"move".into()};
    let mut names = Vec::new();
    match target {
        PortTarget::Container(id) => {
            let c = inspect(env, id)?;
            plan.holder = c["Name"]
                .as_str()
                .unwrap_or("")
                .trim_start_matches('/')
                .into();
            plan.image = c["Config"]["Image"].as_str().map(str::to_owned);
            plan.compose_project = label(&c, "com.docker.compose.project");
            plan.working_dir = label(&c, "com.docker.compose.project.working_dir");
            if let Some(files) = label(&c, "com.docker.compose.project.config_files") {
                plan.config_files = files.split(',').map(str::to_owned).collect();
            }
            // Read-only commands inside a container; never infer an executable from its name.
            for (kind, cmd) in [
                ("nginx", vec!["nginx", "-T"]),
                ("apache2", vec!["apachectl", "-S"]),
                ("caddy", vec!["cat", "/etc/caddy/Caddyfile"]),
            ] {
                let mut args = vec!["exec", id];
                args.extend(cmd);
                if let Ok(raw) = tool(&docker_bin(env), &args) {
                    names.extend(domains_from(&raw, kind));
                }
            }
            if let Some(labels) = c["Config"]["Labels"].as_object() {
                for (k, v) in labels {
                    if k.starts_with("traefik.http.routers.") && k.ends_with(".rule") {
                        names.extend(domains_from(v.as_str().unwrap_or(""), "traefik"));
                    }
                }
            }
            if plan.compose_project.is_some() {
                if plan.config_files.is_empty()
                    || plan
                        .working_dir
                        .as_ref()
                        .is_none_or(|p| !Path::new(p).is_absolute())
                    || plan
                        .config_files
                        .iter()
                        .any(|p| !Path::new(p).is_absolute() || !Path::new(p).is_file())
                {
                    return Err(refused(
                        "Compose labels do not identify existing absolute config files and a working directory.",
                    ));
                }
                let service = label(&c, "com.docker.compose.service").ok_or_else(invalid)?;
                if service.starts_with('-')
                    || service.is_empty()
                    || service.len() > 128
                    || !service
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
                {
                    return Err(invalid());
                }
                let raw = docker_args(
                    env,
                    &compose_args(&plan, None, &["config", "--format", "json"]),
                )?;
                let config: Value = serde_json::from_str(&raw).map_err(|_| internal())?;
                let ports = config["services"][&service]["ports"]
                    .as_array()
                    .ok_or_else(invalid)?;
                let path = Path::new(&plan.config_files[0])
                    .with_file_name("docker-compose.opsbox.override.yml");
                if path.exists() {
                    return Err(refused(
                        "An opsbox Compose override already exists. Review or undo the earlier move first.",
                    ));
                }
                plan.changes.push(WebDiff {
                    path: path.to_string_lossy().into(),
                    before: String::new(),
                    after: compose_override(&service, ports, http, https),
                });
            } else {
                plan.confirmation = format!("recreate {}", plan.holder);
                let args = recreate_args(&c, http, https)?;
                plan.changes.push(WebDiff {
                    path: "docker run (environment and label values kept private)".into(),
                    before: run_preview(&c, &args, false),
                    after: format!(
                        "{}\nSnapshot the writable layer; keep {}-pre-opsbox stopped for Undo.",
                        run_preview(&c, &args, true),
                        plan.holder
                    ),
                });
            }
        }
        PortTarget::Unit(unit) => {
            plan.holder = unit.clone();
            let kind = server_kind(unit);
            let raw = unit_config(kind)?;
            names = domains_from(&raw, kind);
            if kind == "caddy"
                && names.is_empty()
                && let Ok(raw) = command(
                    "/usr/bin/curl",
                    &[
                        "--silent",
                        "--fail",
                        "--noproxy",
                        "*",
                        "--max-time",
                        "2",
                        "http://127.0.0.1:2019/config/",
                    ],
                )
                && let Ok(v) = serde_json::from_str::<Value>(&raw)
            {
                caddy_hosts(&v, &mut names);
            }
            for path in config_files(&raw, kind)? {
                let before = fs::read_to_string(&path).map_err(|_| internal())?;
                let after = match kind {
                    "nginx" => rewrite_listens(&before, false, http, https)?,
                    "apache2" | "httpd" => rewrite_listens(&before, true, http, https)?,
                    "caddy" => rewrite_caddy(&before, http, https)?,
                    "traefik" => rewrite_traefik(&before, http, https)?,
                    _ => before.clone(),
                };
                if before != after {
                    plan.changes.push(WebDiff {
                        path: path.to_string_lossy().into(),
                        before,
                        after,
                    });
                }
            }
            if ["apache2", "httpd"].contains(&kind) {
                // A port-qualified VirtualHost cannot follow a Listen-only move.
                // Refuse rather than changing forbidden non-listen directives.
                let has_port_vhosts = config_files(&raw, kind)?
                    .iter()
                    .filter_map(|p| fs::read_to_string(p).ok())
                    .any(|s| {
                        s.lines().any(|l| {
                            l.trim_start()
                                .to_ascii_lowercase()
                                .starts_with("<virtualhost")
                                && (l.contains(":80") || l.contains(":443"))
                        })
                    });
                if has_port_vhosts {
                    plan.supported = false;
                    plan.explanation="Apache uses port-qualified VirtualHost directives. A Listen-only move would send some sites to the wrong virtual host. Change those bindings manually before moving it; the existing Keep and Stop choices remain available.".into();
                }
            }
            if plan.changes.is_empty() {
                plan.supported = false;
                plan.explanation="This server cannot be moved automatically. Keep using other ports, or stop it with the existing takeover action.".into();
            }
        }
    }
    names.sort();
    names.dedup();
    for domain in names.into_iter().take(256) {
        let cert = find_cert(env, target, &domain, false);
        plan.routes.domains.push(AdoptedDomain {
            has_certificate: cert.is_some(),
            certificate_expiry: cert.map(|c| c.expiry.to_string()),
            reachable_now: probe(&domain, 80, false, 2).is_ok(),
            served_by: plan.holder.clone(),
            domain,
            terminate_tls: false,
        });
    }
    plan.routes.everything_else = plan.routes.domains.is_empty();
    Ok(plan)
}
/// Review-only command text. These strings are never executed.
fn run_preview(c: &Value, args: &[String], rebound: bool) -> String {
    let mut shown = vec!["docker".to_owned(), "run".into(), "-d".into()];
    if !rebound && let Some(ports) = c["HostConfig"]["PortBindings"].as_object() {
        for (target, bindings) in ports {
            for b in bindings.as_array().into_iter().flatten() {
                let ip = b["HostIp"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("0.0.0.0");
                let ip = if ip.contains(':') {
                    format!("[{ip}]")
                } else {
                    ip.into()
                };
                shown.extend([
                    "--publish".into(),
                    format!("{ip}:{}:{target}", b["HostPort"].as_str().unwrap_or("")),
                ]);
            }
        }
    }
    let mut at = 1;
    while at < args.len() {
        if args[at] == "--publish" && !rebound {
            at += 2;
            continue;
        }
        shown.push(args[at].clone());
        if ["--env", "--label"].contains(&args[at].as_str()) {
            at += 1;
            let key = args
                .get(at)
                .and_then(|s| s.split_once('='))
                .map_or("value", |(key, _)| key);
            shown.push(format!("{key}=<kept private>"));
        }
        at += 1;
    }
    shown
        .iter()
        .map(|s| format!("'{}'", s.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ")
}
/// Recreate only inspect shapes we can preserve faithfully; reject special devices,
/// host networking, extra capabilities, anonymous volumes and custom security options.
fn recreate_args(c: &Value, http: u16, https: u16) -> Result<Vec<String>, PrivError> {
    let hc = &c["HostConfig"];
    let nonempty = |key: &str| hc[key].as_array().is_some_and(|a| !a.is_empty());
    if hc["Privileged"].as_bool() == Some(true)
        || [
            "Devices",
            "CapAdd",
            "CapDrop",
            "SecurityOpt",
            "VolumesFrom",
            "Links",
        ]
        .iter()
        .any(|k| nonempty(k))
        || hc["AutoRemove"].as_bool() == Some(true)
        || c["Mounts"].as_array().is_some_and(|a| {
            a.iter()
                .any(|m| !matches!(m["Type"].as_str(), Some("bind" | "volume")))
        })
    {
        return Err(refused(
            "This container has settings that cannot be safely recreated. Use a Compose project or change its ports manually.",
        ));
    }
    let network = hc["NetworkMode"].as_str().unwrap_or("default");
    if ["host", "none"].contains(&network)
        || network.starts_with("container:")
        || c["NetworkSettings"]["Networks"]
            .as_object()
            .is_some_and(|m| m.len() > 1)
    {
        return Err(refused(
            "Containers with host, shared, or multiple networks need a manual port change.",
        ));
    }
    let name = c["Name"]
        .as_str()
        .ok_or_else(invalid)?
        .trim_start_matches('/');
    let mut args = vec![
        "create".into(),
        "--name".into(),
        name.into(),
        "--network".into(),
        network.into(),
    ];
    for (key, flag) in [
        ("Dns", "--dns"),
        ("DnsOptions", "--dns-option"),
        ("DnsSearch", "--dns-search"),
        ("ExtraHosts", "--add-host"),
        ("GroupAdd", "--group-add"),
    ] {
        for val in hc[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            args.extend([flag.into(), val.into()]);
        }
    }
    for (key, flag) in [
        ("Hostname", "--hostname"),
        ("Domainname", "--domainname"),
        ("StopSignal", "--stop-signal"),
    ] {
        if let Some(val) = c["Config"][key].as_str().filter(|s| !s.is_empty()) {
            args.extend([flag.into(), val.into()]);
        }
    }
    if let Some(log) = hc["LogConfig"]["Type"].as_str().filter(|s| !s.is_empty()) {
        args.extend(["--log-driver".into(), log.into()]);
        if let Some(options) = hc["LogConfig"]["Config"].as_object() {
            for (k, v) in options {
                args.extend([
                    "--log-opt".into(),
                    format!("{k}={}", v.as_str().unwrap_or("")),
                ]);
            }
        }
    }
    if let Some(networks) = c["NetworkSettings"]["Networks"].as_object() {
        for n in networks.values() {
            if n["IPAMConfig"].as_object().is_some_and(|o| {
                o.values()
                    .any(|v| v.as_str().is_some_and(|s| !s.is_empty()))
            }) {
                return Err(refused(
                    "A container with a static IP needs a manual recreation.",
                ));
            }
            if network != "bridge" && network != "default" {
                for alias in n["Aliases"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    args.extend(["--network-alias".into(), alias.into()]);
                }
            }
        }
    }
    for u in hc["Ulimits"].as_array().into_iter().flatten() {
        args.extend([
            "--ulimit".into(),
            format!(
                "{}={}:{}",
                u["Name"].as_str().ok_or_else(invalid)?,
                u["Soft"].as_i64().ok_or_else(invalid)?,
                u["Hard"].as_i64().ok_or_else(invalid)?
            ),
        ]);
    }
    for (key, flag) in [("User", "--user"), ("WorkingDir", "--workdir")] {
        if let Some(s) = c["Config"][key].as_str().filter(|s| !s.is_empty()) {
            args.extend([flag.into(), s.into()]);
        }
    }
    for s in c["Config"]["Env"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        args.extend(["--env".into(), s.into()]);
    }
    if let Some(labels) = c["Config"]["Labels"].as_object() {
        for (k, v) in labels {
            args.extend([
                "--label".into(),
                format!("{k}={}", v.as_str().unwrap_or("")),
            ]);
        }
    }
    for m in c["Mounts"].as_array().into_iter().flatten() {
        let source = if m["Type"] == "volume" {
            m["Name"].as_str()
        } else {
            m["Source"].as_str()
        }
        .ok_or_else(invalid)?;
        let target = m["Destination"].as_str().ok_or_else(invalid)?;
        if source.contains(',') || target.contains(',') {
            return Err(invalid());
        }
        args.extend([
            "--mount".into(),
            format!(
                "type={},source={source},target={target}{}",
                m["Type"].as_str().unwrap_or(""),
                if m["RW"] == false { ",readonly" } else { "" }
            ),
        ]);
    }
    let ports = hc["PortBindings"].as_object().ok_or_else(invalid)?;
    for (target, bindings) in ports {
        for b in bindings.as_array().into_iter().flatten() {
            let published = b["HostPort"].as_str().ok_or_else(invalid)?;
            let (ip, p) = match published {
                "80" => ("127.0.0.1".into(), http.to_string()),
                "443" => ("127.0.0.1".into(), https.to_string()),
                _ => (
                    b["HostIp"].as_str().unwrap_or("0.0.0.0").to_owned(),
                    published.into(),
                ),
            };
            let ip = if ip.contains(':') {
                format!("[{ip}]")
            } else {
                ip
            };
            let binding = format!("{ip}:{p}:{target}");
            if !args
                .windows(2)
                .any(|w| w[0] == "--publish" && w[1] == binding)
            {
                args.extend(["--publish".into(), binding]);
            }
        }
    }
    let restart = hc["RestartPolicy"]["Name"].as_str().unwrap_or("no");
    let max = hc["RestartPolicy"]["MaximumRetryCount"]
        .as_u64()
        .unwrap_or(0);
    args.extend([
        "--restart".into(),
        if restart == "on-failure" && max > 0 {
            format!("on-failure:{max}")
        } else {
            restart.into()
        },
    ]);
    for (key, flag) in [
        ("Memory", "--memory"),
        ("MemorySwap", "--memory-swap"),
        ("NanoCpus", "--cpus"),
        ("PidsLimit", "--pids-limit"),
        ("ShmSize", "--shm-size"),
    ] {
        if let Some(n) = hc[key].as_i64().filter(|n| *n != 0) {
            args.extend([
                flag.into(),
                if key == "NanoCpus" {
                    format!("{}", n as f64 / 1e9)
                } else {
                    n.to_string()
                },
            ]);
        }
    }
    if hc["ReadonlyRootfs"] == true {
        args.push("--read-only".into());
    }
    // Pin the image ID, avoiding an unexpected image update during the cutover.
    let entry = c["Config"]["Entrypoint"].as_array();
    if let Some(first) = entry.and_then(|a| a.first()).and_then(Value::as_str) {
        args.extend(["--entrypoint".into(), first.into()]);
    }
    args.push(c["Image"].as_str().ok_or_else(invalid)?.into());
    if let Some(entry) = entry {
        args.extend(
            entry
                .iter()
                .skip(1)
                .filter_map(Value::as_str)
                .map(str::to_owned),
        );
    }
    args.extend(
        c["Config"]["Cmd"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned),
    );
    Ok(args)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedFile {
    path: String,
    backup: String,
    before: Option<String>,
    after: String,
    mode: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Receipt {
    public: WebMove,
    plan: WebPlan,
    files: Vec<SavedFile>,
    service: Option<String>,
    recreate: Vec<String>,
    old_name: Option<String>,
    new_id: Option<String>,
    #[serde(default)]
    snapshot: Option<String>,
    #[serde(default)]
    staged_name: Option<String>,
    baseline: Vec<(String, u16, u16)>,
    previous_routes: AdoptedRoutes,
    mutated: bool,
    #[serde(default)]
    old_restart: String,
}
fn receipt_dir(env: &Env) -> PathBuf {
    env.config_path.with_file_name("web-moves")
}
fn receipt_path(env: &Env, id: &str) -> Result<PathBuf, PrivError> {
    if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    Ok(receipt_dir(env).join(format!("{id}.json")))
}
fn save(env: &Env, r: &Receipt) -> Result<(), PrivError> {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(receipt_dir(env)).map_err(|_| internal())?;
    fs::set_permissions(receipt_dir(env), fs::Permissions::from_mode(0o700))
        .map_err(|_| internal())?;
    atomic_write(
        &receipt_path(env, &r.public.id)?,
        &serde_json::to_vec(r).map_err(|_| internal())?,
        0o600,
    )
}
fn read_receipt(env: &Env, id: &str) -> Result<Receipt, PrivError> {
    serde_json::from_slice(&fs::read(receipt_path(env, id)?).map_err(|_| internal())?)
        .map_err(|_| internal())
}
fn all_receipts(env: &Env) -> Result<Vec<WebMove>, PrivError> {
    let entries = match fs::read_dir(receipt_dir(env)) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(_) => return Err(internal()),
    };
    let mut rows = Vec::new();
    for e in entries.flatten() {
        if let Ok(raw) = fs::read(e.path())
            && let Ok(r) = serde_json::from_slice::<Receipt>(&raw)
        {
            rows.push(r.public);
        }
    }
    Ok(rows)
}
fn step(env: &Env, r: &mut Receipt, message: &str) -> Result<(), PrivError> {
    r.public.steps.push(message.into());
    if r.public.steps.len() > 100 {
        r.public.steps.remove(0);
    }
    save(env, r)
}
fn validate_unit(unit: &str) -> Result<(), PrivError> {
    match server_kind(unit) {
        "nginx" => tool("/usr/sbin/nginx", &["-t"]),
        "apache2" | "httpd" => tool(apache_bin(), &["configtest"]),
        "caddy" => tool(
            "/usr/bin/caddy",
            &[
                "validate",
                "--config",
                "/etc/caddy/Caddyfile",
                "--adapter",
                "caddyfile",
            ],
        ),
        // Traefik has no offline validate subcommand. Parse the complete static
        // document, compare only entrypoint changes, then verify the live listeners.
        "traefik" => {
            traefik_document(&unit_config("traefik")?)?;
            Ok(String::new())
        }
        _ => return Err(invalid()),
    }
    .map(|_| ())
}
fn adopted(env: &Env) -> Result<AdoptedRoutes, PrivError> {
    serde_json::from_value(control(env, json!({"type":"status"}))?["adopted"].clone())
        .map_err(|_| internal())
}
fn load_routes(env: &Env, routes: &AdoptedRoutes) -> Result<(), PrivError> {
    control(env, json!({"type":"set_adopted","args":{"routes":routes}})).map(|_| ())
}
fn rebind(env: &Env, r: &mut Receipt) -> Result<(), PrivError> {
    // Persist intent BEFORE any filesystem/container mutation, so Undo survives failures.
    r.mutated = true;
    save(env, r)?;
    for file in &r.files {
        if let Some(before) = &file.before {
            atomic_write(Path::new(&file.backup), before.as_bytes(), 0o600)?;
        }
        atomic_write(Path::new(&file.path), file.after.as_bytes(), file.mode)?;
    }
    match &r.plan.target {
        PortTarget::Unit(unit) => {
            validate_unit(unit)?;
            command(&systemctl_bin(env), &["reload-or-restart", unit])?;
        }
        PortTarget::Container(id) => {
            if let Some(service) = &r.service {
                let path = &r.files[0].path;
                docker_args(
                    env,
                    &compose_args(&r.plan, Some(path), &["config", "--quiet"]),
                )?;
                docker_args(
                    env,
                    &compose_args(&r.plan, Some(path), &["up", "-d", "--no-deps", service]),
                )?;
            } else {
                let old = r.old_name.clone().ok_or_else(invalid)?;
                // Name conflict is detected before stopping the old site.
                if command(&docker_bin(env), &["inspect", &old]).is_ok() {
                    return Err(refused("The pre-opsbox container name is already in use."));
                }
                // Snapshot writable-layer config/certificates before stopping anything.
                // Volumes remain the same mounts; the original container is retained.
                let output = command(&docker_bin(env), &["commit", "--pause=false", id])?;
                // Docker 29 can print a deprecation notice on stdout before the ID.
                let snapshot = output
                    .lines()
                    .find(|s| {
                        s.strip_prefix("sha256:").is_some_and(|id| {
                            id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
                        })
                    })
                    .ok_or_else(internal)?
                    .to_owned();
                r.snapshot = Some(snapshot.clone());
                let staged = format!("{}-opsbox-{}", r.plan.holder, &r.public.id[..8]);
                r.staged_name = Some(staged.clone());
                save(env, r)?;
                let mut args = r.recreate.clone();
                args[2] = staged;
                let image = args
                    .iter()
                    .position(|s| s.starts_with("sha256:"))
                    .ok_or_else(invalid)?;
                args[image] = snapshot;
                args.splice(
                    1..1,
                    ["--label".into(), format!("opsbox.web-move={}", r.public.id)],
                );
                let new = docker_args(env, &args)?;
                if !valid_container(new.trim()) {
                    return Err(internal());
                }
                r.new_id = Some(new.trim().into());
                save(env, r)?;
                command(&docker_bin(env), &["start", new.trim()])?;
                // Warm the private listeners before releasing the public sockets.
                verify(
                    &r.baseline,
                    r.public.http_port,
                    r.public.https_port,
                    Duration::from_secs(20),
                )?;
                command(&docker_bin(env), &["update", "--restart=no", id])?;
                // Cosmetic renames happen while the original listener is still live.
                // Stopping it is the last command before the immediate proxy bind.
                command(&docker_bin(env), &["rename", id, &old])?;
                command(&docker_bin(env), &["rename", new.trim(), &r.plan.holder])?;
                command(&docker_bin(env), &["stop", "--time", "1", id])?;
            }
        }
    }
    Ok(())
}
fn rollback(env: &Env, r: &mut Receipt) -> Result<(), PrivError> {
    let _ = step(env, r, "Rollback: release public ports");
    listeners(env, ListenerAction::YieldPrimary)?;
    if r.mutated {
        let _ = step(env, r, "Rollback: restore original server and files");
        for file in &r.files {
            if let Some(before) = &file.before {
                atomic_write(Path::new(&file.path), before.as_bytes(), file.mode)?;
            } else if Path::new(&file.path).exists() {
                fs::remove_file(&file.path).map_err(|_| internal())?;
            }
        }
        match &r.plan.target {
            PortTarget::Unit(unit) => {
                validate_unit(unit)?;
                command(&systemctl_bin(env), &["reload-or-restart", unit])?;
            }
            PortTarget::Container(old) => {
                if let Some(service) = &r.service {
                    docker_args(
                        env,
                        &compose_args(&r.plan, None, &["up", "-d", "--no-deps", service]),
                    )?;
                } else {
                    // If create succeeded before saving its id, recover by the old name;
                    // never remove a container without proving it was created by this move.
                    let recover = r.new_id.clone().or_else(|| {
                        r.staged_name.as_ref().and_then(|name| {
                            let raw = command(&docker_bin(env), &["inspect", name]).ok()?;
                            let c: Value = serde_json::from_str(&raw).ok()?;
                            (c[0]["Config"]["Labels"]["opsbox.web-move"] == r.public.id)
                                .then(|| c[0]["Id"].as_str().unwrap_or("").to_owned())
                        })
                    });
                    if let Some(new) = &recover
                        && let Ok(c) = inspect(env, new)
                    {
                        if c["Config"]["Labels"]["opsbox.web-move"] != r.public.id {
                            return Err(refused(
                                "The replacement container changed. Refusing to remove an unrelated container.",
                            ));
                        }
                        command(&docker_bin(env), &["rm", "--force", new])?;
                    }
                    let c = inspect(env, old)?;
                    if c["Name"]
                        .as_str()
                        .is_some_and(|n| n.trim_start_matches('/') != r.plan.holder)
                    {
                        command(&docker_bin(env), &["rename", old, &r.plan.holder])?;
                    }
                    command(
                        &docker_bin(env),
                        &["update", &format!("--restart={}", r.old_restart), old],
                    )?;
                    command(&docker_bin(env), &["start", old])?;
                }
            }
        }
    }
    load_routes(env, &r.previous_routes)?;
    let _ = step(env, r, "Rollback: verify the old server answers again");
    verify(&r.baseline, 80, 443, Duration::from_secs(8))?;
    r.public.active = false;
    step(
        env,
        r,
        "Original server restored; opsbox is on fallback ports",
    )
}
fn verify(
    baseline: &[(String, u16, u16)],
    http: u16,
    https: u16,
    budget: Duration,
) -> Result<(), PrivError> {
    let deadline = Instant::now() + budget;
    for (domain, h, s) in baseline {
        loop {
            if Instant::now() >= deadline {
                return Err(refused(
                    "A website did not match its baseline status class before the verification deadline.",
                ));
            }
            if probe(domain, http, false, 1).is_ok_and(|n| n / 100 == h / 100)
                && probe(domain, https, true, 1).is_ok_and(|n| n / 100 == s / 100)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    Ok(())
}
const STEPS: [&str; 5] = [
    "Pre-check: validate config and baseline responses",
    "Load adopted routes",
    "Rebind the old server to private ports",
    "Bind opsbox to 80/443 immediately",
    "Verify every website through opsbox",
];
/// All failures, including a partial mutation, take the same recovery path.
fn cutover_steps(
    mut action: impl FnMut(usize) -> Result<(), PrivError>,
    mut undo: impl FnMut() -> Result<(), PrivError>,
) -> Result<(), PrivError> {
    for (index, name) in STEPS.iter().enumerate() {
        if let Err(e) = action(index) {
            let recovery = undo();
            return Err(error(
                e.code,
                &format!(
                    "Failed at {name}: {} {}",
                    e.message,
                    if recovery.is_ok() {
                        "Rollback verified: the original server answers again."
                    } else {
                        "Rollback needs attention. The receipt is retained; use Undo move and review the helper logs."
                    }
                ),
            ));
        }
    }
    Ok(())
}
fn move_web(
    env: &Env,
    target: &PortTarget,
    domains: &[String],
    everything_else: bool,
    confirmation: &str,
) -> Result<WebMove, PrivError> {
    if all_receipts(env)?.iter().any(|r| r.active) {
        return Err(refused(
            "Undo the existing move before moving another web server.",
        ));
    }
    let mut plan = discover(env, target)?;
    if !plan.supported {
        return Err(refused(&plan.explanation));
    }
    if confirmation != plan.confirmation {
        return Err(error(
            PrivErrorCode::ConfirmRequired,
            &format!("Type {} to continue.", plan.confirmation),
        ));
    }
    if domains.len() > 256 || domains.iter().any(|d| !valid_domain(d)) {
        return Err(invalid());
    }
    let mut names: BTreeSet<String> = plan
        .routes
        .domains
        .iter()
        .map(|d| d.domain.clone())
        .collect();
    names.extend(domains.iter().map(|s| s.to_ascii_lowercase()));
    if names.is_empty() {
        // HTTP/HTTPS probes still test unknown/no-SNI catch-all servers.
        if !everything_else {
            return Err(refused("Add domains or enable Everything else."));
        }
        names.insert("opsbox-adoption-probe.invalid".into());
    }
    for domain in &names {
        if plan.routes.domains.iter().any(|r| &r.domain == domain) {
            continue;
        }
        plan.routes.domains.push(AdoptedDomain {
            domain: domain.clone(),
            served_by: plan.holder.clone(),
            certificate_expiry: None,
            has_certificate: false,
            reachable_now: false,
            terminate_tls: false,
        });
    }
    plan.routes
        .domains
        .retain(|d| d.domain != "opsbox-adoption-probe.invalid");
    plan.routes.everything_else = everything_else;
    let previous_routes = adopted(env)?;
    let mut random = [0; 16];
    getrandom::fill(&mut random).map_err(|_| internal())?;
    let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let mut files = Vec::new();
    use std::os::unix::fs::PermissionsExt;
    for change in &plan.changes {
        if !change.path.starts_with('/') {
            continue;
        }
        let path = Path::new(&change.path);
        files.push(SavedFile {
            path: change.path.clone(),
            backup: format!("{}.opsbox-{id}.bak", change.path),
            before: fs::read_to_string(path).ok(),
            after: change.after.clone(),
            mode: fs::metadata(path).map_or(0o644, |m| m.permissions().mode() & 0o777),
        });
    }
    let mut r = Receipt {
        public: WebMove {
            id,
            holder: plan.holder.clone(),
            files: files.iter().map(|f| f.path.clone()).collect(),
            http_port: plan.routes.http_port,
            https_port: plan.routes.https_port,
            steps: vec![],
            active: true,
        },
        plan,
        files,
        service: None,
        recreate: vec![],
        old_name: None,
        new_id: None,
        snapshot: None,
        staged_name: None,
        baseline: vec![],
        previous_routes,
        mutated: false,
        old_restart: String::new(),
    };
    if let PortTarget::Container(id) = target {
        let c = inspect(env, id)?;
        r.service = label(&c, "com.docker.compose.service");
        if r.service.is_none() {
            r.recreate = recreate_args(&c, r.public.http_port, r.public.https_port)?;
            let policy = c["HostConfig"]["RestartPolicy"]["Name"]
                .as_str()
                .unwrap_or("no");
            let retries = c["HostConfig"]["RestartPolicy"]["MaximumRetryCount"]
                .as_u64()
                .unwrap_or(0);
            r.old_restart = if policy == "on-failure" && retries > 0 {
                format!("on-failure:{retries}")
            } else {
                policy.into()
            };
            r.old_name = Some(format!("{}-pre-opsbox", r.plan.holder));
        }
    }
    save(env, &r)?;
    // RefCell keeps the real sequence and the recovery path on the same durable receipt.
    let cell = std::cell::RefCell::new(r);
    cutover_steps(
        |index| {
            let mut r = cell.borrow_mut();
            step(env, &mut r, STEPS[index])?;
            match index {
                0 => {
                    revalidate(env, target)?;
                    let _http = TcpListener::bind((Ipv4Addr::LOCALHOST, r.public.http_port))
                        .map_err(|_| invalid())?;
                    let _https = TcpListener::bind((Ipv4Addr::LOCALHOST, r.public.https_port))
                        .map_err(|_| invalid())?;
                    match target {
                        PortTarget::Unit(unit) => validate_unit(unit)?,
                        PortTarget::Container(_) => {
                            if r.service.is_some() {
                                use std::io::Write;
                                let file = &r.files[0];
                                let mut candidate = tempfile::NamedTempFile::new_in(
                                    Path::new(&file.path).parent().ok_or_else(invalid)?,
                                )
                                .map_err(|_| internal())?;
                                candidate
                                    .write_all(file.after.as_bytes())
                                    .map_err(|_| internal())?;
                                let name = candidate.path().to_str().ok_or_else(invalid)?;
                                docker_args(
                                    env,
                                    &compose_args(&r.plan, Some(name), &["config", "--quiet"]),
                                )?;
                            }
                        }
                    }
                    for domain in &names {
                        let h = probe(domain, 80, false, 2)?;
                        let s = probe(domain, 443, true, 2)?;
                        r.baseline.push((domain.clone(), h, s));
                    }
                    save(env, &r)
                }
                1 => load_routes(env, &r.plan.routes),
                2 => rebind(env, &mut r),
                3 => listeners(env, ListenerAction::ClaimPrimary),
                4 => verify(&r.baseline, 80, 443, Duration::from_secs(20)),
                _ => unreachable!(),
            }
        },
        || rollback(env, &mut cell.borrow_mut()),
    )?;
    let mut r = cell.into_inner();
    step(env, &mut r, "Move complete; Undo move is available")?;
    Ok(r.public)
}
fn handle_tls(env: &Env, domain: &str) -> Result<(), PrivError> {
    if !valid_domain(domain) {
        return Err(invalid());
    }
    let mut routes = adopted(env)?;
    if !routes.domains.iter().any(|d| d.domain == domain) {
        return Err(invalid());
    }
    let previous_routes = routes.clone();
    let baseline = probe(domain, 443, true, 2)?;
    let rows = all_receipts(env)?;
    let id = &rows.iter().find(|r| r.active).ok_or_else(invalid)?.id;
    let receipt = read_receipt(env, id)?;
    if let Some(cert) = find_cert(env, &receipt.plan.target, domain, true)
        .filter(|c| c.before <= now() && c.expiry > now() && c.key.is_some())
    {
        let key=zeroize::Zeroizing::new(cert.key.ok_or_else(||refused("The existing certificate key was not found. Keep passthrough until a new certificate can be issued."))?);
        // Check the key matches the certificate before touching active routing.
        let cert_pub = exec::run_with(
            "/usr/bin/openssl",
            &["x509", "-pubkey", "-noout"],
            Duration::from_secs(3),
            Some(&cert.cert),
            &[],
        )
        .filter(|o| o.ok())
        .ok_or_else(internal)?;
        let key_pub = exec::run_with(
            "/usr/bin/openssl",
            &["pkey", "-pubout"],
            Duration::from_secs(3),
            Some(&key),
            &[],
        )
        .filter(|o| o.ok())
        .ok_or_else(internal)?;
        if cert_pub.stdout != key_pub.stdout {
            return Err(refused("The existing certificate and key do not match."));
        }
        let sealed = env
            .secrets
            .seal(key.as_bytes(), &format!("proxy:cert:{domain}"))
            .map_err(|_| internal())?;
        control(
            env,
            json!({"type":"import_cert", "args":{"domain":domain, "cert_pem":cert.cert, "key_sealed":sealed}}),
        )?;
    } else {
        // Keep the passthrough route live during issuance. The proxy's HTTP-01
        // token is intercepted only while a matching local challenge exists.
        control(env, json!({"type":"issue_cert","args":{"domain":domain}}))?;
    }
    for d in &mut routes.domains {
        if d.domain == domain {
            d.terminate_tls = true;
        }
    }
    load_routes(env, &routes)?;
    if !probe(domain, 443, true, 2).is_ok_and(|status| status / 100 == baseline / 100) {
        load_routes(env, &previous_routes)?;
        return Err(refused(
            "The old HTTP server did not preserve this site's response after TLS termination. Passthrough has been restored. Configure the old server to accept X-Forwarded-Proto before trying again.",
        ));
    }
    Ok(())
}
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
/// Only changed directive lines enter the panel; full configs/backups stay root-only.
fn public_plan(mut p: WebPlan) -> WebPlan {
    for d in &mut p.changes {
        if d.path.starts_with('/') && !d.before.is_empty() {
            let before: Vec<_> = d.before.lines().collect();
            let after: Vec<_> = d.after.lines().collect();
            let old = before
                .iter()
                .filter(|l| !after.contains(l))
                .copied()
                .collect::<Vec<_>>()
                .join("\n");
            let new = after
                .iter()
                .filter(|l| !before.contains(l))
                .copied()
                .collect::<Vec<_>>()
                .join("\n");
            d.before = old;
            d.after = new;
        }
    }
    p
}
pub(super) fn dispatch(op: &PrivOp, env: &Env) -> Result<PrivReply, PrivError> {
    match op {
        PrivOp::WebDiscover { target } => discover(env, target)
            .map(public_plan)
            .map(Box::new)
            .map(PrivReply::WebPlan),
        PrivOp::WebMoves => all_receipts(env).map(PrivReply::WebMoves),
        PrivOp::WebMove {
            target,
            domains,
            everything_else,
            confirmation,
        } => move_web(env, target, domains, *everything_else, confirmation).map(PrivReply::WebMove),
        PrivOp::WebUndo { id, confirm } => {
            confirmed(*confirm)?;
            let mut r = read_receipt(env, id)?;
            if !r.public.active {
                return Err(refused("This move was already undone."));
            }
            rollback(env, &mut r)?;
            Ok(PrivReply::WebMove(r.public))
        }
        PrivOp::WebTls { domain } => handle_tls(env, domain).map(|()| PrivReply::Done),
        _ => Err(invalid()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    const NGINX: &str = include_str!("../../tests/fixtures/adoption/nginx.conf");
    const APACHE: &str = include_str!("../../tests/fixtures/adoption/apache.conf");
    #[test]
    fn nginx_listen_fixtures_keep_flags_and_each_virtual_host() {
        let out = rewrite_listens(NGINX, false, 18080, 18443).unwrap();
        assert!(out.contains("listen 127.0.0.1:18443 ssl http2 default_server;"));
        assert_eq!(out.matches("listen 127.0.0.1:18080").count(), 2);
        assert_eq!(out.matches("listen 127.0.0.1:18443").count(), 2);
        assert!(!out.contains("ipv6only"));
        let unrelated = |raw: &str| {
            raw.lines()
                .filter(|l| !l.trim_start().starts_with("listen "))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(unrelated(NGINX), unrelated(&out));
        assert_eq!(
            domains_from(NGINX, "nginx"),
            ["old.test", "second.test", "www.old.test"]
        );
    }
    #[test]
    fn apache_fixtures_change_only_listens() {
        let out = rewrite_listens(APACHE, true, 18081, 18444).unwrap();
        assert!(out.contains("Listen 127.0.0.1:18081\n"));
        assert!(out.contains("Listen 127.0.0.1:18444 https\n"));
        let unrelated = |raw: &str| {
            raw.lines()
                .filter(|l| !l.starts_with("Listen "))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(unrelated(APACHE), unrelated(&out));
        assert_eq!(
            domains_from(
                "port 80 namevhost old.test (/etc/apache2/a.conf:1)\n alias www.old.test",
                "apache2"
            ),
            ["old.test", "www.old.test"]
        );
    }
    #[test]
    fn compose_override_preserves_non_web_ports_and_container_targets() {
        let c: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/adoption/compose-inspect.json"
        ))
        .unwrap();
        assert_eq!(
            label(&c, "com.docker.compose.project").as_deref(),
            Some("existing")
        );
        let ports = vec![
            json!({"target":8080,"published":"80","protocol":"tcp"}),
            json!({"target":8443,"published":"443","protocol":"tcp"}),
            json!({"target":9090,"published":"9090","host_ip":"10.0.0.1","protocol":"tcp"}),
        ];
        let output = compose_override("web", &ports, 18080, 18443);
        assert!(output.contains("ports: !override"));
        let rows: Vec<Value> = output
            .lines()
            .filter_map(|l| l.trim().strip_prefix("- "))
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows[0]["target"], 8080);
        assert_eq!(rows[0]["published"], "18080");
        assert_eq!(rows[0]["host_ip"], "127.0.0.1");
        assert_eq!(rows[1]["published"], "18443");
        assert_eq!(rows[2], ports[2]);
    }
    #[derive(Default, Debug, PartialEq, Eq)]
    struct Fake {
        routes: bool,
        moved: bool,
        proxy: bool,
        verified: bool,
    }
    fn fake_step(s: &mut Fake, n: usize) {
        match n {
            1 => s.routes = true,
            2 => s.moved = true,
            3 => s.proxy = true,
            4 => s.verified = true,
            _ => {}
        }
    }
    #[test]
    fn cutover_success_and_explicit_undo_restore_original_state() {
        let state = RefCell::new(Fake::default());
        cutover_steps(
            |n| {
                fake_step(&mut state.borrow_mut(), n);
                Ok(())
            },
            || panic!("unexpected rollback"),
        )
        .unwrap();
        assert_eq!(
            *state.borrow(),
            Fake {
                routes: true,
                moved: true,
                proxy: true,
                verified: true
            }
        );
        // The same recovery closure is called by Undo and automatic failure recovery.
        let undo = || {
            *state.borrow_mut() = Fake::default();
            Ok::<_, PrivError>(())
        };
        undo().unwrap();
        assert_eq!(*state.borrow(), Fake::default());
    }
    #[test]
    fn every_failed_step_including_partial_rebind_rolls_back() {
        for (fail, failed_step) in STEPS.iter().enumerate() {
            let state = RefCell::new(Fake::default());
            let calls = RefCell::new(Vec::new());
            let error = cutover_steps(
                |n| {
                    calls.borrow_mut().push(n);
                    fake_step(&mut state.borrow_mut(), n);
                    if n == fail {
                        Err(refused("injected failure"))
                    } else {
                        Ok(())
                    }
                },
                || {
                    *state.borrow_mut() = Fake::default();
                    Ok(())
                },
            )
            .unwrap_err();
            assert_eq!(*state.borrow(), Fake::default());
            assert_eq!(calls.borrow().len(), fail + 1);
            assert!(error.message.contains(failed_step));
            assert!(error.message.contains("Rollback verified"));
        }
        let err = cutover_steps(|_| Err(internal()), || Err(internal())).unwrap_err();
        assert!(err.message.contains("receipt is retained"));
    }
    #[test]
    fn caddy_traefik_and_container_rules_are_bounded() {
        let caddy = rewrite_caddy(
            "{\n email admin@old.test\n http_port 80\n}\nold.test {\n respond ok\n}\n",
            18080,
            18443,
        )
        .unwrap();
        assert!(caddy.contains("email admin@old.test"));
        assert!(caddy.contains("http_port 18080"));
        assert!(caddy.contains("default_bind 127.0.0.1"));
        assert!(rewrite_caddy("import secrets/*", 18080, 18443).is_err());
        let traefik = rewrite_traefik(
            "entryPoints:\n  web:\n    address: ':80'\n  websecure:\n    address: ':443'\n",
            18080,
            18443,
        )
        .unwrap();
        assert!(traefik.contains("address: '127.0.0.1:18443'"));
        assert_eq!(
            domains_from("Host(`old.test`, `www.old.test`)", "traefik"),
            ["old.test", "www.old.test"]
        );
        let mut c: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/adoption/compose-inspect.json"
        ))
        .unwrap();
        let args = recreate_args(&c, 18080, 18443).unwrap();
        assert!(args.contains(&"127.0.0.1:18080:80/tcp".to_owned()));
        c["Config"]["Env"] = json!(["PASSWORD=do-not-display"]);
        let args = recreate_args(&c, 18080, 18443).unwrap();
        assert!(!run_preview(&c, &args, true).contains("do-not-display"));
        assert!(run_preview(&c, &args, true).contains("PASSWORD=<kept private>"));
        assert!(run_preview(&c, &args, false).contains("0.0.0.0:80:80/tcp"));
        c["HostConfig"]["NetworkMode"] = json!("host");
        assert!(recreate_args(&c, 18080, 18443).is_err());
        assert!(!valid_domain("old.test;touch /tmp/x"));
        assert!(!valid_domain("../secret"));
    }
}
