//! Access (docs/BACKEND_SPEC.md 6.4): who can log in. Linux accounts with their groups and
//! lock state, their SSH keys, and what the SSH daemon is configured to allow.
//!
//! Key files live in users' home folders, which those users control. Writing them as root
//! would let a planted symlink (`~/.ssh` -> `/etc`) redirect the write, so every read and
//! write of `authorized_keys` runs *as that user* (`setpriv`, falling back to `runuser`).
//! Whatever a user does to their own folder then only ever affects things they can already
//! write.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use nix::unistd::{User, geteuid};
use opsbox_proto::{
    AccessState, AccessUser, AuthorizedKey, PrivError, PrivErrorCode, PrivReply, SshStatus,
    SystemUser,
};
use sha2::{Digest, Sha256};

use super::exec::{Output, find, run, run_with};
use super::terminal;

const QUICK: Duration = Duration::from_secs(10);
/// Lowest user id of an ordinary account (`UID_MIN`; macOS, used in development, starts at 501).
const FIRST_USER_UID: u32 = if cfg!(target_os = "linux") { 1000 } else { 500 };
const SUDO_GROUPS: &[&str] = &["sudo", "wheel", "admin"];
/// Keys are read and rewritten as one unit; two changes at once would lose one of them.
static KEYS_LOCK: Mutex<()> = Mutex::new(());

fn err(code: PrivErrorCode, message: impl Into<String>) -> PrivError {
    PrivError {
        code,
        message: message.into(),
    }
}

// ------------------------------------------------------------------ SSH keys (pure)

const KEY_TYPES: &[&str] = &[
    "ssh-ed25519",
    "ssh-rsa",
    "ssh-dss",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "sk-ssh-ed25519@openssh.com",
    "sk-ecdsa-sha2-nistp256@openssh.com",
];

/// One parsed key line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyLine {
    pub options: Option<String>,
    pub key_type: String,
    pub comment: String,
    pub fingerprint: String,
    /// `ed25519`, `rsa 2048`, ...
    pub kind: String,
    pub weak: bool,
}

/// A length-prefixed field of the SSH wire format.
fn take_string<'a>(buf: &mut &'a [u8]) -> Option<&'a [u8]> {
    let len = u32::from_be_bytes(buf.get(..4)?.try_into().ok()?) as usize;
    let rest = buf.get(4..)?;
    let field = rest.get(..len)?;
    *buf = &rest[len..];
    Some(field)
}

/// Bits in a big-endian integer.
fn mpint_bits(n: &[u8]) -> usize {
    let n = match n.iter().position(|b| *b != 0) {
        Some(i) => &n[i..],
        None => return 0,
    };
    n.len() * 8 - n[0].leading_zeros() as usize
}

/// Describe a key from its base64 blob; `None` if the blob is not the declared type.
fn describe(key_type: &str, b64: &str) -> Option<(String, String, bool)> {
    let blob = STANDARD.decode(b64).ok()?;
    let mut cur = blob.as_slice();
    if take_string(&mut cur)? != key_type.as_bytes() {
        return None;
    }
    let fingerprint = format!("SHA256:{}", STANDARD_NO_PAD.encode(Sha256::digest(&blob)));
    let (kind, weak) = match key_type {
        "ssh-ed25519" => ("ed25519".to_owned(), false),
        "ssh-rsa" => {
            let _e = take_string(&mut cur)?;
            let bits = mpint_bits(take_string(&mut cur)?);
            (format!("rsa {bits}"), bits < 2048)
        }
        "ssh-dss" => ("dsa".to_owned(), true),
        t if t.starts_with("ecdsa-sha2-") => (t.replacen("ecdsa-sha2-", "ecdsa-", 1), false),
        t if t.starts_with("sk-ssh-ed25519") => ("ed25519-sk".to_owned(), false),
        t if t.starts_with("sk-ecdsa") => ("ecdsa-sk".to_owned(), false),
        other => (other.to_owned(), true),
    };
    Some((fingerprint, kind, weak))
}

