//! `opsbox uninstall [--keep-data | --purge] [--yes]`

use opsbox_priv::uninstall;

/// Run the uninstall. Returns process exit code semantics via `Result<(), i32>`.
pub fn run(purge: bool, yes: bool) -> Result<(), i32> {
    uninstall::run_cli(purge, yes)
}
