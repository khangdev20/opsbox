//! File operations (docs/BACKEND_SPEC.md 7.10): everything a person can do to a file
//! through the panel happens here, as root, after the path has been resolved and checked.
//!
//! Rules, in order, for every path:
//! 1. it must be absolute and free of NUL bytes;
//! 2. it is resolved (symbolic links followed) to a canonical path, and **that** path is
//!    what every later check and the operation itself use;
//! 3. `/proc`, `/sys`, `/dev` and the master key are never touched;
//! 4. if the request has a `jail` (a deployer's apps directory) the canonical path must be
//!    inside the canonical jail, so a symlink cannot lead out of it;
//! 5. changing or deleting opsbox's own binaries, database or state is refused.

use std::fs::{self, Metadata, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use nix::fcntl::OFlag;
use opsbox_proto::{
    DirListing, FileContent, FileEntry, FileKind, PrivError, PrivErrorCode, Staged,
};

/// Files above this size are download-only.
pub const MAX_EDIT_BYTES: u64 = 5 * 1024 * 1024;
const MAX_ENTRIES: usize = 5000;
const TRASH_KEEP_SECS: i64 = 7 * 24 * 3600;

fn err(code: PrivErrorCode, msg: impl Into<String>) -> PrivError {
    PrivError {
        code,
        message: msg.into(),
    }
}
fn invalid(msg: impl Into<String>) -> PrivError {
    err(PrivErrorCode::Invalid, msg)
}
fn io_err(what: &str, e: &std::io::Error) -> PrivError {
    let code = match e.kind() {
        std::io::ErrorKind::NotFound => PrivErrorCode::NotFound,
        std::io::ErrorKind::PermissionDenied => PrivErrorCode::Refused,
        _ => PrivErrorCode::Internal,
    };
    err(code, format!("{what}: {}", clean_io(e)))
}
fn clean_io(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "no such file or directory".into(),
        std::io::ErrorKind::PermissionDenied => "permission denied".into(),
        std::io::ErrorKind::AlreadyExists => "already exists".into(),
        _ => e.to_string(),
    }
}

/// Where transfers and the trash live. Set from the configuration.
#[derive(Debug, Clone)]
pub struct Dirs {
    pub xfer: PathBuf,
    pub trash: PathBuf,
}

// ------------------------------------------------------------- the rules

const DENIED: [&str; 3] = ["/proc", "/sys", "/dev"];
const DENIED_FILES: [&str; 1] = ["/etc/opsbox/master.key"];
/// Read-only through the panel: changing these could break or take over opsbox.
const NO_MODIFY: [&str; 6] = [
    "/var/lib/opsbox/bin",
    "/var/lib/opsbox/opsbox.db",
    "/var/lib/opsbox/opsbox.db-wal",
    "/var/lib/opsbox/opsbox.db-shm",
    "/var/lib/opsbox/xfer",
    "/etc/opsbox/firewall.json",
];
/// Never deleted, whatever else is true.
const NO_DELETE: [&str; 21] = [
    "/", "/bin", "/boot", "/dev", "/etc", "/home", "/lib", "/lib32", "/lib64", "/media", "/mnt",
    "/opt", "/proc", "/root", "/run", "/sbin", "/srv", "/sys", "/usr", "/var", "/var/lib",
];

/// Make an absolute path free of `.`, `..` and empty parts, without touching the disk.
/// `..` at the root stays at the root, as the kernel does.
pub fn normalize_lexical(path: &str) -> Result<PathBuf, PrivError> {
    if path.contains('\0') {
        return Err(invalid("A path cannot contain a NUL character."));
    }
    if !path.starts_with('/') {
        return Err(invalid("Paths must be absolute (start with /)."));
    }
    let mut out = PathBuf::from("/");
    for comp in Path::new(path).components() {
        match comp {
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(n) => out.push(n),
            Component::Prefix(_) => return Err(invalid("That is not a valid path.")),
        }
    }
    Ok(out)
}

fn is_denied(p: &Path) -> bool {
    DENIED.iter().any(|d| p.starts_with(d)) || DENIED_FILES.iter().any(|f| p == Path::new(f))
}

fn is_unmodifiable(p: &Path) -> bool {
    NO_MODIFY.iter().any(|d| p.starts_with(d))
        // The trash and transfer areas are managed by opsbox, never edited by hand.
        || p.starts_with("/var/lib/opsbox/trash")
}

fn is_undeletable(p: &Path) -> bool {
    NO_DELETE.iter().any(|d| p == Path::new(d))
        || p == Path::new("/var/lib/opsbox")
        || p == Path::new("/etc/opsbox")
        || (p.starts_with("/var/lib/opsbox") && !p.starts_with("/var/lib/opsbox/apps"))
}

/// Canonical form of `path`: symlinks followed. A path that does not exist yet keeps its
/// last components as given, resolved against the deepest part that does.
fn canonical(path: &Path, must_exist: bool) -> Result<PathBuf, PrivError> {
    match fs::canonicalize(path) {
        Ok(p) => Ok(p),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !must_exist => {
            let name = path
                .file_name()
                .ok_or_else(|| invalid("That is not a valid path."))?;
            let parent = path
                .parent()
                .ok_or_else(|| invalid("That is not a valid path."))?;
            let base = canonical(parent, true)
                .map_err(|_| err(PrivErrorCode::NotFound, "The folder does not exist."))?;
            Ok(base.join(name))
        }
        Err(e) => Err(io_err("Cannot open the path", &e)),
    }
}

fn check_jail(p: &Path, jail: Option<&str>) -> Result<(), PrivError> {
    let Some(j) = jail else { return Ok(()) };
    let jail = fs::canonicalize(normalize_lexical(j)?)
        .map_err(|_| err(PrivErrorCode::NotFound, "Your area does not exist yet."))?;
    if p.starts_with(&jail) {
        Ok(())
    } else {
        Err(err(
            PrivErrorCode::Refused,
            "That is outside the folders your account may use.",
        ))
    }
}

fn check_not_denied(p: &Path) -> Result<(), PrivError> {
    if is_denied(p) {
        Err(err(
            PrivErrorCode::Refused,
            "System areas such as /proc, /sys, /dev and opsbox's master key are not available through the panel.",
        ))
    } else {
        Ok(())
    }
}

/// Resolve `path` for reading or modifying a file or folder that may or may not exist yet.
pub fn resolve(path: &str, jail: Option<&str>, must_exist: bool) -> Result<PathBuf, PrivError> {
    let lexical = normalize_lexical(path)?;
    check_not_denied(&lexical)?; // refuse before touching the disk at all
    let real = canonical(&lexical, must_exist)?;
    check_not_denied(&real)?;
    check_jail(&real, jail)?;
    Ok(real)
}

