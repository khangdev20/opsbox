//! Reference variable syntax parsing, validation, and resolution (`${{ service.KEY }}`).
//!
//! App services store symbolic references such as `${{ postgres-main.DATABASE_URL }}`
//! instead of copying database credentials. `opsbox-priv` resolves references into
//! container environments only at build/start time.

use std::collections::HashSet;

use opsbox_proto::{valid_env_key, valid_service_name};

use crate::DeployError;

/// Parsed `${{ service.KEY }}` variable reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VarRef {
    pub service: String,
    pub key: String,
}

fn parse_inner_ref(inner: &str) -> Result<VarRef, DeployError> {
    let trimmed = inner.trim();
    let Some((svc, key)) = trimmed.split_once('.') else {
        return Err(DeployError::Validation {
            field: Some("vars"),
            message: format!(
                "Invalid variable reference '${{{{ {trimmed} }}}}'. Use ${{{{ service.KEY }}}}."
            ),
        });
    };
    let svc = svc.trim();
    let key = key.trim();
    if !valid_service_name(svc) || !valid_env_key(key) {
        return Err(DeployError::Validation {
            field: Some("vars"),
            message: format!(
                "Invalid variable reference '${{{{ {trimmed} }}}}'. Use ${{{{ service-name.VAR_KEY }}}}."
            ),
        });
    }
    Ok(VarRef {
        service: svc.to_owned(),
        key: key.to_owned(),
    })
}

/// Format a [`VarRef`] in canonical `${{ service.KEY }}` display form.
pub fn canonical_ref_expr(r: &VarRef) -> String {
    format!("${{{{ {}.{} }}}}", r.service, r.key)
}

/// If `raw` (trimmed) is a single pure `${{ service.KEY }}` expression with no surrounding
/// characters, return the parsed [`VarRef`].
pub fn parse_pure_var_ref(raw: &str) -> Option<VarRef> {
    let trimmed = raw.trim();
    let inner = trimmed.strip_prefix("${{")?.strip_suffix("}}")?;
    if inner.contains("${{") || inner.contains("}}") {
        return None;
    }
    parse_inner_ref(inner).ok()
}

/// Extract all `${{ service.KEY }}` references from `raw`, rejecting unclosed or malformed tokens.
pub fn extract_var_refs(raw: &str) -> Result<Vec<VarRef>, DeployError> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut rest = raw;
    while let Some(start) = rest.find("${{") {
        let after_open = &rest[start + 3..];
        let Some(end) = after_open.find("}}") else {
            return Err(DeployError::Validation {
                field: Some("vars"),
                message: "Unclosed variable reference '${{' — expected closing '}}'.".to_owned(),
            });
        };
        let inner = &after_open[..end];
        let parsed = parse_inner_ref(inner)?;
        if seen.insert(parsed.clone()) {
            out.push(parsed);
        }
        rest = &after_open[end + 2..];
    }
    Ok(out)
}

