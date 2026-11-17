//! Build method detection from repository files (docs/BACKEND_SPEC.md 7.5 step 4).

use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
struct PackageJson {
    #[serde(default)]
    scripts: std::collections::HashMap<String, String>,
    #[serde(default)]
    dependencies: std::collections::HashMap<String, String>,
    #[serde(default, rename = "devDependencies")]
    dev_dependencies: std::collections::HashMap<String, String>,
    #[serde(default)]
    engines: Option<PackageEngines>,
}

#[derive(Debug, Deserialize, Default)]
struct PackageEngines {
    node: Option<String>,
}

fn has_exact(entries: &[&str], name: &str) -> bool {
    entries.contains(&name)
}

fn has_ci(entries: &[&str], name: &str) -> bool {
    entries.iter().any(|e| e.eq_ignore_ascii_case(name))
}

fn node_major(engines_node: Option<&str>) -> Option<String> {
    let raw = engines_node?;
    let digits: String = raw
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let major: u32 = digits.parse().ok()?;
    (12..=30).contains(&major).then(|| major.to_string())
}

fn is_static_package(pkg: &PackageJson) -> bool {
    let has_build = pkg.scripts.contains_key("build");
    let has_start = pkg.scripts.contains_key("start");
    let has_static_dep = [
        "astro",
        "@11ty/eleventy",
        "vite",
        "vitepress",
        "vuepress",
        "gatsby",
        "@docusaurus/core",
        "parcel",
    ]
    .iter()
    .any(|d| pkg.dependencies.contains_key(*d) || pkg.dev_dependencies.contains_key(*d));
    let has_server_dep = [
        "express",
        "fastify",
        "koa",
        "hono",
        "@nestjs/core",
        "next",
        "nuxt",
        "remix",
        "@sveltejs/kit",
    ]
    .iter()
    .any(|d| pkg.dependencies.contains_key(*d));

    has_build && has_static_dep && !has_start && !has_server_dep
}

/// Detect the build method from root directory entry names, optional `package.json` content,
/// and optional GitHub repository primary language.
pub fn detect_build_method(
    entries: &[&str],
    package_json: Option<&str>,
    language: Option<&str>,
) -> String {
    if has_ci(entries, "Dockerfile") || has_ci(entries, "Containerfile") {
        return "Dockerfile".to_owned();
    }

    if has_exact(entries, "docker-compose.yml")
        || has_exact(entries, "docker-compose.yaml")
        || has_exact(entries, "compose.yml")
        || has_exact(entries, "compose.yaml")
    {
        return "Docker Compose".to_owned();
    }

    if has_exact(entries, "bun.lockb") || has_exact(entries, "bun.lock") {
        return "Nixpacks · Bun".to_owned();
    }

    if has_exact(entries, "deno.json") || has_exact(entries, "deno.jsonc") {
        return "Nixpacks · Deno".to_owned();
    }

    if has_exact(entries, "package.json") {
        if let Some(raw) = package_json
            && let Ok(pkg) = serde_json::from_str::<PackageJson>(raw)
        {
            if is_static_package(&pkg) {
                return "static site".to_owned();
            }
            if let Some(major) = node_major(pkg.engines.as_ref().and_then(|e| e.node.as_deref())) {
                return format!("Nixpacks · Node {major}");
            }
        }
        return "Nixpacks · Node".to_owned();
    }

    if has_exact(entries, "Cargo.toml") {
        return "Nixpacks · Rust".to_owned();
    }
    if has_exact(entries, "go.mod") {
        return "Nixpacks · Go".to_owned();
    }
    if has_exact(entries, "pyproject.toml")
        || has_exact(entries, "requirements.txt")
        || has_exact(entries, "Pipfile")
        || has_exact(entries, "setup.py")
    {
        return "Nixpacks · Python".to_owned();
    }
    if has_exact(entries, "Gemfile") {
        return "Nixpacks · Ruby".to_owned();
    }
    if has_exact(entries, "composer.json") {
        return "Nixpacks · PHP".to_owned();
    }
    if has_exact(entries, "pom.xml")
        || has_exact(entries, "build.gradle")
        || has_exact(entries, "build.gradle.kts")
    {
        return "Nixpacks · Java".to_owned();
    }
    if has_exact(entries, "mix.exs") {
        return "Nixpacks · Elixir".to_owned();
    }

    if has_exact(entries, "index.html") {
        return "static site".to_owned();
    }

    if let Some(lang) = language.map(str::trim).filter(|s| !s.is_empty()) {
        let lower = lang.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "typescript"
                | "javascript"
                | "go"
                | "rust"
                | "python"
                | "ruby"
                | "php"
                | "java"
                | "kotlin"
                | "c#"
                | "elixir"
                | "dart"
        ) {
            return format!("Nixpacks · {lang}");
        }
        if matches!(lower.as_str(), "html" | "astro" | "css") {
            return "static site".to_owned();
        }
    }

    "nothing to build".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_dockerfile_static_nixpacks_and_docs() {
        assert_eq!(
            detect_build_method(
                &["README.md", "Dockerfile", "package.json"],
                None,
                Some("TypeScript")
            ),
            "Dockerfile"
        );
        assert_eq!(
            detect_build_method(&["compose.yaml", "README.md"], None, None),
            "Docker Compose"
        );
        assert_eq!(
            detect_build_method(&["index.html", "styles.css"], None, Some("HTML")),
            "static site"
        );
        assert_eq!(
            detect_build_method(
                &["package.json", "astro.config.mjs"],
                Some(r#"{"scripts":{"build":"astro build"},"dependencies":{"astro":"^4.0.0"}}"#),
                Some("Astro"),
            ),
            "static site"
        );
        assert_eq!(
            detect_build_method(
                &["package.json", "src"],
                Some(
                    r#"{"scripts":{"build":"tsc","start":"node dist/index.js"},"engines":{"node":">=22"},"dependencies":{"express":"^4.19"}}"#
                ),
                Some("TypeScript"),
            ),
            "Nixpacks · Node 22"
        );
        assert_eq!(
            detect_build_method(&["go.mod", "main.go"], None, Some("Go")),
            "Nixpacks · Go"
        );
        assert_eq!(
            detect_build_method(&["Cargo.toml", "src"], None, Some("Rust")),
            "Nixpacks · Rust"
        );
        assert_eq!(
            detect_build_method(&["README.md", "docs"], None, Some("Markdown")),
            "nothing to build"
        );
    }
}