/// The first whitespace-separated token of `s` outside double quotes, and the rest.
fn split_option_token(s: &str) -> (&str, &str) {
    let mut in_quote = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_quote = !in_quote,
            c if c.is_whitespace() && !in_quote => return (&s[..i], s[i..].trim_start()),
            _ => {}
        }
    }
    (s, "")
}

/// Parse one line of an `authorized_keys` file. Blank lines, comments and lines that are not
/// keys give `None`.
pub fn parse_line(line: &str) -> Option<KeyLine> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let first_is_type = |s: &str| {
        KEY_TYPES
            .iter()
            .any(|t| s.split_whitespace().next() == Some(*t))
    };
    let (options, rest) = if first_is_type(line) {
        (None, line)
    } else {
        let (opts, rest) = split_option_token(line);
        (Some(opts.to_owned()), rest)
    };
    let mut parts = rest.splitn(3, char::is_whitespace);
    let key_type = parts.next()?;
    if !KEY_TYPES.contains(&key_type) {
        return None;
    }
    let b64 = parts.next()?;
    let comment = parts.next().unwrap_or("").trim().to_owned();
    let (fingerprint, kind, weak) = describe(key_type, b64)?;
    Some(KeyLine {
        options,
        key_type: key_type.to_owned(),
        comment,
        fingerprint,
        kind,
        weak,
    })
}

pub fn parse_keys(text: &str) -> Vec<KeyLine> {
    text.lines().filter_map(parse_line).collect()
}

/// Check a key a person pasted and return the single line to store.
pub fn vet_new_key(pasted: &str) -> Result<(String, KeyLine), PrivError> {
    let line = pasted.trim();
    if line.contains("PRIVATE KEY") {
        return Err(err(
            PrivErrorCode::Invalid,
            "That is a private key. Never paste it here. Paste the .pub file instead.",
        ));
    }
    if line.contains('\n') || line.contains('\r') {
        return Err(err(
            PrivErrorCode::Invalid,
            "Add one key at a time (one line).",
        ));
    }
    if line.chars().any(|c| c.is_control()) || line.len() > 8192 {
        return Err(err(
            PrivErrorCode::Invalid,
            "That key contains characters that are not allowed.",
        ));
    }
    // Restrictions are not accepted here: a pasted line with `command=...` could be used to
    // sneak behaviour past the person adding it.
    let starts_with_type = KEY_TYPES
        .iter()
        .any(|t| line.split_whitespace().next() == Some(*t));
    let parsed = if starts_with_type {
        parse_line(line)
    } else {
        None
    };
    let Some(parsed) = parsed else {
        return Err(err(
            PrivErrorCode::Invalid,
            "This is not a public key. It should start with ssh-ed25519 or ssh-rsa.",
        ));
    };
    if parsed.weak {
        return Err(err(
            PrivErrorCode::Invalid,
            format!(
                "This key type ({}) is too weak to accept. Use ed25519, or RSA of at least 2048 bits.",
                parsed.kind
            ),
        ));
    }
    Ok((line.to_owned(), parsed))
}

/// Append `line` to the file's text.
pub fn add_to(existing: &str, line: &str, parsed: &KeyLine) -> Result<String, PrivError> {
    if parse_keys(existing)
        .iter()
        .any(|k| k.fingerprint == parsed.fingerprint)
    {
        return Err(err(
            PrivErrorCode::Conflict,
            "This key is already authorised for that user.",
        ));
    }
    let mut out = existing.to_owned();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(line);
    out.push('\n');
    Ok(out)
}

/// The text without any line for `fingerprint`, and how many were dropped. Other lines
/// (comments, unknown formats) are kept exactly as they were.
pub fn remove_from(existing: &str, fingerprint: &str) -> (String, u32) {
    let mut removed = 0;
    let mut out = String::new();
    for line in existing.lines() {
        if parse_line(line).is_some_and(|k| k.fingerprint == fingerprint) {
            removed += 1;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    (out, removed)
}

// ------------------------------------------------------------------ accounts (pure)

/// `/etc/group`: name -> (gid, members).
pub fn parse_groups(text: &str) -> Vec<(String, u32, Vec<String>)> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.trim_end().split(':').collect();
            if f.len() < 4 || f[0].is_empty() {
                return None;
            }
            let members = f[3]
                .split(',')
                .filter(|m| !m.is_empty())
                .map(str::to_owned)
                .collect();
            Some((f[0].to_owned(), f[2].parse().ok()?, members))
        })
        .collect()
}

