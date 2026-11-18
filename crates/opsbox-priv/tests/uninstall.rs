//! Plan builder cases and dry-run path removals (prompt 16j).

use std::fs;
use std::path::Path;

use opsbox_priv::uninstall::{
    Layout, PlanContext, apply_path_removals, build_plan, discover_timers,
};
use opsbox_proto::UninstallAdopted;
use tempfile::tempdir;

fn seed(root: &Path) -> Layout {
    let layout = Layout::under(root);
    fs::create_dir_all(&layout.unit_dir).unwrap();
    fs::create_dir_all(&layout.slot_dir).unwrap();
    fs::create_dir_all(&layout.state_dir).unwrap();
    fs::create_dir_all(&layout.conf_dir).unwrap();
    fs::create_dir_all(&layout.backup_dir).unwrap();
    fs::create_dir_all(layout.prefix_bin.parent().unwrap()).unwrap();
    fs::write(&layout.prefix_bin, b"fake").unwrap();
    fs::write(layout.slot_dir.join("opsbox-0.1.0"), b"bin").unwrap();
    for u in [
        "opsbox.service",
        "opsbox-priv.service",
        "opsbox-proxy.service",
        "opsbox-watchdog.service",
        "opsbox-cleanup.timer",
        "opsbox-cleanup.service",
    ] {
        fs::write(layout.unit_dir.join(u), b"unit").unwrap();
    }
    fs::write(layout.unit_dir.join("opsbox-job-abcd1234.timer"), b"t").unwrap();
    fs::write(layout.unit_dir.join("opsbox-job-abcd1234.service"), b"s").unwrap();
    fs::write(layout.conf_dir.join("config.toml"), b"# cfg").unwrap();
    fs::write(layout.state_dir.join("opsbox.db"), b"db").unwrap();
    layout
}

#[test]
fn plan_builder_no_proxy_domains() {
    let plan = build_plan(
        &Layout::production(),
        &PlanContext {
            purge: false,
            ..Default::default()
        },
    );
    assert!(plan.domains.is_empty());
    assert!(plan.kept.iter().any(|k| k.contains("/var/lib/opsbox")));
    assert!(!plan.purge);
}

#[test]
fn plan_builder_with_domains() {
    let plan = build_plan(
        &Layout::production(),
        &PlanContext {
            purge: false,
            domains: vec!["app.example.com".into()],
            ..Default::default()
        },
    );
    assert_eq!(plan.domains, vec!["app.example.com".to_owned()]);
    assert!(plan.warnings.iter().any(|w| w.contains("80/443")));
}

#[test]
fn plan_builder_with_adopted_web_server() {
    let plan = build_plan(
        &Layout::production(),
        &PlanContext {
            purge: false,
            domains: vec!["old.example.com".into()],
            adopted: Some(UninstallAdopted {
                holder: "nginx.service".into(),
                domains: vec!["old.example.com".into()],
                will_restore: true,
            }),
            ..Default::default()
        },
    );
    assert!(
        plan.kept
            .iter()
            .any(|k| k.contains("nginx.service") && k.contains("restored"))
    );
}

#[test]
fn dry_run_keep_data_removes_units_and_keeps_state() {
    let tmp = tempdir().unwrap();
    let layout = seed(tmp.path());
    let extra = discover_timers(&layout.unit_dir);
    assert!(extra.iter().any(|t| t == "opsbox-job-abcd1234.timer"));

    apply_path_removals(&layout, false, &extra, false).unwrap();

    assert!(!layout.prefix_bin.exists());
    assert!(!layout.unit_dir.join("opsbox.service").exists());
    assert!(!layout.unit_dir.join("opsbox-job-abcd1234.timer").exists());
    assert!(!layout.slot_dir.exists());
    assert!(layout.state_dir.exists());
    assert!(layout.conf_dir.exists());
    assert!(layout.backup_dir.exists());
}

#[test]
fn dry_run_purge_removes_data_dirs() {
    let tmp = tempdir().unwrap();
    let layout = seed(tmp.path());
    let extra = discover_timers(&layout.unit_dir);
    apply_path_removals(&layout, true, &extra, false).unwrap();
    assert!(!layout.state_dir.exists());
    assert!(!layout.conf_dir.exists());
    assert!(!layout.backup_dir.exists());
}

#[test]
fn dry_run_flag_lists_without_deleting() {
    let tmp = tempdir().unwrap();
    let layout = seed(tmp.path());
    let extra = discover_timers(&layout.unit_dir);
    let listed = apply_path_removals(&layout, true, &extra, true).unwrap();
    assert!(!listed.is_empty());
    assert!(layout.prefix_bin.exists());
    assert!(layout.state_dir.exists());
}
