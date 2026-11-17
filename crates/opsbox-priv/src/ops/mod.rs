//! One module per privileged operation. Adding an operation means a new
//! `PrivOp` variant in `opsbox-proto`, a module here, and one line in [`dispatch`];
//! the compiler enforces the last two because the `match` is exhaustive.

pub(crate) mod cron;
pub(crate) mod exec;
mod facts;
mod firewall;
mod host_info;
mod hostset;
mod jobs;
mod journal;
mod metrics;
mod ping;
mod pkg;
mod totp;
mod units;

pub use firewall::{janitor_tick, restore_on_start};

pub use totp::{base32_encode, code_at, verify_at};

use opsbox_proto::{PrivError, PrivOp, PrivReply};

use crate::secrets::Secrets;

/// Blocking; the server runs it on a blocking thread.
pub(crate) fn dispatch(op: &PrivOp, secrets: &Secrets) -> Result<PrivReply, PrivError> {
    match op {
        PrivOp::Ping => Ok(ping::run()),
        PrivOp::HostInfo => host_info::run().map(PrivReply::HostInfo),
        PrivOp::MetricsSample { procs } => metrics::run(*procs),
        PrivOp::SystemFacts => Ok(PrivReply::SystemFacts(facts::collect())),
        PrivOp::PkgList => pkg::list().map(PrivReply::Pkgs),
        PrivOp::PkgInstall { which } => pkg::install(*which).map(PrivReply::Outcome),
        PrivOp::AutoUpdatesSet { on } => pkg::set_auto(*on).map(|()| PrivReply::Done),
        PrivOp::UnitList => units::list().map(PrivReply::Units),
        PrivOp::UnitAction { name, action } => {
            units::action(name, *action).map(|()| PrivReply::Done)
        }
        PrivOp::JournalQuery {
            unit,
            max_priority,
            text,
            limit,
            after,
        } => journal::query(
            unit.as_deref(),
            *max_priority,
            text.as_deref(),
            *limit,
            after.as_deref(),
        )
        .map(PrivReply::Journal),
        PrivOp::JobList => jobs::list().map(PrivReply::Jobs),
        PrivOp::JobAdd {
            name,
            cron,
            command,
        } => jobs::add(name, cron, command).map(PrivReply::JobId),
        PrivOp::JobRun { id } => jobs::run_now(id).map(|()| PrivReply::Done),
        PrivOp::JobRemove { id } => jobs::remove(id).map(|()| PrivReply::Done),
        PrivOp::FirewallList => firewall::list().map(PrivReply::Firewall),
        PrivOp::FirewallAdd {
            rule,
            caller_ip,
            confirm,
        } => firewall::add(rule, caller_ip.as_deref(), *confirm).map(PrivReply::FirewallChange),
        PrivOp::FirewallRemove {
            id,
            caller_ip,
            confirm,
        } => firewall::remove(id, caller_ip.as_deref(), *confirm).map(PrivReply::FirewallChange),
        PrivOp::FirewallConfirm => firewall::confirm().map(|()| PrivReply::Done),
        PrivOp::SshEmergency { caller_ip } => firewall::ssh_emergency(caller_ip).map(|until| {
            PrivReply::FirewallChange(opsbox_proto::FirewallChange {
                revert_at: Some(until),
            })
        }),
        PrivOp::Reboot { delay_min } => hostset::reboot(*delay_min).map(|()| PrivReply::Done),
        PrivOp::RebootCancel => hostset::reboot_cancel().map(|()| PrivReply::Done),
        PrivOp::SwapSet { size_mb } => hostset::set_swap(*size_mb).map(|()| PrivReply::Done),
        PrivOp::HostnameSet { name } => hostset::set_hostname(name).map(|()| PrivReply::Done),
        PrivOp::TimezoneSet { zone } => hostset::set_timezone(zone).map(|()| PrivReply::Done),
        PrivOp::TotpNewSecret { context } => totp::new_secret(secrets, context),
        PrivOp::TotpVerify {
            sealed,
            context,
            code,
            after_step,
        } => totp::verify(secrets, sealed, context, code.expose(), *after_step),
    }
}