/// The groups a user belongs to (primary group first).
pub fn groups_of(name: &str, gid: u32, groups: &[(String, u32, Vec<String>)]) -> Vec<String> {
    let mut out: Vec<String> = groups
        .iter()
        .filter(|(_, g, _)| *g == gid)
        .map(|(n, _, _)| n.clone())
        .collect();
    out.extend(
        groups
            .iter()
            .filter(|(_, g, m)| *g != gid && m.iter().any(|x| x == name))
            .map(|(n, _, _)| n.clone()),
    );
    out
}

/// `/etc/shadow`: user -> account expiry in days since 1970 (field 8), when set.
pub fn parse_shadow_expiry(text: &str) -> HashMap<String, i64> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            let days: i64 = f.get(7)?.trim().parse().ok()?;
            Some((f.first()?.to_string(), days))
        })
        .collect()
}

/// An account is locked when its expiry date has passed (`chage -E 0` sets 1970-01-01).
pub fn is_locked(expiry_days: Option<i64>, today_days: i64) -> bool {
    expiry_days.is_some_and(|d| d >= 0 && d <= today_days)
}

/// `lastlog -u name`: `ubuntu  pts/0  203.0.113.7  Mon Nov 17 07:00:00 +0000 2026`, or
/// `**Never logged in**`. Gives `Nov 17 07:00`, `never` or an empty string.
pub fn parse_lastlog(text: &str) -> String {
    let Some(row) = text.lines().nth(1) else {
        return String::new();
    };
    if row.contains("Never logged in") {
        return "never".into();
    }
    let t: Vec<&str> = row.split_whitespace().collect();
    let days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    match t.iter().position(|w| days.contains(w)) {
        Some(i) if t.len() > i + 3 => format!(
            "{} {} {}",
            t[i + 1],
            t[i + 2],
            t[i + 3].get(..5).unwrap_or(t[i + 3])
        ),
        _ => String::new(),
    }
}

/// `sshd -T`: lowercase `key value` lines.
pub fn parse_sshd_t(text: &str) -> (Option<u16>, Option<bool>, Option<String>) {
    let (mut port, mut pw, mut root) = (None, None, None);
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match (it.next(), it.next()) {
            (Some("port"), Some(v)) if port.is_none() => port = v.parse().ok(),
            (Some("passwordauthentication"), Some(v)) => pw = Some(v == "yes"),
            (Some("permitrootlogin"), Some(v)) => {
                root = Some(v.replace("without-password", "prohibit-password"))
            }
            _ => {}
        }
    }
    (port, pw, root)
}

// ------------------------------------------------------------------ system access

fn today_days() -> i64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    i64::try_from(secs / 86400).unwrap_or(0)
}

fn all_users() -> Result<Vec<SystemUser>, PrivError> {
    match terminal::list()? {
        PrivReply::Users(u) => Ok(u),
        _ => Ok(Vec::new()),
    }
}

/// Accounts that can log in: root and ordinary users with a real shell.
pub(crate) fn login_users() -> Result<Vec<SystemUser>, PrivError> {
    Ok(all_users()?
        .into_iter()
        .filter(|u| u.login && (u.uid == 0 || u.uid >= FIRST_USER_UID) && u.uid < 60000)
        .collect())
}

fn find_user(name: &str) -> Result<SystemUser, PrivError> {
    if !terminal::valid_user_name(name) {
        return Err(err(
            PrivErrorCode::Invalid,
            "That is not a valid user name.",
        ));
    }
    login_users()?
        .into_iter()
        .find(|u| u.name == name)
        .ok_or_else(|| {
            err(
                PrivErrorCode::NotFound,
                format!("There is no user “{name}” that can log in."),
            )
        })
}