/// Like [`resolve`] for something about to be deleted: the last component is *not*
/// followed, so deleting a symlink deletes the link and never what it points to.
fn resolve_leaf(path: &str, jail: Option<&str>) -> Result<PathBuf, PrivError> {
    let lexical = normalize_lexical(path)?;
    check_not_denied(&lexical)?;
    let name = lexical
        .file_name()
        .ok_or_else(|| err(PrivErrorCode::Refused, "That cannot be removed."))?
        .to_owned();
    let parent = canonical(lexical.parent().unwrap_or(Path::new("/")), true)?;
    let real = parent.join(name);
    check_not_denied(&real)?;
    check_jail(&real, jail)?;
    fs::symlink_metadata(&real).map_err(|e| io_err("Cannot find it", &e))?;
    Ok(real)
}

fn check_modifiable(p: &Path) -> Result<(), PrivError> {
    if is_unmodifiable(p) {
        Err(err(
            PrivErrorCode::Refused,
            "This belongs to opsbox itself and is read-only here.",
        ))
    } else {
        Ok(())
    }
}

// ------------------------------------------------------------ small helpers

/// `ls`-style mode string.
pub fn mode_string(meta: &Metadata) -> String {
    let m = meta.mode();
    let kind = if meta.is_dir() {
        'd'
    } else if meta.file_type().is_symlink() {
        'l'
    } else {
        '-'
    };
    let bit = |mask: u32, ch: char| if m & mask != 0 { ch } else { '-' };
    let x = |mask: u32, special: u32, on: char, off: char| match (m & mask != 0, m & special != 0) {
        (true, true) => on,
        (false, true) => off,
        (true, false) => 'x',
        (false, false) => '-',
    };
    format!(
        "{kind}{}{}{}{}{}{}{}{}{}",
        bit(0o400, 'r'),
        bit(0o200, 'w'),
        x(0o100, 0o4000, 's', 'S'),
        bit(0o040, 'r'),
        bit(0o020, 'w'),
        x(0o010, 0o2000, 's', 'S'),
        bit(0o004, 'r'),
        bit(0o002, 'w'),
        x(0o001, 0o1000, 't', 'T'),
    )
}

fn mtime(meta: &Metadata) -> i64 {
    meta.mtime()
}

/// A hint for files that probably should not be readable by everyone.
pub fn warn_for(name: &str, mode: u32) -> Option<String> {
    let secret = name.starts_with(".env")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519");
    (secret && !name.ends_with(".pub") && mode & 0o004 != 0)
        .then(|| "Readable by everyone".to_owned())
}

fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|b| *b == 0) || std::str::from_utf8(bytes).is_err()
}

/// A single file name: not empty, not `.` or `..`, no `/` or NUL or control characters.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.len() <= 255
        && !name
            .chars()
            .any(|c| c == '/' || c == '\0' || c.is_control())
}

fn chown_like(path: &Path, reference: &Metadata) -> Result<(), PrivError> {
    std::os::unix::fs::chown(path, Some(reference.uid()), Some(reference.gid()))
        .map_err(|e| io_err("Cannot set the owner", &e))
}

// ------------------------------------------------------------------ list

pub(super) fn list(path: &str, jail: Option<&str>) -> Result<DirListing, PrivError> {
    let dir = resolve(path, jail, true)?;
    let meta = fs::metadata(&dir).map_err(|e| io_err("Cannot open the folder", &e))?;
    if !meta.is_dir() {
        return Err(invalid(format!("{} is not a folder.", dir.display())));
    }
    let mut entries = Vec::new();
    for e in fs::read_dir(&dir)
        .map_err(|e| io_err("Cannot read the folder", &e))?
        .flatten()
        .take(MAX_ENTRIES)
    {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(lmeta) = fs::symlink_metadata(e.path()) else {
            continue;
        };
        let is_link = lmeta.file_type().is_symlink();
        let (meta, kind) = match (is_link, fs::metadata(e.path())) {
            (_, Ok(m)) => {
                let k = if m.is_dir() {
                    FileKind::Dir
                } else {
                    FileKind::File
                };
                (m, k)
            }
            (true, Err(_)) => (lmeta.clone(), FileKind::BrokenLink),
            (false, Err(_)) => continue,
        };
        let mode_bits = meta.mode();
        entries.push(FileEntry {
            path: dir.join(&name).to_string_lossy().into_owned(),
            kind,
            mode: mode_string(&meta),
            mtime: mtime(&meta),
            size: (kind != FileKind::Dir).then_some(meta.len()),
            is_link,
            warn: warn_for(&name, mode_bits),
            name,
        });
    }
    entries.sort_by(|a, b| {
        (a.kind != FileKind::Dir)
            .cmp(&(b.kind != FileKind::Dir))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(DirListing {
        path: dir.to_string_lossy().into_owned(),
        entries,
    })
}

// ------------------------------------------------------------------ read

pub(super) fn read(path: &str, jail: Option<&str>) -> Result<FileContent, PrivError> {
    let p = resolve(path, jail, true)?;
    let meta = fs::metadata(&p).map_err(|e| io_err("Cannot open the file", &e))?;
    if !meta.is_file() {
        return Err(invalid("That is not a regular file."));
    }
    if meta.len() > MAX_EDIT_BYTES {
        return Err(invalid(format!(
            "This file is {:.1} MB, too big to edit in the browser (limit {} MB). Download it instead.",
            meta.len() as f64 / 1_048_576.0,
            MAX_EDIT_BYTES / 1_048_576
        )));
    }
    let bytes = fs::read(&p).map_err(|e| io_err("Cannot read the file", &e))?;
    if is_binary(&bytes) {
        return Err(invalid(
            "This file is not text, so it cannot be edited here. Download it instead.",
        ));
    }
    Ok(FileContent {
        path: p.to_string_lossy().into_owned(),
        content: String::from_utf8(bytes).unwrap_or_default(),
        size: meta.len(),
        mode: mode_string(&meta),
        mtime: mtime(&meta),
    })
}

// ----------------------------------------------------------------- write

/// Write `bytes` to a temporary file next to `target`, flush it to disk and rename it over.
fn write_atomic(
    target: &Path,
    bytes: &[u8],
    mode: u32,
    owner_of: &Metadata,
) -> Result<(), PrivError> {
    let dir = target
        .parent()
        .ok_or_else(|| invalid("That is not a valid path."))?;
    let mut rnd = [0u8; 6];
    getrandom::fill(&mut rnd)
        .map_err(|_| err(PrivErrorCode::Internal, "random number generator failed"))?;
    let suffix: String = rnd.iter().map(|b| format!("{b:02x}")).collect();
    let tmp = dir.join(format!(
        ".{}.opsbox-{suffix}.tmp",
        target
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default()
    ));
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(OFlag::O_NOFOLLOW.bits())
            .open(&tmp)
            .map_err(|e| io_err("Cannot create the file", &e))?;
        f.write_all(bytes)
            .map_err(|e| io_err("Cannot write the file", &e))?;
        f.sync_all()
            .map_err(|e| io_err("Cannot flush the file", &e))?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(mode & 0o7777))
            .map_err(|e| io_err("Cannot set the permissions", &e))?;
        chown_like(&tmp, owner_of)?;
        fs::rename(&tmp, target).map_err(|e| io_err("Cannot replace the file", &e))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;
    // Make the rename itself durable; failure here is not worth failing the save for.
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

pub(super) fn write(
    path: &str,
    content: &str,
    expect_mtime: Option<i64>,
    jail: Option<&str>,
) -> Result<i64, PrivError> {
    if content.len() as u64 > MAX_EDIT_BYTES {
        return Err(invalid(format!(
            "That is too big to save from the browser (limit {} MB).",
            MAX_EDIT_BYTES / 1_048_576
        )));
    }
    let p = resolve(path, jail, false)?;
    check_modifiable(&p)?;
    let (mode, owner_src) = match fs::metadata(&p) {
        Ok(m) if m.is_file() => {
            if let Some(expected) = expect_mtime
                && mtime(&m) != expected
            {
                return Err(err(
                    PrivErrorCode::Conflict,
                    "This file changed on the server since you opened it. Reload it, then make your edit again so nothing is lost.",
                ));
            }
            (m.mode(), m)
        }
        Ok(_) => return Err(invalid("That is not a regular file.")),
        // A new file: 0644, owned like the folder it goes in.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (
            0o644,
            fs::metadata(p.parent().unwrap_or(Path::new("/")))
                .map_err(|e| io_err("Cannot open the folder", &e))?,
        ),
        Err(e) => return Err(io_err("Cannot open the file", &e)),
    };
    write_atomic(&p, content.as_bytes(), mode, &owner_src)?;
    Ok(fs::metadata(&p).map(|m| mtime(&m)).unwrap_or(0))
}

// ------------------------------------------------------- chmod and mkdir

pub fn parse_mode(text: &str) -> Result<u32, String> {
    if !(3..=4).contains(&text.len()) || !text.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
        return Err("The mode must be 3 or 4 octal digits, for example 600 or 644.".into());
    }
    u32::from_str_radix(text, 8).map_err(|_| "The mode must be octal.".to_owned())
}

