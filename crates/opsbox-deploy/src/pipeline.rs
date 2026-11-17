//! Build planning, static-site Dockerfile generation, and secret log scrubbing (BACKEND_SPEC 7.5 steps 4-5).

use std::path::Path;

use serde::Deserialize;

use crate::{DeployError, detect_build_method};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckoutBuildPlan {
    Dockerfile {
        rel_path: String,
        label: String,
    },
    StaticHtml {
        doc_root_rel: String,
        label: String,
    },
    StaticNodeBuild {
        output_dir: String,
        node_major: String,
        label: String,
    },
    Nixpacks {
        label: String,
    },
}

impl CheckoutBuildPlan {
    pub fn label(&self) -> &str {
        match self {
            Self::Dockerfile { label, .. }
            | Self::StaticHtml { label, .. }
            | Self::StaticNodeBuild { label, .. }
            | Self::Nixpacks { label } => label,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct PartialPackageJson {
    #[serde(default)]
    scripts: std::collections::HashMap<String, String>,
    #[serde(default)]
    dependencies: std::collections::HashMap<String, String>,
    #[serde(default, rename = "devDependencies")]
    dev_dependencies: std::collections::HashMap<String, String>,
    #[serde(default)]
    engines: Option<PartialEngines>,
}

#[derive(Debug, Deserialize, Default)]
struct PartialEngines {
    node: Option<String>,
}

fn pick_node_major(pkg: &PartialPackageJson) -> String {
    if let Some(raw) = pkg.engines.as_ref().and_then(|e| e.node.as_deref()) {
        let digits: String = raw
            .trim_start_matches(|c: char| !c.is_ascii_digit())
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if let Ok(major) = digits.parse::<u32>()
            && (18..=24).contains(&major)
        {
            return major.to_string();
        }
    }
    "22".to_owned()
}

fn infer_static_output_dir(pkg: &PartialPackageJson) -> &'static str {
    let has =
        |name: &str| pkg.dependencies.contains_key(name) || pkg.dev_dependencies.contains_key(name);
    if let Some(script) = pkg.scripts.get("build") {
        if script.contains("react-scripts") {
            return "build";
        }
        if script.contains("eleventy") {
            return "_site";
        }
    }
    if has("gatsby") {
        "public"
    } else if has("@11ty/eleventy") {
        "_site"
    } else if has("docusaurus") || has("@docusaurus/core") {
        "build"
    } else {
        "dist"
    }
}

fn find_dockerfile(
    checkout_dir: &Path,
    custom: Option<&str>,
) -> Result<Option<String>, DeployError> {
    if let Some(rel) = custom {
        if !opsbox_proto::valid_rel_repo_path(rel) {
            return Err(DeployError::Validation {
                field: Some("dockerfilePath"),
                message: "Dockerfile path must be relative inside the repository.".to_owned(),
            });
        }
        let candidate = checkout_dir.join(rel);
        if candidate.is_file() {
            return Ok(Some(rel.to_owned()));
        }
        return Err(DeployError::Validation {
            field: Some("dockerfilePath"),
            message: format!("Dockerfile not found at '{rel}'."),
        });
    }
    for name in ["Dockerfile", "dockerfile", "Containerfile", "containerfile"] {
        if checkout_dir.join(name).is_file() {
            return Ok(Some(name.to_owned()));
        }
    }
    Ok(None)
}

/// Inspect a checked-out source directory and decide how to turn it into a Docker image.
pub fn plan_checkout_build(
    checkout_dir: &Path,
    requested_method: &str,
    custom_dockerfile: Option<&str>,
) -> Result<CheckoutBuildPlan, DeployError> {
    let method = requested_method.trim().to_ascii_lowercase();
    if method == "dockerfile" || custom_dockerfile.is_some() {
        let rel = find_dockerfile(checkout_dir, custom_dockerfile)?.ok_or_else(|| {
            DeployError::Validation {
                field: Some("build"),
                message: "No Dockerfile or Containerfile found in this repository.".to_owned(),
            }
        })?;
        return Ok(CheckoutBuildPlan::Dockerfile {
            rel_path: rel,
            label: "Dockerfile".to_owned(),
        });
    }

    let entries: Vec<String> = std::fs::read_dir(checkout_dir)
        .map_err(|e| DeployError::Internal(format!("Cannot inspect checked out source: {e}")))?
        .filter_map(Result::ok)
        .filter_map(|ent| ent.file_name().to_str().map(str::to_owned))
        .collect();
    let entry_refs: Vec<&str> = entries.iter().map(String::as_str).collect();
    let pkg_text = std::fs::read_to_string(checkout_dir.join("package.json")).ok();
    let pkg_parsed = pkg_text
        .as_deref()
        .and_then(|s| serde_json::from_str::<PartialPackageJson>(s).ok());

    if method == "static" || method == "static site" {
        if let Some(pkg) = pkg_parsed.as_ref()
            && pkg.scripts.contains_key("build")
        {
            return Ok(CheckoutBuildPlan::StaticNodeBuild {
                output_dir: infer_static_output_dir(pkg).to_owned(),
                node_major: pick_node_major(pkg),
                label: "Static site".to_owned(),
            });
        }
        let doc_root = if checkout_dir.join("index.html").is_file() {
            "."
        } else if checkout_dir.join("public/index.html").is_file() {
            "public"
        } else {
            "."
        };
        return Ok(CheckoutBuildPlan::StaticHtml {
            doc_root_rel: doc_root.to_owned(),
            label: "Static site".to_owned(),
        });
    }

    if method.starts_with("nixpacks") {
        let detected = detect_build_method(&entry_refs, pkg_text.as_deref(), None);
        let label = if detected.starts_with("Nixpacks") {
            detected
        } else {
            "Nixpacks".to_owned()
        };
        return Ok(CheckoutBuildPlan::Nixpacks { label });
    }

    // "auto" or previously detected label: Dockerfile -> static site -> Nixpacks.
    if let Some(rel) = find_dockerfile(checkout_dir, None)? {
        return Ok(CheckoutBuildPlan::Dockerfile {
            rel_path: rel,
            label: "Dockerfile".to_owned(),
        });
    }

    let detected = detect_build_method(&entry_refs, pkg_text.as_deref(), None);
    if detected.eq_ignore_ascii_case("static site")
        || checkout_dir.join("public/index.html").is_file()
    {
        if let Some(pkg) = pkg_parsed.as_ref()
            && pkg.scripts.contains_key("build")
        {
            return Ok(CheckoutBuildPlan::StaticNodeBuild {
                output_dir: infer_static_output_dir(pkg).to_owned(),
                node_major: pick_node_major(pkg),
                label: "static site".to_owned(),
            });
        }
        let doc_root = if checkout_dir.join("index.html").is_file() {
            "."
        } else if checkout_dir.join("public/index.html").is_file() {
            "public"
        } else {
            "."
        };
        return Ok(CheckoutBuildPlan::StaticHtml {
            doc_root_rel: doc_root.to_owned(),
            label: "static site".to_owned(),
        });
    }

    Ok(CheckoutBuildPlan::Nixpacks { label: detected })
}

/// Generate an ephemeral `Dockerfile` for built-in static site builds.
pub fn static_site_dockerfile(plan: &CheckoutBuildPlan, port: u16) -> Option<String> {
    let listen_port = if port == 0 { 80 } else { port };
    match plan {
        CheckoutBuildPlan::StaticHtml { doc_root_rel, .. } => {
            if !is_safe_copy_token(doc_root_rel) {
                return None;
            }
            Some(format!(
                "FROM nginx:1.27-alpine\n\
                 RUN printf 'server {{\\n  listen {listen_port};\\n  root /usr/share/nginx/html;\\n  index index.html;\\n  location / {{ try_files $uri $uri/ /index.html; }}\\n}}\\n' > /etc/nginx/conf.d/default.conf\n\
                 COPY {doc_root_rel} /usr/share/nginx/html\n\
                 EXPOSE {listen_port}\n"
            ))
        }
        CheckoutBuildPlan::StaticNodeBuild {
            output_dir,
            node_major,
            ..
        } => {
            if !is_safe_copy_token(output_dir) || !node_major.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            Some(format!(
                "FROM node:{node_major}-alpine AS builder\n\
                 WORKDIR /app\n\
                 COPY package*.json ./\n\
                 RUN if [ -f package-lock.json ]; then npm ci; else npm install; fi\n\
                 COPY . .\n\
                 RUN npm run build\n\
                 \n\
                 FROM nginx:1.27-alpine\n\
                 RUN printf 'server {{\\n  listen {listen_port};\\n  root /usr/share/nginx/html;\\n  index index.html;\\n  location / {{ try_files $uri $uri/ /index.html; }}\\n}}\\n' > /etc/nginx/conf.d/default.conf\n\
                 COPY --from=builder /app/{output_dir} /usr/share/nginx/html\n\
                 EXPOSE {listen_port}\n"
            ))
        }
        _ => None,
    }
}

fn is_safe_copy_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-'))
        && !s.contains("..")
}

/// Heuristic matching `apps.js` masking to default `is_secret` when callers omit it.
pub fn looks_secret_var_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    [
        "SECRET",
        "KEY",
        "TOKEN",
        "PASS",
        "URL",
        "AUTH",
        "PRIVATE",
        "CERT",
        "DSN",
        "CREDENTIAL",
    ]
    .iter()
    .any(|kw| upper.contains(kw))
}