/// Run a shell snippet as `user`, never as root. `HOME` is the account's home folder.
fn run_as(user: &SystemUser, script: &str, stdin: Option<&str>) -> Result<Output, PrivError> {
    let env = [("HOME", user.home.as_str())];
    let internal = |m: &str| err(PrivErrorCode::Internal, m.to_owned());
    let out = if geteuid().is_root() {
        let (uid, gid) = (user.uid.to_string(), user.gid.to_string());
        if let Some(sp) = find(&["/usr/bin/setpriv"]) {
            run_with(
                &sp,
                &[
                    &format!("--reuid={uid}"),
                    &format!("--regid={gid}"),
                    "--init-groups",
                    "--",
                    "/bin/sh",
                    "-c",
                    script,
                ],
                QUICK,
                stdin,
                &env,
            )
        } else if let Some(ru) = find(&["/usr/sbin/runuser", "/sbin/runuser"]) {
            run_with(
                &ru,
                &["-u", &user.name, "--", "/bin/sh", "-c", script],
                QUICK,
                stdin,
                &env,
            )
        } else {
            return Err(err(
                PrivErrorCode::Unsupported,
                "Neither setpriv nor runuser is installed, so keys cannot be changed safely.",
            ));
        }
    } else {
        let me = User::from_uid(geteuid()).ok().flatten().map(|u| u.name);
        if me.as_deref() != Some(user.name.as_str()) {
            return Err(err(
                PrivErrorCode::Refused,
                "opsbox is not running as root, so only its own user's keys can be changed.",
            ));
        }
        run_with("/bin/sh", &["-c", script], QUICK, stdin, &env)
    };
    out.ok_or_else(|| internal("The shell did not run."))
}

pub(crate) fn read_key_file(user: &SystemUser) -> Result<String, PrivError> {
    // A missing file is just "no keys".
    let out = run_as(
        user,
        r#"f="$HOME/.ssh/authorized_keys"; if [ -f "$f" ]; then cat "$f"; fi"#,
        None,
    )?;
    Ok(out.stdout)
}

fn write_key_file(user: &SystemUser, text: &str) -> Result<(), PrivError> {
    const SCRIPT: &str = r#"umask 077
d="$HOME/.ssh"
mkdir -p "$d" || exit 1
chmod 700 "$d" || exit 1
t="$d/.authorized_keys.opsbox.$$"
cat > "$t" || { rm -f "$t"; exit 1; }
chmod 600 "$t" && mv -f "$t" "$d/authorized_keys""#;
    let out = run_as(user, SCRIPT, Some(text))?;
    if out.ok() {
        Ok(())
    } else {
        Err(err(
            PrivErrorCode::Internal,
            format!("Could not write the keys file: {}", out.combined().trim()),
        ))
    }
}

pub(crate) fn add_key(name: &str, pasted: &str) -> Result<String, PrivError> {
    let (line, parsed) = vet_new_key(pasted)?;
    let user = find_user(name)?;
    let _g = KEYS_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let updated = add_to(&read_key_file(&user)?, &line, &parsed)?;
    write_key_file(&user, &updated)?;
    Ok(parsed.fingerprint)
}

pub(crate) fn remove_key(name: &str, fingerprint: &str) -> Result<u32, PrivError> {
    if !fingerprint.starts_with("SHA256:") || fingerprint.len() > 80 {
        return Err(err(
            PrivErrorCode::Invalid,
            "That is not a key fingerprint.",
        ));
    }
    let user = find_user(name)?;
    let _g = KEYS_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let existing = read_key_file(&user)?;
    let (updated, removed) = remove_from(&existing, fingerprint);
    if removed == 0 {
        return Err(err(
            PrivErrorCode::NotFound,
            "That key is not there any more.",
        ));
    }
    write_key_file(&user, &updated)?;
    Ok(removed)
}

// ------------------------------------------------------------------ lock / unlock

/// Whether locking `target` is allowed, given everyone's current state. Pure.
pub fn lock_refusal(target: &AccessUser, all: &[AccessUser], service_user: &str) -> Option<String> {
    if target.uid == 0 {
        return Some("root cannot be locked here: scheduled jobs and system tools run as root. Turn off root login over SSH in Security instead.".into());
    }
    if target.name == service_user {
        return Some(format!(
            "{service_user} runs opsbox itself. Locking it would stop the panel."
        ));
    }
    if target.sudo && !target.locked {
        let others = all
            .iter()
            .filter(|u| u.name != target.name && u.sudo && !u.locked)
            .count();
        if others == 0 {
            return Some(format!(
                "{} is the only account that can use sudo. Locking it would leave nobody able to administer this server.",
                target.name
            ));
        }
    }
    None
}

