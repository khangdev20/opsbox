//! Exercise the installer migration with real files and mocked account/systemd tools.
use std::process::Command;

const INSTALLER: &str = include_str!("../../../install.sh");

#[test]
fn proxy_identity_migration_is_idempotent_and_preserves_state() {
    let script = INSTALLER
        .split("# BEGIN proxy-identity\n")
        .nth(1)
        .expect("installer must share the proxy migration with helper startup")
        .split("# END proxy-identity")
        .next()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(state.join("proxy/certs")).unwrap();
    std::fs::write(state.join("proxy/certs/key"), "sealed").unwrap();
    let harness = format!(
        r#"
set -eu
STATE_DIR=$1
PROXY_STATE_DIR=$STATE_DIR/proxy
DRY_RUN=0
PROXY_USER=opsbox-proxy
run() {{ "$@"; }}
die() {{ echo "$*" >&2; exit 1; }}
getent() {{
    case "$1" in
    group) test -f "$STATE_DIR/group";;
    passwd) if test -f "$STATE_DIR/user"; then echo 'opsbox-proxy:x:4242:4242::/nonexistent:/usr/sbin/nologin'; else return 2; fi;;
    esac
}}
groupadd() {{ touch "$STATE_DIR/group"; }}
useradd() {{ test -f "$STATE_DIR/group"; test ! -e "$STATE_DIR/user"; touch "$STATE_DIR/user"; }}
chown() {{ printf '%s\n' "$*" >> "$STATE_DIR/chowns"; }}
systemctl() {{ printf '%s\n' "$*" >> "$STATE_DIR/units"; }}
{script}
prepare_proxy_identity
prepare_proxy_identity
"#
    );
    let output = Command::new("sh")
        .args(["-c", &harness, "migration"])
        .arg(&state)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(state.join("proxy/certs/key")).unwrap(),
        "sealed"
    );
    let chowns = std::fs::read_to_string(state.join("chowns")).unwrap();
    assert!(chowns.contains("-hR -P opsbox-proxy:opsbox-proxy"));
    assert!(!chowns.contains("opsbox:opsbox"));
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&state).unwrap().permissions().mode() & 0o7777,
        0o1771
    );
    // A link must be refused without changing the target.
    let alias = dir.path().join("alias");
    std::os::unix::fs::symlink(&state, &alias).unwrap();
    let output = Command::new("sh")
        .args(["-c", &harness, "migration"])
        .arg(&alias)
        .output()
        .unwrap();
    assert!(!output.status.success());
}