/// Resolve all `${{ service.KEY }}` references inside `raw` using `lookup(service, key)`.
///
/// Returns `(resolved_value, used_any_secret)`. Refuses self-references, missing targets,
/// and reference cycles (up to depth 4).
pub fn resolve_template_value<'a, F>(
    raw: &str,
    self_service: &str,
    lookup: &F,
) -> Result<(String, bool), DeployError>
where
    F: Fn(&str, &str) -> Option<(&'a str, bool)>,
{
    let mut stack = Vec::new();
    resolve_recursive(raw, self_service, lookup, &mut stack, 0)
}

fn resolve_recursive<'a, F>(
    raw: &str,
    self_service: &str,
    lookup: &F,
    stack: &mut Vec<(String, String)>,
    depth: usize,
) -> Result<(String, bool), DeployError>
where
    F: Fn(&str, &str) -> Option<(&'a str, bool)>,
{
    if depth > 4 {
        return Err(DeployError::Validation {
            field: Some("vars"),
            message: "Variable reference nesting exceeds maximum depth (4).".to_owned(),
        });
    }
    if !raw.contains("${{") {
        return Ok((raw.to_owned(), false));
    }

    let mut out = String::with_capacity(raw.len());
    let mut used_secret = false;
    let mut rest = raw;
    while let Some(start) = rest.find("${{") {
        out.push_str(&rest[..start]);
        let after_open = &rest[start + 3..];
        let Some(end) = after_open.find("}}") else {
            return Err(DeployError::Validation {
                field: Some("vars"),
                message: "Unclosed variable reference '${{'.".to_owned(),
            });
        };
        let vr = parse_inner_ref(&after_open[..end])?;
        if vr.service == self_service {
            return Err(DeployError::Validation {
                field: Some("vars"),
                message: format!(
                    "Service '{self_service}' cannot reference its own variable '{}'.",
                    vr.key
                ),
            });
        }
        let pair = (vr.service.clone(), vr.key.clone());
        if stack.contains(&pair) {
            return Err(DeployError::Validation {
                field: Some("vars"),
                message: format!(
                    "Circular variable reference involving '{}.{}'.",
                    vr.service, vr.key
                ),
            });
        }
        let Some((target_val, target_secret)) = lookup(&vr.service, &vr.key) else {
            return Err(DeployError::Validation {
                field: Some("vars"),
                message: format!(
                    "Referenced variable '{}.{}' was not found.",
                    vr.service, vr.key
                ),
            });
        };
        if target_secret {
            used_secret = true;
        }
        stack.push(pair);
        let (resolved_target, nested_secret) =
            resolve_recursive(target_val, &vr.service, lookup, stack, depth + 1)?;
        stack.pop();
        if nested_secret {
            used_secret = true;
        }
        out.push_str(&resolved_target);
        rest = &after_open[end + 2..];
    }
    out.push_str(rest);
    Ok((out, used_secret))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn parses_pure_and_embedded_references() {
        let pure = parse_pure_var_ref("  ${{ postgres-main.DATABASE_URL }} ").unwrap();
        assert_eq!(pure.service, "postgres-main");
        assert_eq!(pure.key, "DATABASE_URL");
        assert_eq!(
            canonical_ref_expr(&pure),
            "${{ postgres-main.DATABASE_URL }}"
        );

        assert!(parse_pure_var_ref("prefix-${{ postgres-main.PGPASSWORD }}").is_none());
        assert!(parse_pure_var_ref("${{ a.B }}-${{ c.D }}").is_none());

        let refs = extract_var_refs("postgres://${{ pg.PGUSER }}:${{ pg.PGPASSWORD }}@${{ pg.PGHOST }}:5432/${{ pg.PGDATABASE }}").unwrap();
        assert_eq!(refs.len(), 4);
        assert_eq!(refs[0].key, "PGUSER");
        assert_eq!(refs[3].key, "PGDATABASE");

        assert!(extract_var_refs("${{ unclosed").is_err());
        assert!(extract_var_refs("${{ BadName.KEY }}").is_err());
        assert!(extract_var_refs("${{ pg.bad-key }}").is_err());
        assert!(extract_var_refs("${{ nodot }}").is_err());
    }

    #[test]
    fn resolves_references_and_blocks_cycles_and_self_refs() {
        let mut map: HashMap<(&str, &str), (&str, bool)> = HashMap::new();
        map.insert(
            ("postgres-main", "DATABASE_URL"),
            ("postgresql://app:s3cr3t@postgres-main:5432/app", true),
        );
        map.insert(("postgres-main", "PGHOST"), ("postgres-main", false));
        map.insert(("api", "UPSTREAM"), ("${{ api-cycle.UPSTREAM }}", false));
        map.insert(("api-cycle", "UPSTREAM"), ("${{ api.UPSTREAM }}", false));

        let lookup = |s: &str, k: &str| map.get(&(s, k)).copied();

        let (resolved, is_sec) =
            resolve_template_value("${{ postgres-main.DATABASE_URL }}", "web", &lookup).unwrap();
        assert_eq!(resolved, "postgresql://app:s3cr3t@postgres-main:5432/app");
        assert!(is_sec);

        let (host, host_sec) =
            resolve_template_value("${{ postgres-main.PGHOST }}", "web", &lookup).unwrap();
        assert_eq!(host, "postgres-main");
        assert!(!host_sec);

        assert!(resolve_template_value("${{ web.DATABASE_URL }}", "web", &lookup).is_err());
        assert!(resolve_template_value("${{ api.UPSTREAM }}", "web", &lookup).is_err());
        assert!(resolve_template_value("${{ postgres-main.MISSING }}", "web", &lookup).is_err());
    }
}