/// Scrub secret strings (length >= 4) from log lines and error messages, longest match first.
pub fn scrub_secrets(text: &str, secrets: &[&str]) -> String {
    let mut filtered: Vec<&str> = secrets.iter().copied().filter(|s| s.len() >= 4).collect();
    if filtered.is_empty() {
        return text.to_owned();
    }
    filtered.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    filtered.dedup();
    let mut out = text.to_owned();
    for secret in filtered {
        if out.contains(secret) {
            out = out.replace(secret, "[REDACTED]");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_dockerfile_static_and_node_builds() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "<h1>hi</h1>").unwrap();
        let html_plan = plan_checkout_build(dir.path(), "auto", None).unwrap();
        assert!(matches!(
            html_plan,
            CheckoutBuildPlan::StaticHtml { ref doc_root_rel, .. } if doc_root_rel == "."
        ));
        let df = static_site_dockerfile(&html_plan, 3000).unwrap();
        assert!(df.contains("listen 3000;") && df.contains("nginx:1.27-alpine"));

        let astro = tempfile::tempdir().unwrap();
        std::fs::write(
            astro.path().join("package.json"),
            r#"{"scripts":{"build":"astro build"},"dependencies":{"astro":"^4.0.0"}}"#,
        )
        .unwrap();
        let node_plan = plan_checkout_build(astro.path(), "auto", None).unwrap();
        assert!(matches!(
            node_plan,
            CheckoutBuildPlan::StaticNodeBuild { ref output_dir, .. } if output_dir == "dist"
        ));
        let ndf = static_site_dockerfile(&node_plan, 80).unwrap();
        assert!(ndf.contains("npm run build") && ndf.contains("COPY --from=builder /app/dist"));

        std::fs::write(astro.path().join("Dockerfile"), "FROM scratch\n").unwrap();
        let df_plan = plan_checkout_build(astro.path(), "auto", None).unwrap();
        assert!(matches!(df_plan, CheckoutBuildPlan::Dockerfile { .. }));
    }

    #[test]
    fn scrubs_secrets_longest_first() {
        let out = scrub_secrets(
            "connecting with token_secret_extra and token_secret",
            &["token_secret", "token_secret_extra", "abc"],
        );
        assert_eq!(out, "connecting with [REDACTED] and [REDACTED]");
    }
}