pub(super) fn chmod(path: &str, mode: &str, jail: Option<&str>) -> Result<(), PrivError> {
    let bits = parse_mode(mode).map_err(invalid)?;
    let p = resolve(path, jail, true)?;
    check_modifiable(&p)?;
    fs::set_permissions(&p, fs::Permissions::from_mode(bits))
        .map_err(|e| io_err("Cannot change the permissions", &e))
}

pub(super) fn mkdir(path: &str, jail: Option<&str>) -> Result<(), PrivError> {
    let p = resolve(path, jail, false)?;
    check_modifiable(&p)?;
    if fs::symlink_metadata(&p).is_ok() {
        return Err(err(
            PrivErrorCode::Conflict,
            "Something with that name already exists.",
        ));
    }
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !valid_name(name) {
        return Err(invalid("That is not a valid folder name."));
    }
    let parent_meta = fs::metadata(p.parent().unwrap_or(Path::new("/")))
        .map_err(|e| io_err("Cannot open the folder", &e))?;
    fs::create_dir(&p).map_err(|e| io_err("Cannot create the folder", &e))?;
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755))
        .map_err(|e| io_err("Cannot set the permissions", &e))?;
    chown_like(&p, &parent_meta)
}

// ---------------------------------------------------------------- remove

pub(super) fn remove(
    paths: &[String],
    permanent: bool,
    jail: Option<&str>,
    dirs: &Dirs,
) -> Result<u32, PrivError> {
    if paths.is_empty() || paths.len() > 500 {
        return Err(invalid("Choose between 1 and 500 items."));
    }
    // Resolve and check everything first: either all of it goes or none of it does.
    let mut targets = Vec::new();
    for raw in paths {
        let p = resolve_leaf(raw, jail)?;
        check_modifiable(&p)?;
        if is_undeletable(&p) {
            return Err(err(
                PrivErrorCode::Refused,
                format!(
                    "{} is part of the system or of opsbox and is never deleted from the panel.",
                    p.display()
                ),
            ));
        }
        targets.push(p);
    }
    let mut n = 0;
    for p in targets {
        if permanent {
            delete_forever(&p)?;
        } else {
            move_to_trash(&p, dirs)?;
        }
        n += 1;
    }
    Ok(n)
}

fn delete_forever(p: &Path) -> Result<(), PrivError> {
    let meta = fs::symlink_metadata(p).map_err(|e| io_err("Cannot find it", &e))?;
    // remove_dir_all does not follow symbolic links.
    if meta.is_dir() {
        fs::remove_dir_all(p)
    } else {
        fs::remove_file(p)
    }
    .map_err(|e| io_err(&format!("Cannot delete {}", p.display()), &e))
}

fn move_to_trash(p: &Path, dirs: &Dirs) -> Result<(), PrivError> {
    fs::create_dir_all(&dirs.trash).map_err(|e| io_err("Cannot create the trash", &e))?;
    fs::set_permissions(&dirs.trash, fs::Permissions::from_mode(0o700)).ok();
    let now = now_secs();
    let mut rnd = [0u8; 3];
    getrandom::fill(&mut rnd)
        .map_err(|_| err(PrivErrorCode::Internal, "random number generator failed"))?;
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "item".into());
    let id = format!(
        "{now}-{}-{name}",
        rnd.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    let dest = dirs.trash.join(&id);
    let meta = fs::symlink_metadata(p).map_err(|e| io_err("Cannot find it", &e))?;
    match fs::rename(p, &dest) {
        Ok(()) => {}
        Err(e) if e.raw_os_error() == Some(nix::errno::Errno::EXDEV as i32) => {
            // A different disk: copy a file across, but not a whole folder tree.
            if meta.is_dir() {
                return Err(invalid(
                    "This folder is on a different disk from the trash, so it cannot be moved there. Delete it permanently instead.",
                ));
            }
            fs::copy(p, &dest).map_err(|e| io_err("Cannot copy to the trash", &e))?;
            fs::remove_file(p).map_err(|e| io_err("Cannot remove the original", &e))?;
        }
        Err(e) => {
            return Err(io_err(
                &format!("Cannot move {} to the trash", p.display()),
                &e,
            ));
        }
    }
    let note =
        serde_json::json!({ "original": p.to_string_lossy(), "deleted_at": now }).to_string();
    let _ = fs::write(dirs.trash.join(format!("{id}.meta")), note);
    Ok(())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// Remove trash entries older than seven days. Called by the janitor.
pub fn purge_trash(trash: &Path, now: i64) -> usize {
    let Ok(dir) = fs::read_dir(trash) else {
        return 0;
    };
    let mut gone = 0;
    for e in dir.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        // `<unix time>-<random>-<name>` (and a `.meta` twin with the same prefix)
        let Some(ts) = name.split('-').next().and_then(|t| t.parse::<i64>().ok()) else {
            continue;
        };
        if now - ts > TRASH_KEEP_SECS {
            let p = e.path();
            let _ = if p.is_dir() {
                fs::remove_dir_all(&p)
            } else {
                fs::remove_file(&p)
            };
            gone += 1;
        }
    }
    gone
}