pub(crate) fn set_lock(name: &str, lock: bool) -> Result<(), PrivError> {
    let state = collect_users()?;
    let target = state.iter().find(|u| u.name == name).ok_or_else(|| {
        err(
            PrivErrorCode::NotFound,
            format!("There is no user “{name}” that can log in."),
        )
    })?;
    if lock && let Some(why) = lock_refusal(target, &state, "opsbox") {
        return Err(err(PrivErrorCode::Refused, why));
    }
    let chage = find(&["/usr/bin/chage", "/usr/sbin/chage"]).ok_or_else(|| {
        err(
            PrivErrorCode::Unsupported,
            "The chage tool is not installed, so accounts cannot be locked here.",
        )
    })?;
    // Expiring the account stops every login, including key logins; locking only the
    // password would not. It also cannot make an account that never had a password
    // passwordless, which `usermod -U` refuses to do.
    let date = if lock { "0" } else { "-1" };
    let out = run(&chage, &["-E", date, name], QUICK)
        .ok_or_else(|| err(PrivErrorCode::Internal, "chage did not run."))?;
    if !out.ok() {
        return Err(err(
            PrivErrorCode::Internal,
            format!("Could not change the account: {}", out.combined().trim()),
        ));
    }
    Ok(())
}

// ------------------------------------------------------------------ listing

pub(crate) fn collect_users() -> Result<Vec<AccessUser>, PrivError> {
    let users = login_users()?;
    let groups = std::fs::read_to_string("/etc/group")
        .map(|t| parse_groups(&t))
        .unwrap_or_default();
    let expiry = std::fs::read_to_string("/etc/shadow")
        .map(|t| parse_shadow_expiry(&t))
        .unwrap_or_default();
    let today = today_days();
    Ok(users
        .into_iter()
        .map(|u| {
            let g = groups_of(&u.name, u.gid, &groups);
            let last = run("/usr/bin/lastlog", &["-u", &u.name], QUICK)
                .filter(Output::ok)
                .map(|o| parse_lastlog(&o.stdout))
                .unwrap_or_default();
            AccessUser {
                sudo: g.iter().any(|x| SUDO_GROUPS.contains(&x.as_str())),
                locked: is_locked(expiry.get(&u.name).copied(), today),
                groups: g,
                last_login: last,
                name: u.name,
                uid: u.uid,
            }
        })
        .collect())
}

fn ssh_status() -> SshStatus {
    let Some(sshd) = find(&["/usr/sbin/sshd", "/usr/bin/sshd"]) else {
        return SshStatus::default();
    };
    let (port, pw, root) = run(&sshd, &["-T"], QUICK)
        .filter(Output::ok)
        .map(|o| parse_sshd_t(&o.stdout))
        .unwrap_or((None, None, None));
    let active = ["ssh", "sshd"].iter().any(|unit| {
        run("/usr/bin/systemctl", &["is-active", unit], QUICK)
            .is_some_and(|o| o.stdout.trim() == "active")
    });
    SshStatus {
        installed: true,
        active,
        port: port.unwrap_or(22),
        password_login: pw,
        root_login: root,
    }
}

