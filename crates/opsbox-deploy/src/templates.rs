//! Built-in TOML service templates shipped in the binary (docs/BACKEND_SPEC.md 7.6).

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::DeployError;

const RAW_TEMPLATES: &[(&str, &str)] = &[
    ("postgres", include_str!("../templates/postgres.toml")),
    ("mysql", include_str!("../templates/mysql.toml")),
    ("mariadb", include_str!("../templates/mariadb.toml")),
    ("redis", include_str!("../templates/redis.toml")),
    ("mongo", include_str!("../templates/mongo.toml")),
    ("minio", include_str!("../templates/minio.toml")),
    ("kuma", include_str!("../templates/kuma.toml")),
    ("umami", include_str!("../templates/umami.toml")),
    ("n8n", include_str!("../templates/n8n.toml")),
    ("grafana", include_str!("../templates/grafana.toml")),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TemplateCategory {
    Databases,
    Tools,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TemplateKind {
    Db,
    Tool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SecretGenerator {
    #[default]
    None,
    Password,
    Hex32,
    Hex64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct TemplateVolumeSpec {
    pub suffix: String,
    pub container_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct TemplateEnvSpec {
    pub key: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub generate: SecretGenerator,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub struct TemplateConnectionVar {
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub secret: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub struct TemplateConnectionSpec {
    #[serde(default)]
    pub primary_var: Option<String>,
    #[serde(default)]
    pub url_template: Option<String>,
    #[serde(default)]
    pub vars: Vec<TemplateConnectionVar>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub struct TemplateHealthSpec {
    #[serde(default)]
    pub exec: Vec<String>,
    #[serde(default)]
    pub http_path: Option<String>,
    #[serde(default)]
    pub tcp_port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct TemplateBackupSpec {
    pub enabled_by_default: bool,
    pub cron: String,
    pub method: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct TemplateSpec {
    pub id: String,
    pub name: String,
    pub category: TemplateCategory,
    pub kind: TemplateKind,
    pub description: String,
    pub resources: String,
    pub image: String,
    pub versions: Vec<String>,
    pub port: u16,
    #[serde(default)]
    pub extra_ports: Vec<u16>,
    #[serde(default)]
    pub cmd: Vec<String>,
    #[serde(default)]
    pub internal_by_default: bool,
    #[serde(default)]
    pub volumes: Vec<TemplateVolumeSpec>,
    #[serde(default)]
    pub env: Vec<TemplateEnvSpec>,
    #[serde(default)]
    pub connection: TemplateConnectionSpec,
    #[serde(default)]
    pub health: TemplateHealthSpec,
    pub backup: TemplateBackupSpec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedVar {
    pub key: String,
    pub value: String,
    pub is_secret: bool,
    pub container_env: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedInstanceEnv {
    pub vars: Vec<GeneratedVar>,
    pub primary_var: Option<String>,
    pub primary_url: Option<String>,
}

fn load_catalog() -> Vec<TemplateSpec> {
    RAW_TEMPLATES
        .iter()
        .map(|(id, raw)| {
            let spec: TemplateSpec = toml::from_str(raw)
                .unwrap_or_else(|e| panic!("embedded template '{id}.toml' failed to parse: {e}"));
            assert_eq!(spec.id, *id, "template id mismatch in {id}.toml");
            assert!(
                !spec.versions.is_empty(),
                "template '{id}' must list at least one version"
            );
            spec
        })
        .collect()
}

/// Return the compiled-in v1 template catalog.
pub fn catalog() -> &'static [TemplateSpec] {
    static CATALOG: OnceLock<Vec<TemplateSpec>> = OnceLock::new();
    CATALOG.get_or_init(load_catalog)
}

/// Look up a template by ID (`postgres`, `redis`, `mysql`, `mariadb`, `mongo`,
/// `minio`, `kuma`, `umami`, `n8n`, `grafana`).
pub fn get_template(id: &str) -> Option<&'static TemplateSpec> {
    catalog().iter().find(|t| t.id == id)
}

/// Generate a URL-safe alphanumeric secret of `len` characters using the OS CSPRNG.
pub fn random_password(len: usize) -> Result<String, DeployError> {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut buf = vec![0u8; len];
    getrandom::fill(&mut buf).map_err(|e| DeployError::Internal(format!("RNG failed: {e}")))?;
    Ok(buf
        .into_iter()
        .map(|b| ALPHABET[(b as usize) % ALPHABET.len()] as char)
        .collect())
}

/// Generate `n_bytes` random bytes encoded as lowercase hex.
pub fn random_hex(n_bytes: usize) -> Result<String, DeployError> {
    let mut buf = vec![0u8; n_bytes];
    getrandom::fill(&mut buf).map_err(|e| DeployError::Internal(format!("RNG failed: {e}")))?;
    let mut out = String::with_capacity(n_bytes * 2);
    for b in buf {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    Ok(out)
}

fn expand_placeholders(tmpl: &str, vars: &HashMap<String, String>) -> String {
    let mut out = tmpl.to_owned();
    for (k, v) in vars {
        let token = format!("{{{{{k}}}}}");
        if out.contains(&token) {
            out = out.replace(&token, v);
        }
    }
    out
}

/// Generate all container environment variables, connection strings, and secrets
/// for a new template instance.
pub fn generate_instance_env(
    tmpl: &TemplateSpec,
    instance_name: &str,
    overrides: &[(String, String, bool)],
) -> Result<GeneratedInstanceEnv, DeployError> {
    let mut resolved: HashMap<String, String> = HashMap::new();
    resolved.insert("service".to_owned(), instance_name.to_owned());

    let mut override_map: HashMap<&str, (&str, bool)> = HashMap::new();
    for (k, v, sec) in overrides {
        override_map.insert(k.as_str(), (v.as_str(), *sec));
    }

    let mut vars = Vec::new();
    for spec in &tmpl.env {
        let val = if let Some((ov, _)) = override_map.get(spec.key.as_str()) {
            (*ov).to_owned()
        } else {
            match spec.generate {
                SecretGenerator::None => expand_placeholders(&spec.value, &resolved),
                SecretGenerator::Password => random_password(24)?,
                SecretGenerator::Hex32 => random_hex(16)?,
                SecretGenerator::Hex64 => random_hex(32)?,
            }
        };
        resolved.insert(spec.key.clone(), val.clone());
        vars.push(GeneratedVar {
            key: spec.key.clone(),
            value: val,
            is_secret: spec.secret,
            container_env: true,
        });
    }

    // Apply any additional caller-provided variables (e.g., DATABASE_URL for Umami).
    for (k, v, sec) in overrides {
        if !vars.iter().any(|existing| existing.key == *k) {
            resolved.insert(k.clone(), v.clone());
            vars.push(GeneratedVar {
                key: k.clone(),
                value: v.clone(),
                is_secret: *sec,
                container_env: true,
            });
        }
    }

    for conn in &tmpl.connection.vars {
        let val = expand_placeholders(&conn.value, &resolved);
        resolved.insert(conn.key.clone(), val.clone());
        if let Some(existing) = vars.iter_mut().find(|v| v.key == conn.key) {
            existing.value = val;
            existing.is_secret = conn.secret;
        } else {
            vars.push(GeneratedVar {
                key: conn.key.clone(),
                value: val,
                is_secret: conn.secret,
                container_env: false,
            });
        }
    }

    let primary_url = tmpl
        .connection
        .url_template
        .as_deref()
        .map(|u| expand_placeholders(u, &resolved));

    Ok(GeneratedInstanceEnv {
        vars,
        primary_var: tmpl.connection.primary_var.clone(),
        primary_url,
    })
}

/// Deterministic Docker volume name for a template instance volume slot.
pub fn template_volume_name(instance_name: &str, suffix: &str) -> String {
    format!("opsbox-{instance_name}-{suffix}")
}

/// Parse disk choice label (`"1 GB"`, `"5 GB"`, `"20 GB"`) into gigabytes.
pub fn parse_disk_gb(raw: Option<&str>) -> Result<u64, DeployError> {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(1);
    };
    let clean = s
        .strip_suffix("GB")
        .or_else(|| s.strip_suffix("gb"))
        .unwrap_or(s)
        .trim();
    let gb: u64 = clean.parse().map_err(|_| DeployError::Validation {
        field: Some("disk"),
        message: "Disk size must be 1 GB, 5 GB, or 20 GB.".to_owned(),
    })?;
    if !(1..=10_000).contains(&gb) {
        return Err(DeployError::Validation {
            field: Some("disk"),
            message: "Disk size must be between 1 GB and 10000 GB.".to_owned(),
        });
    }
    Ok(gb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_all_ten_embedded_toml_templates() {
        let all = catalog();
        assert_eq!(all.len(), 10);
        for id in [
            "postgres", "mysql", "mariadb", "redis", "mongo", "minio", "kuma", "umami", "n8n",
            "grafana",
        ] {
            assert!(get_template(id).is_some(), "missing template {id}");
        }
    }

    #[test]
    fn generates_distinct_secrets_and_connection_urls_without_placeholders() {
        let pg = get_template("postgres").unwrap();
        let g1 = generate_instance_env(pg, "postgres-main", &[]).unwrap();
        let g2 = generate_instance_env(pg, "postgres-main", &[]).unwrap();
        let url1 = g1.primary_url.unwrap();
        let url2 = g2.primary_url.unwrap();
        assert_ne!(url1, url2);
        assert!(url1.starts_with("postgresql://app:"));
        assert!(url1.ends_with("@postgres-main:5432/app"));
        assert!(!url1.contains("{{"));
        assert!(
            g1.vars
                .iter()
                .any(|v| v.key == "POSTGRES_PASSWORD" && v.is_secret && !v.value.is_empty())
        );
        assert!(
            g1.vars
                .iter()
                .any(|v| v.key == "PGHOST" && !v.is_secret && v.value == "postgres-main")
        );

        for tmpl in catalog() {
            let generated = generate_instance_env(tmpl, "svc-test", &[]).unwrap();
            for v in &generated.vars {
                assert!(
                    !v.value.contains("{{"),
                    "unexpanded placeholder in {}.{}: {}",
                    tmpl.id,
                    v.key,
                    v.value
                );
            }
        }
    }
}