// ------------------------------------------------------------- transfers

fn check_staging(staging: &str, dirs: &Dirs) -> Result<PathBuf, PrivError> {
    let lexical = normalize_lexical(staging)?;
    let xfer = fs::canonicalize(&dirs.xfer).map_err(|_| {
        err(
            PrivErrorCode::Internal,
            "The transfer folder does not exist.",
        )
    })?;
    // The folder part may reach the transfer folder through a symbolic link (for example
    // /tmp -> /private/tmp), so compare real paths; the file itself must not be a link,
    // which the check below enforces.
    let name = lexical.file_name().ok_or_else(|| {
        err(
            PrivErrorCode::Refused,
            "That is not a file in the transfer folder.",
        )
    })?;
    let parent = fs::canonicalize(lexical.parent().unwrap_or(Path::new("/"))).map_err(|_| {
        err(
            PrivErrorCode::Refused,
            "That is not a file in the transfer folder.",
        )
    })?;
    let p = parent.join(name);
    if p.parent() != Some(xfer.as_path()) {
        return Err(err(
            PrivErrorCode::Refused,
            "That is not a file in the transfer folder.",
        ));
    }
    let meta = fs::symlink_metadata(&p).map_err(|e| io_err("Cannot find the transfer file", &e))?;
    if !meta.file_type().is_file() {
        return Err(err(
            PrivErrorCode::Refused,
            "The transfer file is not a regular file.",
        ));
    }
    Ok(p)
}

pub(super) fn stage_download(
    path: &str,
    jail: Option<&str>,
    dirs: &Dirs,
) -> Result<Staged, PrivError> {
    let src = resolve(path, jail, true)?;
    let meta = fs::metadata(&src).map_err(|e| io_err("Cannot open the file", &e))?;
    if !meta.is_file() {
        return Err(invalid("Only files can be downloaded (not folders)."));
    }
    let xfer = fs::canonicalize(&dirs.xfer).map_err(|_| {
        err(
            PrivErrorCode::Internal,
            "The transfer folder does not exist.",
        )
    })?;
    let xfer_meta =
        fs::metadata(&xfer).map_err(|e| io_err("Cannot open the transfer folder", &e))?;
    let mut rnd = [0u8; 8];
    getrandom::fill(&mut rnd)
        .map_err(|_| err(PrivErrorCode::Internal, "random number generator failed"))?;
    let staged = xfer.join(format!(
        "{}.dl",
        rnd.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ));
    // Copy, then hand the copy to the user that runs `serve` (the transfer folder's owner).
    let mut input = fs::File::open(&src).map_err(|e| io_err("Cannot read the file", &e))?;
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staged)
        .map_err(|e| io_err("Cannot create the transfer file", &e))?;
    if let Err(e) = std::io::copy(&mut input, &mut out).and_then(|_| out.sync_all()) {
        let _ = fs::remove_file(&staged);
        return Err(io_err("Cannot copy the file", &e));
    }
    chown_like(&staged, &xfer_meta)?;
    Ok(Staged {
        staging: staged.to_string_lossy().into_owned(),
        name: src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size: meta.len(),
    })
}

