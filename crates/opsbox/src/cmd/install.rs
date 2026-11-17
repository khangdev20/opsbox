use crate::cli::InstallArgs;

/// Phase 0 stub: report the plan, change nothing.
pub fn run(args: &InstallArgs) {
    println!("opsbox install (Phase 0 stub: nothing was changed)");
    println!();
    println!("When implemented, this command will, idempotently:");
    println!("  1. create the system user and group `opsbox` (no shell, no login)");
    println!("  2. create /etc/opsbox, /var/lib/opsbox and /var/lib/opsbox/bin");
    println!("  3. write /etc/opsbox/config.toml with defaults if it does not exist");
    println!("  4. install and enable opsbox-priv.service and opsbox.service");
    println!("  5. start both and print the setup URL");
    println!();
    println!("Until then, install.sh performs steps 1-4 itself.");
    if args.pair.is_some() || args.web || args.channel.is_some() {
        println!();
        println!("Options received (not acted on yet):");
        if args.pair.is_some() {
            // Never echo the secret itself.
            println!("  --pair <secret given>");
        }
        if args.web {
            println!("  --web");
        }
        if let Some(channel) = &args.channel {
            println!("  --channel {channel}");
        }
    }
}