pub(crate) fn list() -> Result<PrivReply, PrivError> {
    let users = collect_users()?;
    let mut keys = Vec::new();
    for u in login_users()? {
        // A user whose keys cannot be read is shown without keys rather than failing the page.
        if let Ok(text) = read_key_file(&u) {
            keys.extend(parse_keys(&text).into_iter().map(|k| AuthorizedKey {
                user: u.name.clone(),
                name: k.comment,
                kind: k.kind,
                fingerprint: k.fingerprint,
                options: k.options,
                weak: k.weak,
            }));
        }
    }
    Ok(PrivReply::Access(AccessState {
        users,
        keys,
        ssh: ssh_status(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Throwaway keys made with ssh-keygen for these tests; the fingerprints are what
    // `ssh-keygen -l` printed for them.
    const ED: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAILccwtaElMJNNkowDtiI0r07uTtMUL77z/R1AI9LiDmm khang@laptop";
    const ED_FP: &str = "SHA256:+OaUMcAeJ8koX3oIly+aNJGfr/gIIw0yM4eahC7sA4s";
    const RSA2048: &str = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDUFw3N8g6OaiFsTLXVkU3kcWvn1hYnz6JKodeApElOguS87AtKjwIXbcpkyk+97Hk5fXy/fM3NSIrwtGeE7+qwsKhtywliYZar/nahP0P8wNY9zATeuQ5s/BM3CVRmbmBaGJcMbBDVZdwo0MP0k0WFAV+MQ4SFWfCE2tiAJiyAYgO95VVVfAI5cXBOk5B2Xs+6qzQTkQQttC9S4AtVCe93KKiDpI+VQPJQglGnbBRLopVDDX9aIf9+FK27yXc7v41BjVVU5s/JkPjDk7ikpNOazp/ogdsz6BwJ87UH6fTr47WDjn4peb2aqgU69Iuys8B8lVdUVQIbVnxdEMCRwM3D ci deploy";
    const RSA2048_FP: &str = "SHA256:EuJIKnLbNiUivH3/sLWFU2Ckm5nTM6b0dAxUmE9Sa9I";
    const RSA1024: &str = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAAAgQDJ4JSCeP0JTk884OyEP4c81PZLTZttNWxxftE/dMqG7rX14udDr3zuSd7FjWlj7DF3oZh7yaFirUP0TAFdKVn9S9X3kmiWDmkdMZXDi0unZGfjnP334kbkPVM9evtd4DoYjCkGNHNed6ldbIf5n0XBQn2Zi7QDUaZbPvinTpwWdQ== old";
    const EC: &str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBKQQYbyA0kLEZVK/OuK9eDg+pvRmJxRdZSqG8B52K6zLT178312IdayQrI1qBLbg8vdwBDVnrcTkDDi1H8L6k+k= ec";
    const EC_FP: &str = "SHA256:FfORCZGXgkwsc4zHFKMoW76M9xu1k7pBPujg6i2mvUs";

    #[test]
    fn fingerprints_and_kinds_match_ssh_keygen() {
        let k = parse_line(ED).unwrap();
        assert_eq!(
            (
                k.fingerprint.as_str(),
                k.kind.as_str(),
                k.comment.as_str(),
                k.weak
            ),
            (ED_FP, "ed25519", "khang@laptop", false)
        );
        let r = parse_line(RSA2048).unwrap();
        assert_eq!(
            (r.fingerprint.as_str(), r.kind.as_str(), r.weak),
            (RSA2048_FP, "rsa 2048", false)
        );
        assert_eq!(r.comment, "ci deploy", "the comment may contain spaces");
        let w = parse_line(RSA1024).unwrap();
        assert_eq!((w.kind.as_str(), w.weak), ("rsa 1024", true));
        let e = parse_line(EC).unwrap();
        assert_eq!(
            (e.fingerprint.as_str(), e.kind.as_str()),
            (EC_FP, "ecdsa-nistp256")
        );
    }

    #[test]
    fn options_comments_and_junk() {
        let line = format!(r#"command="git-shell -c \"$SSH_ORIGINAL_COMMAND\"",no-pty {ED}"#);
        let k = parse_line(&line).unwrap();
        assert!(k.options.as_deref().unwrap().starts_with("command="));
        assert_eq!(k.fingerprint, ED_FP);
        for junk in [
            "",
            "   ",
            "# a comment",
            "ssh-ed25519",
            "ssh-ed25519 not-base64!!",
            "ssh-rsa AAAAC3NzaC1lZDI1NTE5AAAAILccwtaElMJNNkowDtiI0r07uTtMUL77z/R1AI9LiDmm x",
            "hello world",
        ] {
            assert!(parse_line(junk).is_none(), "{junk:?}");
        }
        assert_eq!(
            parse_keys(&format!("# keys\n{ED}\n\nnonsense\n{EC}\n")).len(),
            2
        );
    }

    #[test]
    fn pasted_keys_are_vetted() {
        assert!(vet_new_key(&format!("  {ED}\n")).is_ok());
        for (bad, word) in [
            ("-----BEGIN OPENSSH PRIVATE KEY-----\nabc", "private key"),
            (&format!("{ED}\n{EC}") as &str, "one key at a time"),
            ("hello", "not a public key"),
            (&format!("no-pty {ED}") as &str, "not a public key"),
            (RSA1024, "too weak"),
            (&format!("{ED}\x07") as &str, "characters"),
        ] {
            let e = vet_new_key(bad).unwrap_err();
            assert_eq!(e.code, PrivErrorCode::Invalid, "{bad:?}");
            assert!(
                e.message.to_lowercase().contains(word),
                "{bad:?}: {}",
                e.message
            );
        }
    }

    #[test]
    fn adding_and_removing_keeps_everything_else_exactly() {
        let (line, parsed) = vet_new_key(ED).unwrap();
        let existing = format!("# my keys\n{RSA2048}"); // no trailing newline
        let after = add_to(&existing, &line, &parsed).unwrap();
        assert_eq!(after, format!("# my keys\n{RSA2048}\n{ED}\n"));
        assert_eq!(
            add_to(&after, &line, &parsed).unwrap_err().code,
            PrivErrorCode::Conflict,
            "no duplicates"
        );
        assert_eq!(add_to("", &line, &parsed).unwrap(), format!("{ED}\n"));
        let (rest, n) = remove_from(&after, ED_FP);
        assert_eq!(
            (rest.as_str(), n),
            (format!("# my keys\n{RSA2048}\n").as_str(), 1)
        );
        assert_eq!(remove_from(&after, EC_FP).1, 0);
        // The same key listed twice (by hand) goes in one removal.
        assert_eq!(
            remove_from(&format!("{ED}\n{ED}\n"), ED_FP),
            (String::new(), 2)
        );
    }

    #[test]
    fn groups_sudo_and_expiry() {
        let groups = parse_groups(
            "root:x:0:\nsudo:x:27:ubuntu,deploy\nubuntu:x:1000:\ndeploy:x:1001:\ndocker:x:998:deploy\nbroken\n",
        );
        assert_eq!(groups_of("ubuntu", 1000, &groups), ["ubuntu", "sudo"]);
        assert_eq!(
            groups_of("deploy", 1001, &groups),
            ["deploy", "sudo", "docker"]
        );
        let sh = parse_shadow_expiry(
            "root:*:19000:0:99999:7:::\nubuntu:!:19000:0:99999:7::0:\ndeploy:$6$x:19000:0:99999:7:::\nold:!:19000:0:99999:7::20000:\nfuture:!:19000:0:99999:7::99999:\n",
        );
        let today = 20500;
        assert!(
            is_locked(sh.get("ubuntu").copied(), today),
            "chage -E 0 expires at 1970-01-01"
        );
        assert!(!is_locked(sh.get("deploy").copied(), today), "no expiry");
        assert!(
            is_locked(sh.get("old").copied(), today),
            "expired in the past"
        );
        assert!(!is_locked(sh.get("future").copied(), today));
        assert!(!is_locked(Some(-1), today), "-1 means never");
    }

    #[test]
    fn lastlog_and_sshd_output() {
        let l = "Username         Port     From             Latest\nubuntu           pts/0    203.0.113.7      Mon Nov 17 07:00:12 +0000 2026\n";
        assert_eq!(parse_lastlog(l), "Nov 17 07:00");
        let console = "Username         Port     From             Latest\nubuntu                                     Tue Mar  2 03:40:01 +0000 2026\n";
        assert_eq!(parse_lastlog(console), "Mar 2 03:40");
        assert_eq!(
            parse_lastlog(
                "Username Port From Latest\ndeploy                                     **Never logged in**\n"
            ),
            "never"
        );
        assert_eq!(parse_lastlog(""), "");
        let (port, pw, root) = parse_sshd_t(
            "port 2222\npasswordauthentication no\npermitrootlogin without-password\nport 22\n",
        );
        assert_eq!(
            (port, pw, root.as_deref()),
            (Some(2222), Some(false), Some("prohibit-password"))
        );
    }

    fn u(name: &str, uid: u32, sudo: bool, locked: bool) -> AccessUser {
        AccessUser {
            name: name.into(),
            uid,
            groups: vec![],
            sudo,
            locked,
            last_login: String::new(),
        }
    }

    #[test]
    fn what_may_not_be_locked() {
        let all = vec![
            u("root", 0, false, false),
            u("ubuntu", 1000, true, false),
            u("deploy", 1001, false, false),
            u("opsbox", 998, false, false),
        ];
        assert!(
            lock_refusal(&all[0], &all, "opsbox")
                .unwrap()
                .contains("root")
        );
        assert!(
            lock_refusal(&all[3], &all, "opsbox")
                .unwrap()
                .contains("runs opsbox")
        );
        assert!(
            lock_refusal(&all[1], &all, "opsbox")
                .unwrap()
                .contains("only account that can use sudo")
        );
        assert!(lock_refusal(&all[2], &all, "opsbox").is_none());
        let mut two = all.clone();
        two.push(u("admin2", 1002, true, false));
        assert!(
            lock_refusal(&two[1], &two, "opsbox").is_none(),
            "another administrator remains"
        );
        two[4].locked = true;
        assert!(
            lock_refusal(&two[1], &two, "opsbox").is_some(),
            "a locked one does not count"
        );
    }

    /// The current account with a throwaway home folder, so the real script runs without
    /// touching anything of the person running the tests.
    fn me_with_home(home: &std::path::Path) -> SystemUser {
        let me = User::from_uid(geteuid()).unwrap().unwrap();
        SystemUser {
            name: me.name,
            uid: me.uid.as_raw(),
            gid: me.gid.as_raw(),
            home: home.to_str().unwrap().to_owned(),
            shell: "/bin/sh".into(),
            login: true,
        }
    }

    #[test]
    fn the_key_file_is_written_and_read_back_with_safe_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let user = me_with_home(home.path());
        assert_eq!(
            read_key_file(&user).unwrap(),
            "",
            "no file yet means no keys"
        );
        write_key_file(&user, &format!("{ED}\n")).unwrap();
        write_key_file(&user, &format!("{ED}\n{EC}\n")).unwrap();
        assert_eq!(parse_keys(&read_key_file(&user).unwrap()).len(), 2);
        let ssh = home.path().join(".ssh");
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&ssh), 0o700);
        assert_eq!(mode(&ssh.join("authorized_keys")), 0o600);
        // No temporary file is left behind.
        let names: Vec<_> = std::fs::read_dir(&ssh)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["authorized_keys"]);
    }

    #[test]
    fn an_existing_loose_ssh_folder_is_tightened_and_a_failed_write_keeps_the_old_file() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let user = me_with_home(home.path());
        let ssh = home.path().join(".ssh");
        std::fs::create_dir(&ssh).unwrap();
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(ssh.join("authorized_keys"), format!("{ED}\n")).unwrap();
        write_key_file(&user, &format!("{ED}\n{EC}\n")).unwrap();
        assert_eq!(
            std::fs::metadata(&ssh).unwrap().permissions().mode() & 0o777,
            0o700
        );
        // A home that cannot be written to fails loudly and leaves the file alone.
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o500)).unwrap();
        let before = std::fs::read_to_string(ssh.join("authorized_keys")).unwrap();
        let result = write_key_file(&user, "garbage\n");
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o700)).unwrap();
        // (As root a read-only folder is still writable, so only check when it failed.)
        if result.is_err() {
            assert_eq!(
                std::fs::read_to_string(ssh.join("authorized_keys")).unwrap(),
                before
            );
        }
    }

    #[test]
    fn invalid_names_never_reach_a_command() {
        assert_eq!(
            remove_key("a;b", "SHA256:x").unwrap_err().code,
            PrivErrorCode::Invalid
        );
        assert_eq!(add_key("-r", ED).unwrap_err().code, PrivErrorCode::Invalid);
        assert_eq!(
            remove_key("someone", "nope").unwrap_err().code,
            PrivErrorCode::Invalid
        );
        assert_eq!(
            set_lock("a b", true).unwrap_err().code,
            PrivErrorCode::NotFound
        );
    }
}