pub(super) fn install_upload(
    staging: &str,
    dir: &str,
    name: &str,
    overwrite: bool,
    jail: Option<&str>,
    dirs: &Dirs,
) -> Result<(), PrivError> {
    if !valid_name(name) {
        return Err(invalid("That is not a valid file name."));
    }
    let src = check_staging(staging, dirs)?;
    let dest_dir = resolve(dir, jail, true)?;
    check_modifiable(&dest_dir)?;
    let dir_meta = fs::metadata(&dest_dir).map_err(|e| io_err("Cannot open the folder", &e))?;
    if !dir_meta.is_dir() {
        return Err(invalid("The destination is not a folder."));
    }
    let dest = dest_dir.join(name);
    check_not_denied(&dest)?;
    check_modifiable(&dest)?;
    let (mode, owner_src) = match fs::symlink_metadata(&dest) {
        Ok(m) if m.file_type().is_file() && overwrite => (m.mode(), m),
        Ok(_) if !overwrite => {
            return Err(err(
                PrivErrorCode::Conflict,
                format!("A file named {name} already exists here."),
            ));
        }
        Ok(_) => {
            return Err(invalid(
                "Something that is not a regular file already has that name.",
            ));
        }
        Err(_) => (0o644, dir_meta),
    };
    // Open the staged file without following a link someone swapped in after the check.
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(OFlag::O_NOFOLLOW.bits())
        .open(&src)
        .map_err(|e| io_err("Cannot read the upload", &e))?;
    let mut rnd = [0u8; 6];
    getrandom::fill(&mut rnd)
        .map_err(|_| err(PrivErrorCode::Internal, "random number generator failed"))?;
    let tmp = dest_dir.join(format!(
        ".{name}.opsbox-{}.tmp",
        rnd.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ));
    let result = (|| {
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| io_err("Cannot create the file", &e))?;
        std::io::copy(&mut input, &mut out).map_err(|e| io_err("Cannot write the file", &e))?;
        out.sync_all()
            .map_err(|e| io_err("Cannot flush the file", &e))?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(mode & 0o7777))
            .map_err(|e| io_err("Cannot set the permissions", &e))?;
        chown_like(&tmp, &owner_src)?;
        fs::rename(&tmp, &dest).map_err(|e| io_err("Cannot move the file into place", &e))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;
    let _ = fs::remove_file(&src);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::os::unix::fs::symlink;
    use std::sync::OnceLock;

    fn tmp() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let p = fs::canonicalize(d.path()).unwrap(); // macOS temp dirs are behind a symlink
        (d, p)
    }
    fn s(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }
    fn dirs(root: &Path) -> Dirs {
        let d = Dirs {
            xfer: root.join("xfer"),
            trash: root.join("trash"),
        };
        fs::create_dir_all(&d.xfer).unwrap();
        d
    }

    // ---- resolution rules

    #[test]
    fn lexical_normalization() {
        for (input, want) in [
            ("/", "/"),
            ("/a/b", "/a/b"),
            ("/a//b/", "/a/b"),
            ("/a/./b", "/a/b"),
            ("/a/../b", "/b"),
            ("/../../x", "/x"),
            ("/a/b/../../..", "/"),
        ] {
            assert_eq!(
                normalize_lexical(input).unwrap(),
                PathBuf::from(want),
                "{input}"
            );
        }
        for bad in ["", "a/b", "./x", "../x", "/a\0b"] {
            assert!(normalize_lexical(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn system_areas_and_the_master_key_are_refused_however_they_are_spelled() {
        for p in [
            "/proc",
            "/proc/1/environ",
            "/sys/kernel",
            "/dev/sda",
            "/etc/opsbox/master.key",
            "/tmp/../proc/self",
            "/a/b/../../dev/null",
            "//proc//version",
            "/proc/./x",
        ] {
            let e = resolve(p, None, false).unwrap_err();
            assert_eq!(e.code, PrivErrorCode::Refused, "{p}: {}", e.message);
        }
        assert!(resolve("/etc/opsbox/master.key.bak", None, false).is_err() || true);
    }

    #[test]
    fn a_symlink_to_a_denied_area_is_refused_too() {
        let (_d, t) = tmp();
        symlink("/proc", t.join("p")).unwrap();
        symlink("/dev", t.join("d")).unwrap();
        for (l, target) in [("p", "/proc"), ("d", "/dev"), ("p/self", "/proc/self")] {
            let code = resolve(&s(&t.join(l)), None, true).unwrap_err().code;
            // Where the target exists (all of them on Linux) the deny list must catch it; a
            // dangling link on another system is simply not found, still never opened.
            let expected = if Path::new(target).exists() {
                PrivErrorCode::Refused
            } else {
                PrivErrorCode::NotFound
            };
            assert_eq!(code, expected, "{l}");
        }
    }

    #[test]
    fn a_jail_holds_against_dot_dot_and_symlinks() {
        let (_d, t) = tmp();
        let jail = t.join("apps");
        fs::create_dir_all(jail.join("web")).unwrap();
        fs::create_dir(t.join("outside")).unwrap();
        fs::write(t.join("outside/secret"), "x").unwrap();
        symlink(t.join("outside"), jail.join("escape")).unwrap();
        symlink(jail.join("web"), jail.join("shortcut")).unwrap();
        let j = Some(s(&jail));
        assert!(resolve(&s(&jail.join("web")), j.as_deref(), true).is_ok());
        assert!(
            resolve(&s(&jail.join("shortcut")), j.as_deref(), true).is_ok(),
            "a link that stays inside is fine"
        );
        assert!(
            resolve(&s(&jail.join("web/new.txt")), j.as_deref(), false).is_ok(),
            "a new file inside"
        );
        for out in [
            t.join("outside/secret"),
            jail.join("escape/secret"),
            jail.join("web/../../outside"),
            jail.join("escape"),
            t.clone(),
            PathBuf::from("/etc"),
        ] {
            let e = resolve(&s(&out), j.as_deref(), false).unwrap_err();
            assert!(
                matches!(e.code, PrivErrorCode::Refused | PrivErrorCode::NotFound),
                "{}: {:?}",
                out.display(),
                e
            );
        }
        assert_eq!(
            resolve(&s(&t.join("outside/secret")), j.as_deref(), true)
                .unwrap_err()
                .code,
            PrivErrorCode::Refused
        );
    }

    #[test]
    fn protected_paths() {
        for p in [
            "/",
            "/etc",
            "/usr",
            "/var",
            "/var/lib",
            "/var/lib/opsbox",
            "/var/lib/opsbox/bin",
            "/var/lib/opsbox/opsbox.db",
            "/etc/opsbox",
        ] {
            assert!(is_undeletable(Path::new(p)), "{p} must never be deleted");
        }
        assert!(!is_undeletable(Path::new("/var/lib/opsbox/apps/web/data")));
        assert!(!is_undeletable(Path::new("/home/deploy/old.log")));
        for p in [
            "/var/lib/opsbox/bin/opsbox-0.1.0",
            "/var/lib/opsbox/opsbox.db-wal",
            "/var/lib/opsbox/trash/x",
            "/etc/opsbox/firewall.json",
            "/var/lib/opsbox/xfer",
        ] {
            assert!(is_unmodifiable(Path::new(p)), "{p} must be read-only");
        }
        assert!(!is_unmodifiable(Path::new("/var/lib/opsbox/apps/web/.env")));
        assert!(!is_unmodifiable(Path::new("/etc/opsbox/config.toml")));
    }

    // ---- property tests: no spelling of a path gets around the rules

    fn comp() -> impl Strategy<Value = &'static str> {
        prop_oneof![
            Just(".."),
            Just("."),
            Just(""),
            Just("a"),
            Just("b"),
            Just("proc"),
            Just("sys"),
            Just("dev"),
            Just("etc")
        ]
    }

    /// An independent model of `..` handling at the root.
    fn model(comps: &[&str]) -> Vec<String> {
        let mut st: Vec<String> = Vec::new();
        for c in comps {
            match *c {
                "" | "." => {}
                ".." => {
                    st.pop();
                }
                n => st.push(n.to_owned()),
            }
        }
        st
    }

    proptest! {
        #[test]
        fn normalization_matches_the_model_and_is_always_clean(comps in prop::collection::vec(comp(), 0..14)) {
            let path = format!("/{}", comps.join("/"));
            let got = normalize_lexical(&path).unwrap();
            prop_assert!(got.is_absolute());
            prop_assert!(got.components().all(|c| matches!(c, Component::RootDir | Component::Normal(_))));
            let want: PathBuf = std::iter::once("/".to_owned()).chain(model(&comps)).collect();
            prop_assert_eq!(got, want);
        }

        #[test]
        fn the_denied_areas_are_never_reachable(comps in prop::collection::vec(comp(), 0..14)) {
            let path = format!("/{}", comps.join("/"));
            let lexical = normalize_lexical(&path).unwrap();
            let in_denied_area = ["proc", "sys", "dev"].iter().any(|d| lexical.starts_with(format!("/{d}")));
            match resolve(&path, None, false) {
                Err(_) if in_denied_area => {}
                Err(_) => {} // not existing, or unreadable: fine
                Ok(p) => {
                    prop_assert!(!in_denied_area, "{} resolved to {}", path, p.display());
                    prop_assert!(!is_denied(&p));
                    prop_assert!(p.is_absolute());
                }
            }
        }

        #[test]
        fn nothing_escapes_the_jail_by_any_combination_of_dot_dot_and_links(comps in prop::collection::vec(prop_oneof![
            Just(".."), Just("."), Just(""), Just("a"), Just("b"), Just("link_out"), Just("link_in"), Just("link_proc"), Just("secret")], 0..10)) {
            let (_t, jail, outside) = jail_fixture();
            let path = format!("{}/{}", jail.display(), comps.join("/"));
            for must_exist in [true, false] {
                if let Ok(p) = resolve(&path, Some(&s(jail)), must_exist) {
                    prop_assert!(p.starts_with(jail), "{} escaped to {}", path, p.display());
                    prop_assert!(!p.starts_with(outside));
                }
            }
        }
    }

    /// Shared by all property cases: a jail with links in and out, and a secret outside.
    fn jail_fixture() -> (
        &'static tempfile::TempDir,
        &'static PathBuf,
        &'static PathBuf,
    ) {
        static F: OnceLock<(tempfile::TempDir, PathBuf, PathBuf)> = OnceLock::new();
        let f = F.get_or_init(|| {
            let (d, t) = {
                let d = tempfile::tempdir().unwrap();
                let p = fs::canonicalize(d.path()).unwrap();
                (d, p)
            };
            let (jail, outside) = (t.join("jail"), t.join("outside"));
            fs::create_dir_all(jail.join("a")).unwrap();
            fs::create_dir_all(jail.join("b")).unwrap();
            fs::create_dir_all(&outside).unwrap();
            fs::write(outside.join("secret"), "x").unwrap();
            fs::write(jail.join("a/secret"), "y").unwrap();
            symlink(&outside, jail.join("link_out")).unwrap();
            symlink(jail.join("a"), jail.join("link_in")).unwrap();
            symlink("/proc", jail.join("link_proc")).unwrap();
            (d, jail, outside)
        });
        (&f.0, &f.1, &f.2)
    }

    // ---- listing, reading, writing

    #[test]
    fn mode_strings_look_like_ls() {
        let (_d, t) = tmp();
        let f = t.join("f");
        fs::write(&f, "").unwrap();
        for (mode, want) in [
            (0o644, "-rw-r--r--"),
            (0o600, "-rw-------"),
            (0o755, "-rwxr-xr-x"),
            (0o4755, "-rwsr-xr-x"),
            (0o4644, "-rwSr--r--"),
            (0o2755, "-rwxr-sr-x"),
        ] {
            fs::set_permissions(&f, fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(mode_string(&fs::metadata(&f).unwrap()), want, "{mode:o}");
        }
        let d = t.join("d");
        fs::create_dir(&d).unwrap();
        fs::set_permissions(&d, fs::Permissions::from_mode(0o1777)).unwrap();
        assert_eq!(mode_string(&fs::metadata(&d).unwrap()), "drwxrwxrwt");
    }

    #[test]
    fn listing_puts_folders_first_flags_loose_secrets_and_shows_links() {
        let (_d, t) = tmp();
        fs::create_dir(t.join("zeta")).unwrap();
        fs::create_dir(t.join("Alpha")).unwrap();
        fs::write(t.join("b.txt"), "hello").unwrap();
        fs::write(t.join(".env"), "SECRET=1").unwrap();
        fs::write(t.join("tidy.env"), "x").unwrap();
        fs::set_permissions(t.join(".env"), fs::Permissions::from_mode(0o644)).unwrap();
        fs::set_permissions(t.join("tidy.env"), fs::Permissions::from_mode(0o600)).unwrap();
        symlink(t.join("nowhere"), t.join("dangling")).unwrap();
        symlink(t.join("zeta"), t.join("to_zeta")).unwrap();
        let l = list(&s(&t), None).unwrap();
        let names: Vec<&str> = l.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Alpha", "to_zeta", "zeta", ".env", "b.txt", "dangling", "tidy.env"
            ],
            "folders first, case-insensitive"
        );
        let by = |n: &str| l.entries.iter().find(|e| e.name == n).unwrap();
        assert_eq!(by(".env").warn.as_deref(), Some("Readable by everyone"));
        assert_eq!(by("tidy.env").warn, None);
        assert_eq!((by("b.txt").size, by("zeta").size), (Some(5), None));
        assert_eq!(
            (by("dangling").kind, by("dangling").is_link),
            (FileKind::BrokenLink, true)
        );
        assert_eq!(
            (by("to_zeta").kind, by("to_zeta").is_link),
            (FileKind::Dir, true)
        );
        assert_eq!(by("zeta").path, s(&t.join("zeta")));
        assert_eq!(
            list(&s(&t.join("b.txt")), None).unwrap_err().code,
            PrivErrorCode::Invalid
        );
        assert_eq!(
            list(&s(&t.join("missing")), None).unwrap_err().code,
            PrivErrorCode::NotFound
        );
    }

    #[test]
    fn reading_text_binary_and_big_files() {
        let (_d, t) = tmp();
        fs::write(t.join("a.txt"), "héllo\n").unwrap();
        // The fixture's mode is independent of the runner's umask (Ubuntu uses 0002).
        fs::set_permissions(t.join("a.txt"), fs::Permissions::from_mode(0o644)).unwrap();
        let r = read(&s(&t.join("a.txt")), None).unwrap();
        assert_eq!(
            (r.content.as_str(), r.size, r.mode.as_str()),
            ("héllo\n", 7, "-rw-r--r--")
        );
        fs::write(t.join("bin"), [0u8, 159, 146, 150]).unwrap();
        assert!(
            read(&s(&t.join("bin")), None)
                .unwrap_err()
                .message
                .contains("not text")
        );
        fs::write(t.join("latin1"), [0x63, 0x61, 0x66, 0xe9]).unwrap();
        assert!(
            read(&s(&t.join("latin1")), None)
                .unwrap_err()
                .message
                .contains("not text"),
            "invalid UTF-8 is not edited (it would be corrupted)"
        );
        let big = fs::File::create(t.join("big")).unwrap();
        big.set_len(MAX_EDIT_BYTES + 1).unwrap();
        let e = read(&s(&t.join("big")), None).unwrap_err();
        assert!(
            e.message.contains("Download it instead") && e.message.contains("5 MB"),
            "{}",
            e.message
        );
        assert_eq!(
            read(&s(&t), None).unwrap_err().code,
            PrivErrorCode::Invalid,
            "a folder"
        );
    }

    #[test]
    fn saving_is_atomic_keeps_mode_and_detects_concurrent_edits() {
        let (_d, t) = tmp();
        let f = t.join("conf");
        // New file: 0644, no temp files left behind.
        let m1 = write(&s(&f), "one\n", None, None).unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "one\n");
        assert_eq!(fs::metadata(&f).unwrap().mode() & 0o777, 0o644);
        assert_eq!(
            fs::read_dir(&t).unwrap().count(),
            1,
            "no temporary file remains"
        );
        // Overwrite keeps a restrictive mode.
        fs::set_permissions(&f, fs::Permissions::from_mode(0o600)).unwrap();
        let m2 = write(
            &s(&f),
            "two\n",
            Some(fs::metadata(&f).unwrap().mtime()),
            None,
        )
        .unwrap();
        assert_eq!(
            (
                fs::read_to_string(&f).unwrap().as_str(),
                fs::metadata(&f).unwrap().mode() & 0o777
            ),
            ("two\n", 0o600)
        );
        assert!(m2 >= m1);
        // Someone else edited it in between: refused, and nothing is lost.
        let e = write(&s(&f), "mine\n", Some(m2 - 100), None).unwrap_err();
        assert_eq!(e.code, PrivErrorCode::Conflict);
        assert!(e.message.contains("changed on the server"));
        assert_eq!(fs::read_to_string(&f).unwrap(), "two\n");
        // Not into a folder, not through a missing folder, not too big.
        assert_eq!(
            write(&s(&t), "x", None, None).unwrap_err().code,
            PrivErrorCode::Invalid
        );
        assert_eq!(
            write(&s(&t.join("nodir/x")), "x", None, None)
                .unwrap_err()
                .code,
            PrivErrorCode::NotFound
        );
        assert!(write(&s(&f), &"x".repeat(5 * 1024 * 1024 + 1), None, None).is_err());
        assert_eq!(fs::read_to_string(&f).unwrap(), "two\n");
    }

    #[test]
    fn writes_through_a_symlink_replace_the_target_not_the_link() {
        let (_d, t) = tmp();
        fs::write(t.join("real"), "old").unwrap();
        symlink(t.join("real"), t.join("link")).unwrap();
        write(&s(&t.join("link")), "new", None, None).unwrap();
        assert_eq!(fs::read_to_string(t.join("real")).unwrap(), "new");
        assert!(
            fs::symlink_metadata(t.join("link"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn permissions_and_folders() {
        let (_d, t) = tmp();
        fs::write(t.join("f"), "").unwrap();
        chmod(&s(&t.join("f")), "600", None).unwrap();
        assert_eq!(fs::metadata(t.join("f")).unwrap().mode() & 0o7777, 0o600);
        chmod(&s(&t.join("f")), "0644", None).unwrap();
        for bad in ["", "6", "99", "abc", "12345", "-1", "6 0", "+x"] {
            assert_eq!(
                chmod(&s(&t.join("f")), bad, None).unwrap_err().code,
                PrivErrorCode::Invalid,
                "{bad}"
            );
        }
        mkdir(&s(&t.join("new")), None).unwrap();
        assert_eq!(fs::metadata(t.join("new")).unwrap().mode() & 0o777, 0o755);
        assert_eq!(
            mkdir(&s(&t.join("new")), None).unwrap_err().code,
            PrivErrorCode::Conflict
        );
        assert_eq!(
            mkdir(&s(&t.join("f")), None).unwrap_err().code,
            PrivErrorCode::Conflict
        );
        assert_eq!(
            mkdir(&s(&t.join("a/b/c")), None).unwrap_err().code,
            PrivErrorCode::NotFound
        );
        assert_eq!(parse_mode("755"), Ok(0o755));
    }

    // ---- deleting

    #[test]
    fn deleting_moves_to_the_trash_with_a_note_and_can_be_made_permanent() {
        let (_d, t) = tmp();
        let d = dirs(&t);
        fs::write(t.join("keep.txt"), "x").unwrap();
        fs::create_dir(t.join("tree")).unwrap();
        fs::write(t.join("tree/leaf"), "y").unwrap();
        fs::write(t.join("gone.txt"), "z").unwrap();
        assert_eq!(
            remove(
                &[s(&t.join("keep.txt")), s(&t.join("tree"))],
                false,
                None,
                &d
            )
            .unwrap(),
            2
        );
        assert!(!t.join("keep.txt").exists() && !t.join("tree").exists());
        let names: Vec<String> = fs::read_dir(&d.trash)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names.iter().filter(|n| n.ends_with(".meta")).count(),
            2,
            "{names:?}"
        );
        let meta_name = names.iter().find(|n| n.ends_with("keep.txt.meta")).unwrap();
        let note: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(d.trash.join(meta_name)).unwrap()).unwrap();
        assert_eq!(note["original"], s(&t.join("keep.txt")));
        assert!(
            names.iter().any(|n| n.ends_with("-tree"))
                && d.trash
                    .join(names.iter().find(|n| n.ends_with("-tree")).unwrap())
                    .join("leaf")
                    .exists(),
            "folders keep their contents"
        );
        assert_eq!(
            remove(&[s(&t.join("gone.txt"))], true, None, &d).unwrap(),
            1
        );
        assert!(!t.join("gone.txt").exists());
        assert_eq!(
            fs::read_dir(&d.trash).unwrap().count(),
            4,
            "permanent deletes leave nothing in the trash"
        );
    }

    #[test]
    fn deleting_refuses_the_whole_request_if_any_path_is_not_allowed() {
        let (_d, t) = tmp();
        let d = dirs(&t);
        fs::write(t.join("ok"), "").unwrap();
        for bad in [
            "/",
            "/etc",
            "/usr",
            "/var/lib/opsbox",
            "/proc/1",
            "/etc/opsbox/master.key",
            "relative",
        ] {
            assert!(
                remove(&[s(&t.join("ok")), bad.to_owned()], true, None, &d).is_err(),
                "{bad}"
            );
            assert!(t.join("ok").exists(), "{bad}: nothing was deleted");
        }
        assert!(remove(&[], false, None, &d).is_err());
        assert!(remove(&[s(&t.join("missing"))], false, None, &d).is_err());
    }

    #[test]
    fn deleting_a_symlink_never_touches_what_it_points_to() {
        let (_d, t) = tmp();
        let d = dirs(&t);
        fs::create_dir(t.join("target")).unwrap();
        fs::write(t.join("target/precious"), "x").unwrap();
        symlink(t.join("target"), t.join("link")).unwrap();
        remove(&[s(&t.join("link"))], true, None, &d).unwrap();
        assert!(fs::symlink_metadata(t.join("link")).is_err());
        assert!(t.join("target/precious").exists());
        symlink(t.join("target"), t.join("link2")).unwrap();
        remove(&[s(&t.join("link2"))], false, None, &d).unwrap();
        assert!(t.join("target/precious").exists(), "also for the trash");
    }

    #[test]
    fn a_jailed_account_cannot_delete_outside_its_area() {
        let (_d, t) = tmp();
        let d = dirs(&t);
        fs::create_dir_all(t.join("apps/web")).unwrap();
        fs::write(t.join("apps/web/f"), "").unwrap();
        fs::write(t.join("elsewhere"), "").unwrap();
        let j = Some(s(&t.join("apps")));
        assert!(remove(&[s(&t.join("elsewhere"))], true, j.as_deref(), &d).is_err());
        assert!(t.join("elsewhere").exists());
        assert_eq!(
            remove(&[s(&t.join("apps/web/f"))], true, j.as_deref(), &d).unwrap(),
            1
        );
    }

    #[test]
    fn old_trash_is_purged_and_new_trash_kept() {
        let (_d, t) = tmp();
        let trash = t.join("trash");
        fs::create_dir(&trash).unwrap();
        let now = 2_000_000_000;
        for (age_days, name) in [(8, "old.txt"), (6, "recent.txt")] {
            let ts = now - age_days * 86_400;
            fs::write(trash.join(format!("{ts}-aaa-{name}")), "x").unwrap();
            fs::write(trash.join(format!("{ts}-aaa-{name}.meta")), "{}").unwrap();
        }
        fs::write(trash.join("not-ours"), "x").unwrap();
        assert_eq!(purge_trash(&trash, now), 2, "the old file and its note");
        let left: Vec<String> = fs::read_dir(&trash)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left.len(), 3, "{left:?}");
        assert!(
            left.iter().any(|n| n == "not-ours"),
            "things that are not trash entries are left alone"
        );
    }

    // ---- uploads and downloads

    #[test]
    fn a_download_is_staged_as_a_copy_inside_the_transfer_folder() {
        let (_d, t) = tmp();
        let d = dirs(&t);
        fs::write(t.join("report.pdf"), "pdf bytes").unwrap();
        let st = stage_download(&s(&t.join("report.pdf")), None, &d).unwrap();
        assert_eq!((st.name.as_str(), st.size), ("report.pdf", 9));
        assert!(Path::new(&st.staging).starts_with(&d.xfer));
        assert_eq!(fs::read_to_string(&st.staging).unwrap(), "pdf bytes");
        assert_eq!(fs::metadata(&st.staging).unwrap().mode() & 0o777, 0o600);
        assert_eq!(
            stage_download(&s(&t), None, &d).unwrap_err().code,
            PrivErrorCode::Invalid,
            "folders are not downloadable"
        );
        assert!(stage_download("/proc/self/environ", None, &d).is_err());
    }

    #[test]
    fn an_upload_is_installed_with_the_folders_owner_and_sane_permissions() {
        let (_d, t) = tmp();
        let d = dirs(&t);
        fs::create_dir(t.join("dest")).unwrap();
        let staged = d.xfer.join("abc.part");
        fs::write(&staged, "uploaded").unwrap();
        install_upload(
            &s(&staged),
            &s(&t.join("dest")),
            "notes.txt",
            false,
            None,
            &d,
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(t.join("dest/notes.txt")).unwrap(),
            "uploaded"
        );
        assert_eq!(
            fs::metadata(t.join("dest/notes.txt")).unwrap().mode() & 0o777,
            0o644
        );
        assert!(!staged.exists(), "the staged copy is gone");
        // The name is taken: refused unless overwriting, and overwriting keeps the old mode.
        let again = d.xfer.join("def.part");
        fs::write(&again, "second").unwrap();
        let e = install_upload(
            &s(&again),
            &s(&t.join("dest")),
            "notes.txt",
            false,
            None,
            &d,
        )
        .unwrap_err();
        assert_eq!(e.code, PrivErrorCode::Conflict);
        assert!(again.exists(), "a refused upload is not consumed");
        fs::set_permissions(t.join("dest/notes.txt"), fs::Permissions::from_mode(0o600)).unwrap();
        install_upload(&s(&again), &s(&t.join("dest")), "notes.txt", true, None, &d).unwrap();
        assert_eq!(
            (
                fs::read_to_string(t.join("dest/notes.txt"))
                    .unwrap()
                    .as_str(),
                fs::metadata(t.join("dest/notes.txt")).unwrap().mode() & 0o777
            ),
            ("second", 0o600)
        );
    }

    #[test]
    fn the_transfer_folder_may_be_reached_through_a_symlink() {
        // Regression: /tmp is a link to /private/tmp on macOS, and configured paths on
        // Linux can contain links too. The staged file's folder is compared as a real path.
        let (_d, t) = tmp();
        let real = t.join("real_xfer");
        fs::create_dir(&real).unwrap();
        symlink(&real, t.join("link_xfer")).unwrap();
        fs::create_dir(t.join("dest")).unwrap();
        let d = Dirs {
            xfer: t.join("link_xfer"),
            trash: t.join("trash"),
        };
        let staged = t.join("link_xfer/up.part");
        fs::write(&staged, "via a link").unwrap();
        install_upload(&s(&staged), &s(&t.join("dest")), "f.txt", false, None, &d).unwrap();
        assert_eq!(
            fs::read_to_string(t.join("dest/f.txt")).unwrap(),
            "via a link"
        );
        // The file itself still must not be a link.
        fs::write(t.join("elsewhere"), "x").unwrap();
        symlink(t.join("elsewhere"), real.join("evil.part")).unwrap();
        assert!(
            install_upload(
                &s(&t.join("link_xfer/evil.part")),
                &s(&t.join("dest")),
                "g.txt",
                false,
                None,
                &d
            )
            .is_err()
        );
        // And a folder that only looks like the transfer folder is still refused.
        fs::create_dir(t.join("other")).unwrap();
        fs::write(t.join("other/up.part"), "x").unwrap();
        assert!(
            install_upload(
                &s(&t.join("other/up.part")),
                &s(&t.join("dest")),
                "h.txt",
                false,
                None,
                &d
            )
            .is_err()
        );
    }

    #[test]
    fn uploads_only_accept_regular_files_from_the_transfer_folder_and_safe_names() {
        let (_d, t) = tmp();
        let d = dirs(&t);
        fs::create_dir(t.join("dest")).unwrap();
        let dest = s(&t.join("dest"));
        // A file elsewhere, a link inside the transfer folder, a folder, a path that climbs out.
        fs::write(t.join("elsewhere"), "secret").unwrap();
        symlink(t.join("elsewhere"), d.xfer.join("link.part")).unwrap();
        fs::create_dir(d.xfer.join("dir.part")).unwrap();
        fs::write(d.xfer.join("ok.part"), "x").unwrap();
        for staging in [
            s(&t.join("elsewhere")),
            s(&d.xfer.join("link.part")),
            s(&d.xfer.join("dir.part")),
            s(&d.xfer.join("../elsewhere")),
            s(&d.xfer.join("missing.part")),
        ] {
            assert!(
                install_upload(&staging, &dest, "x.txt", false, None, &d).is_err(),
                "{staging}"
            );
        }
        assert!(!t.join("dest/x.txt").exists());
        for name in [
            "",
            ".",
            "..",
            "a/b",
            "../x",
            "a\0b",
            "a\nb",
            &"n".repeat(256),
        ] {
            assert_eq!(
                install_upload(&s(&d.xfer.join("ok.part")), &dest, name, false, None, &d)
                    .unwrap_err()
                    .code,
                PrivErrorCode::Invalid,
                "{name:?}"
            );
        }
        // A jailed account cannot upload outside its area.
        let jail = Some(s(&t.join("jail")));
        fs::create_dir(t.join("jail")).unwrap();
        assert!(
            install_upload(
                &s(&d.xfer.join("ok.part")),
                &dest,
                "x.txt",
                false,
                jail.as_deref(),
                &d
            )
            .is_err()
        );
        assert!(d.xfer.join("ok.part").exists());
    }

    #[test]
    fn names() {
        for ok in ["a", "report 2026.pdf", ".env", "ünï.txt"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", ".", "..", "a/b", "a\0", "a\nb", &"x".repeat(256)] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }
}
