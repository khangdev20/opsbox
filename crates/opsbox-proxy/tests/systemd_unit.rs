use std::path::Path;

const PROXY_UNIT: &str = include_str!("../../../packaging/systemd/opsbox-proxy.service");

fn service_values(key: &str) -> Vec<&str> {
    let mut in_service = false;
    let mut values = Vec::new();
    for line in PROXY_UNIT.lines().map(str::trim) {
        if line.starts_with('[') {
            in_service = line == "[Service]";
        }
        if !in_service {
            continue;
        }
        if let Some(value) = line.strip_prefix(key).and_then(|s| s.strip_prefix('=')) {
            if value.trim().is_empty() {
                values.clear();
            } else {
                values.extend(value.split_whitespace());
            }
        }
    }
    values
}

#[test]
fn proxy_writable_paths_exclude_database_and_its_parents() {
    assert_eq!(service_values("ProtectSystem"), ["strict"]);
    let paths = service_values("ReadWritePaths");
    let database = Path::new("/var/lib/opsbox/opsbox.db");
    for entry in &paths {
        let path = entry.trim_start_matches(['-', '+']);
        assert!(
            !database.starts_with(path),
            "proxy ReadWritePaths={entry} grants write access to {}",
            database.display()
        );
    }
    assert!(paths.contains(&opsbox_proxy::DEFAULT_STATE_DIR));
}

#[test]
fn proxy_transfer_directory_is_inaccessible() {
    assert!(
        service_values("InaccessiblePaths").contains(&"/var/lib/opsbox/xfer"),
        "proxy must hide the transfer directory"
    );
}

#[test]
fn proxy_database_is_inaccessible() {
    assert!(
        service_values("InaccessiblePaths")
            .iter()
            .any(|path| path.trim_start_matches('-') == "/var/lib/opsbox/opsbox.db"),
        "proxy must hide the database"
    );
}
