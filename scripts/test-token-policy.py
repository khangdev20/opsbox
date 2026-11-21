#!/usr/bin/env python3
"""Require a reviewed token decision for every live mutating router handler.

Direct deny_token calls are decisions. Remaining handlers must be explicitly
reviewed below; a new route cannot silently inherit Full token permission.
Run through tokens_http so the ordinary Cargo API tests enforce this inventory.
"""
import importlib.util
from pathlib import Path
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("token_policy_index", ROOT / "scripts/gen-index.py")
GEN = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = GEN
SPEC.loader.exec_module(GEN)

# These operations accept tokens subject to role, scope, ownership and validation.
ALLOW = {
    "alerts": "silence quiet set_rule",  # Notification preferences only.
    "backups": "delete_destination delete_job update_job check create_destination test_destination set_default_destination test_saved_destination create_job run_job prune run_restore_test_all run_all test_snapshot_restore",  # Validated backup operations; live restore is session-only.
    "cleanup": "run set_auto",  # Fixed cleanup categories.
    "deploy": "update_service create_service redeploy rollback",  # Service policy, including elevated Compose checks.
    "docker": "container_action",  # Existing containers, fixed actions; no create/exec API.
    "domains": "remove_domain update_domain issue_cert check_dns make_primary add_domain",
    "github": "disconnect delete_deploy_key complete_manifest create_deploy_key sync_installation start_manifest inspect_public_repo sync_webhook",
    "metrics": "read_all_attention read_attention snooze_attention",
    "panel": "remove_domain issue_cert check_dns",  # Domain and certificate management.
    "templates": "create_template_instance",  # Fixed catalog; no caller-supplied Compose.
    "update": "check",  # Release lookup only.
}
# Public bootstrapping/credential flows have their own setup/invite/TOTP checks.
PUBLIC = {
    "auth": "login totp_confirm totp_enroll",
    "setup": "admin claim confirm",
    "users": "claim",
    "github": "webhook",  # Signature checked by the webhook handler.
}
# Session-only policy delegated to a shared guard; assert the call remains.
DELEGATED = {
    "network::set_mode": "change", "network::takeover": "change", "network::undo": "change", "network::keep": "change",
    "network::move_web": "web_task", "network::undo_move": "web_task", "network::adopted_tls": "web_task",
    "databases::reveal_connection_string": "begin_reveal",
    "deploy::reveal_var": "begin_reveal",
    "backups::recovery_key": "via_token",
    "tools::index": "ToolGateway", "tools::gateway": "ToolGateway",
}


def check():
    routes, registry, _ = GEN.route_data(ROOT)
    decisions = {f"{module}::{name}" for group in (ALLOW, PUBLIC)
                 for module, names in group.items() for name in names.split()}
    failures = []
    mutations = [r for r in routes if r[1] not in ("GET", "GET (WS)", "HEAD", "OPTIONS")]
    assert mutations, "route scanner returned no mutations"
    for _, method, path, handler, _, auth in mutations:
        if "token refused" in auth or handler in decisions:
            continue
        _, fn, code, _ = registry[handler]
        if handler in DELEGATED and DELEGATED[handler] in code[fn.params:fn.end]:
            continue
        failures.append(f"{method} {path} ({handler}) has no token policy")
    assert not failures, "\n".join(failures)
    print(f"Token policy checked for {len(mutations)} mutating routes.")


if __name__ == "__main__":
    check()
