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

use std::collections::{HashSet, VecDeque};
use std::fs::{self, Metadata};
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use opsbox_proto::{
    DirListing, FileContent, FileEntry, FileKind, PrivError, PrivErrorCode, Staged,
};
use rustix::fs::{
    AtFlags, Dir, FileType, Gid, Mode, OFlags, Stat, Uid, chmodat, fchmod, fchown, fstat, fsync,
    mkdirat, open, openat, readlinkat, renameat, stat, statat, unlinkat,
};
#[cfg(target_os = "linux")]
use rustix::fs::{ResolveFlags, openat2};
use rustix::io::Errno;

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

fn r_io_err(what: &str, e: Errno) -> PrivError {
    io_err(what, &std::io::Error::from(e))
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
#[cfg(test)]
const NO_MODIFY: [&str; 6] = [
    "/var/lib/opsbox/bin",
    "/var/lib/opsbox/opsbox.db",
    "/var/lib/opsbox/opsbox.db-wal",
    "/var/lib/opsbox/opsbox.db-shm",
    "/var/lib/opsbox/xfer",
    "/etc/opsbox/firewall.json",
];
const NO_MODIFY_DIRS: [&str; 3] = [
    "/var/lib/opsbox/bin",
    "/var/lib/opsbox/xfer",
    "/var/lib/opsbox/trash",
];
const NO_MODIFY_FILES: [&str; 4] = [
    "/var/lib/opsbox/opsbox.db",
    "/var/lib/opsbox/opsbox.db-wal",
    "/var/lib/opsbox/opsbox.db-shm",
    "/etc/opsbox/firewall.json",
];
/// Never deleted, whatever else is true.
const NO_DELETE: [&str; 21] = [
    "/", "/bin", "/boot", "/dev", "/etc", "/home", "/lib", "/lib32", "/lib64", "/media", "/mnt",
    "/opt", "/proc", "/root", "/run", "/sbin", "/srv", "/sys", "/usr", "/var", "/var/lib",
];

const MAX_SYMLINK_HOPS: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct DevIno {
    dev: u64,
    ino: u64,
}

#[allow(clippy::unnecessary_cast)]
fn stat_dev(st: &Stat) -> u64 {
    st.st_dev as u64
}

#[allow(clippy::unnecessary_cast)]
fn stat_mode(st: &Stat) -> u32 {
    st.st_mode as u32
}

impl DevIno {
    fn from_stat(st: &Stat) -> Self {
        Self {
            dev: stat_dev(st),
            ino: st.st_ino,
        }
    }
}

/// Device and inode sets for denied, read-only, and undeletable filesystem objects.
#[derive(Debug, Clone)]
struct Policy {
    denied_fs_devs: HashSet<u64>,
    denied_objs: HashSet<DevIno>,
    no_modify_dirs: HashSet<DevIno>,
    no_modify_files: HashSet<DevIno>,
    no_delete_exact: HashSet<DevIno>,
    opsbox_dir: Option<DevIno>,
    opsbox_apps_dir: Option<DevIno>,
}

impl Policy {
    fn current() -> Self {
        let root_dev = stat("/").ok().map(|st| stat_dev(&st));
        let mut denied_fs_devs = HashSet::new();
        let mut denied_objs = HashSet::new();
        for p in DENIED {
            if let Ok(st) = stat(p) {
                let dev = stat_dev(&st);
                if Some(dev) != root_dev {
                    denied_fs_devs.insert(dev);
                }
                denied_objs.insert(DevIno::from_stat(&st));
            }
        }
        for p in DENIED_FILES {
            if let Ok(st) = stat(p) {
                denied_objs.insert(DevIno::from_stat(&st));
            }
        }
        let collect = |paths: &[&str]| -> HashSet<DevIno> {
            paths
                .iter()
                .filter_map(|p| stat(*p).ok().map(|st| DevIno::from_stat(&st)))
                .collect()
        };
        let no_modify_dirs = collect(&NO_MODIFY_DIRS);
        let no_modify_files = collect(&NO_MODIFY_FILES);
        let mut no_delete_exact = collect(&NO_DELETE);
        for p in NO_DELETE
            .iter()
            .copied()
            .chain(["/var/lib/opsbox", "/etc/opsbox"])
        {
            if let Ok(st) = stat(p) {
                no_delete_exact.insert(DevIno::from_stat(&st));
            }
            if let Ok(lst) = statat(rustix::fs::CWD, p, AtFlags::SYMLINK_NOFOLLOW) {
                no_delete_exact.insert(DevIno::from_stat(&lst));
            }
        }
        let opsbox_dir = stat("/var/lib/opsbox")
            .ok()
            .map(|st| DevIno::from_stat(&st));
        let opsbox_apps_dir = stat("/var/lib/opsbox/apps")
            .ok()
            .map(|st| DevIno::from_stat(&st));
        Self {
            denied_fs_devs,
            denied_objs,
            no_modify_dirs,
            no_modify_files,
            no_delete_exact,
            opsbox_dir,
            opsbox_apps_dir,
        }
    }

    fn is_denied_stat(&self, st: &Stat) -> bool {
        self.denied_fs_devs.contains(&stat_dev(st))
            || self.denied_objs.contains(&DevIno::from_stat(st))
    }

    fn check_not_denied(&self, st: &Stat) -> Result<(), PrivError> {
        if self.is_denied_stat(st) {
            Err(err(
                PrivErrorCode::Refused,
                "System areas such as /proc, /sys, /dev and opsbox's master key are not available through the panel.",
            ))
        } else {
            Ok(())
        }
    }

    fn is_unmodifiable_stat(&self, st: &Stat) -> bool {
        let id = DevIno::from_stat(st);
        self.no_modify_dirs.contains(&id) || self.no_modify_files.contains(&id)
    }

    fn check_modifiable_stat(&self, st: &Stat) -> Result<(), PrivError> {
        if self.is_unmodifiable_stat(st) {
            Err(err(
                PrivErrorCode::Refused,
                "This belongs to opsbox itself and is read-only here.",
            ))
        } else {
            Ok(())
        }
    }

    fn is_undeletable_stat(&self, st: &Stat, in_opsbox_non_apps: bool) -> bool {
        let id = DevIno::from_stat(st);
        in_opsbox_non_apps
            || self.no_delete_exact.contains(&id)
            || self.no_modify_dirs.contains(&id)
            || self.no_modify_files.contains(&id)
    }
}

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

#[cfg(test)]
fn is_unmodifiable(p: &Path) -> bool {
    NO_MODIFY.iter().any(|d| p.starts_with(d))
        // The trash and transfer areas are managed by opsbox, never edited by hand.
        || p.starts_with("/var/lib/opsbox/trash")
}

#[cfg(test)]
fn is_undeletable(p: &Path) -> bool {
    NO_DELETE.iter().any(|d| p == Path::new(d))
        || p == Path::new("/var/lib/opsbox")
        || p == Path::new("/etc/opsbox")
        || (p.starts_with("/var/lib/opsbox") && !p.starts_with("/var/lib/opsbox/apps"))
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

fn check_unmodifiable_flag(unmodifiable: bool) -> Result<(), PrivError> {
    if unmodifiable {
        Err(err(
            PrivErrorCode::Refused,
            "This belongs to opsbox itself and is read-only here.",
        ))
    } else {
        Ok(())
    }
}

/// An opened root (either `/` or a deployer's jail directory) and the relative path inside it.
struct RootCtx {
    root_fd: OwnedFd,
    root_lex: PathBuf,
    root_real: Option<PathBuf>,
    jailed: bool,
    policy: Policy,
}

struct ResolvedParent {
    parent_fd: OwnedFd,
    leaf: String,
    display_path: PathBuf,
    unmodifiable: bool,
    in_opsbox_non_apps: bool,
}

struct ResolvedDir {
    dir_fd: OwnedFd,
    display_path: PathBuf,
}

struct WalkState {
    cur_fd: OwnedFd,
    cur_path: PathBuf,
    dir_stack: Vec<(OwnedFd, PathBuf, bool, bool)>,
    unmodifiable: bool,
    in_opsbox_non_apps: bool,
    symlinks_followed: usize,
}

impl RootCtx {
    fn open(path: &str, jail: Option<&str>) -> Result<(Self, Vec<String>), PrivError> {
        let lexical = normalize_lexical(path)?;
        check_not_denied(&lexical)?;
        let policy = Policy::current();
        if let Some(j) = jail {
            let root_lex = normalize_lexical(j)?;
            if !lexical.starts_with(&root_lex) {
                return Err(err(
                    PrivErrorCode::Refused,
                    "That is outside the folders your account may use.",
                ));
            }
            let root_fd = open(
                &root_lex,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| err(PrivErrorCode::NotFound, "Your area does not exist yet."))?;
            let st = fstat(&root_fd).map_err(|e| r_io_err("Cannot open your area", e))?;
            policy.check_not_denied(&st)?;
            let root_real = fs::canonicalize(&root_lex).ok();
            let rel = lexical
                .strip_prefix(&root_lex)
                .unwrap_or(Path::new(""))
                .components()
                .filter_map(|c| match c {
                    Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect();
            Ok((
                Self {
                    root_fd,
                    root_lex,
                    root_real,
                    jailed: true,
                    policy,
                },
                rel,
            ))
        } else {
            let root_fd = open(
                "/",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|e| r_io_err("Cannot open root", e))?;
            let rel = lexical
                .components()
                .filter_map(|c| match c {
                    Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect();
            Ok((
                Self {
                    root_fd,
                    root_lex: PathBuf::from("/"),
                    root_real: None,
                    jailed: false,
                    policy,
                },
                rel,
            ))
        }
    }

    fn init_walk(&self) -> Result<WalkState, PrivError> {
        let cur_fd = self
            .root_fd
            .as_fd()
            .try_clone_to_owned()
            .map_err(|e| io_err("Cannot duplicate directory handle", &e))?;
        let st = fstat(&cur_fd).map_err(|e| r_io_err("Cannot stat root", e))?;
        Ok(WalkState {
            cur_fd,
            cur_path: self.root_lex.clone(),
            dir_stack: Vec::new(),
            unmodifiable: self.policy.is_unmodifiable_stat(&st),
            in_opsbox_non_apps: false,
            symlinks_followed: 0,
        })
    }

    fn handle_symlink(
        &self,
        state: &mut WalkState,
        link_name: &str,
        queue: &mut VecDeque<String>,
    ) -> Result<(), PrivError> {
        state.symlinks_followed += 1;
        if state.symlinks_followed > MAX_SYMLINK_HOPS {
            return Err(r_io_err("Cannot open the path", Errno::LOOP));
        }
        let raw_target = readlinkat(&state.cur_fd, link_name, Vec::new())
            .map_err(|e| r_io_err("Cannot read symbolic link", e))?;
        let target_path = Path::new(std::ffi::OsStr::from_bytes(raw_target.as_bytes()));
        let mut extra = Vec::new();
        if target_path.is_absolute() {
            let stripped = if self.jailed {
                if let Ok(s) = target_path.strip_prefix(&self.root_lex) {
                    s
                } else if let Some(real) = &self.root_real
                    && let Ok(s) = target_path.strip_prefix(real)
                {
                    s
                } else {
                    return Err(err(
                        PrivErrorCode::Refused,
                        "That is outside the folders your account may use.",
                    ));
                }
            } else {
                target_path.strip_prefix("/").unwrap_or(target_path)
            };
            for c in stripped.components() {
                match c {
                    Component::RootDir | Component::CurDir => {}
                    Component::ParentDir => extra.push("..".to_owned()),
                    Component::Normal(n) => extra.push(n.to_string_lossy().into_owned()),
                    Component::Prefix(_) => return Err(invalid("That is not a valid path.")),
                }
            }
            *state = WalkState {
                symlinks_followed: state.symlinks_followed,
                ..self.init_walk()?
            };
        } else {
            for c in target_path.components() {
                match c {
                    Component::RootDir | Component::CurDir => {}
                    Component::ParentDir => extra.push("..".to_owned()),
                    Component::Normal(n) => extra.push(n.to_string_lossy().into_owned()),
                    Component::Prefix(_) => return Err(invalid("That is not a valid path.")),
                }
            }
        }
        for c in extra.into_iter().rev() {
            queue.push_front(c);
        }
        Ok(())
    }

    fn step_dir(
        &self,
        state: &mut WalkState,
        comp: &str,
        queue: &mut VecDeque<String>,
    ) -> Result<(), PrivError> {
        match openat(
            &state.cur_fd,
            comp,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(next_fd) => {
                let st = fstat(&next_fd).map_err(|e| r_io_err("Cannot open the path", e))?;
                self.policy.check_not_denied(&st)?;
                let prev_st = fstat(&state.cur_fd).ok();
                let next_id = DevIno::from_stat(&st);
                let next_unmod = state.unmodifiable || self.policy.is_unmodifiable_stat(&st);
                let next_opsbox_non_apps = if state.in_opsbox_non_apps {
                    true
                } else if let (Some(pst), Some(ob)) = (prev_st, self.policy.opsbox_dir) {
                    DevIno::from_stat(&pst) == ob && Some(next_id) != self.policy.opsbox_apps_dir
                } else {
                    false
                };
                let old_fd = std::mem::replace(&mut state.cur_fd, next_fd);
                let next_path = state.cur_path.join(comp);
                let old_path = std::mem::replace(&mut state.cur_path, next_path);
                state.dir_stack.push((
                    old_fd,
                    old_path,
                    state.unmodifiable,
                    state.in_opsbox_non_apps,
                ));
                state.unmodifiable = next_unmod;
                state.in_opsbox_non_apps = next_opsbox_non_apps;
                Ok(())
            }
            Err(Errno::LOOP) => self.handle_symlink(state, comp, queue),
            Err(Errno::NOENT) => Err(err(PrivErrorCode::NotFound, "The folder does not exist.")),
            Err(Errno::NOTDIR) => {
                if let Ok(st) = statat(&state.cur_fd, comp, AtFlags::SYMLINK_NOFOLLOW)
                    && FileType::from_raw_mode(st.st_mode as _).is_symlink()
                {
                    return self.handle_symlink(state, comp, queue);
                }
                Err(invalid(format!(
                    "{} is not a folder.",
                    state.cur_path.join(comp).display()
                )))
            }
            Err(e) => Err(r_io_err("Cannot open the path", e)),
        }
    }

    fn step_parent(&self, state: &mut WalkState) -> Result<(), PrivError> {
        if let Some((fd, path, unmod, opsbox_non_apps)) = state.dir_stack.pop() {
            state.cur_fd = fd;
            state.cur_path = path;
            state.unmodifiable = unmod;
            state.in_opsbox_non_apps = opsbox_non_apps;
            Ok(())
        } else if self.jailed {
            Err(err(
                PrivErrorCode::Refused,
                "That is outside the folders your account may use.",
            ))
        } else {
            Ok(())
        }
    }

    /// Walk component-by-component with `O_NOFOLLOW | O_DIRECTORY` on every parent, stopping at
    /// `(parent_fd, leaf_name)`. When `follow_leaf_symlink` is true, a symlink at the leaf is
    /// followed within the jail.
    fn resolve_parent_and_leaf(
        &self,
        rel: &[String],
        follow_leaf_symlink: bool,
    ) -> Result<ResolvedParent, PrivError> {
        let mut state = self.init_walk()?;
        let mut queue: VecDeque<String> = rel.iter().cloned().collect();
        while let Some(comp) = queue.pop_front() {
            if comp.is_empty() || comp == "." {
                continue;
            }
            if comp == ".." {
                self.step_parent(&mut state)?;
                continue;
            }
            if queue.is_empty() {
                if follow_leaf_symlink
                    && let Ok(st) = statat(&state.cur_fd, &comp, AtFlags::SYMLINK_NOFOLLOW)
                    && FileType::from_raw_mode(st.st_mode as _).is_symlink()
                {
                    self.handle_symlink(&mut state, &comp, &mut queue)?;
                    continue;
                }
                let display_path = state.cur_path.join(&comp);
                let prev_st = fstat(&state.cur_fd).ok();
                let leaf_opsbox_non_apps = if state.in_opsbox_non_apps {
                    true
                } else if let (Some(pst), Some(ob)) = (prev_st, self.policy.opsbox_dir) {
                    if DevIno::from_stat(&pst) == ob {
                        match statat(&state.cur_fd, &comp, AtFlags::SYMLINK_NOFOLLOW) {
                            Ok(lst) => Some(DevIno::from_stat(&lst)) != self.policy.opsbox_apps_dir,
                            Err(_) => true,
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };
                return Ok(ResolvedParent {
                    parent_fd: state.cur_fd,
                    leaf: comp,
                    display_path,
                    unmodifiable: state.unmodifiable,
                    in_opsbox_non_apps: leaf_opsbox_non_apps,
                });
            }
            self.step_dir(&mut state, &comp, &mut queue)?;
        }
        Err(invalid("That is not a valid path."))
    }

    #[cfg(target_os = "linux")]
    fn try_openat2(&self, rel: &[String], oflags: OFlags) -> Option<Result<OwnedFd, PrivError>> {
        if rel.is_empty() {
            return None;
        }
        let rel_path: PathBuf = rel.iter().collect();
        match openat2(
            &self.root_fd,
            &rel_path,
            oflags,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS,
        ) {
            Ok(fd) => Some(Ok(fd)),
            // Fall back to component walk when openat2 is unavailable (< 5.6) or encounters an
            // intra-jail absolute symlink (EXDEV).
            Err(Errno::NOSYS | Errno::XDEV) => None,
            // RESOLVE_BENEATH returns EAGAIN when a concurrent rename prevents the
            // kernel from proving that `..` stays inside the root. Fail closed.
            Err(Errno::AGAIN) => Some(Err(err(
                PrivErrorCode::Refused,
                "The path changed while opening it. Try again.",
            ))),
            Err(Errno::NOENT) => Some(Err(err(
                PrivErrorCode::NotFound,
                "No such file or directory.",
            ))),
            Err(Errno::NOTDIR) => Some(Err(invalid("Not a folder."))),
            Err(e) => Some(Err(r_io_err("Cannot open the path", e))),
        }
    }

    fn resolve_dir(&self, rel: &[String]) -> Result<ResolvedDir, PrivError> {
        if rel.is_empty() {
            let dir_fd = self
                .root_fd
                .as_fd()
                .try_clone_to_owned()
                .map_err(|e| io_err("Cannot duplicate directory handle", &e))?;
            let st = fstat(&dir_fd).map_err(|e| r_io_err("Cannot open the folder", e))?;
            self.policy.check_not_denied(&st)?;
            return Ok(ResolvedDir {
                dir_fd,
                display_path: self.root_lex.clone(),
            });
        }
        #[cfg(target_os = "linux")]
        if let Some(res) =
            self.try_openat2(rel, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC)
        {
            let dir_fd = res?;
            let st = fstat(&dir_fd).map_err(|e| r_io_err("Cannot open the folder", e))?;
            self.policy.check_not_denied(&st)?;
            let mut display_path = self.root_lex.clone();
            for c in rel {
                display_path.push(c);
            }
            return Ok(ResolvedDir {
                dir_fd,
                display_path,
            });
        }
        let mut state = self.init_walk()?;
        let mut queue: VecDeque<String> = rel.iter().cloned().collect();
        while let Some(comp) = queue.pop_front() {
            if comp.is_empty() || comp == "." {
                continue;
            }
            if comp == ".." {
                self.step_parent(&mut state)?;
                continue;
            }
            self.step_dir(&mut state, &comp, &mut queue)?;
        }
        let st = fstat(&state.cur_fd).map_err(|e| r_io_err("Cannot open the folder", e))?;
        self.policy.check_not_denied(&st)?;
        Ok(ResolvedDir {
            dir_fd: state.cur_fd,
            display_path: state.cur_path,
        })
    }

    fn resolve_open_file(&self, rel: &[String]) -> Result<(OwnedFd, Stat, PathBuf), PrivError> {
        if rel.is_empty() {
            return Err(invalid("That is not a regular file."));
        }
        #[cfg(target_os = "linux")]
        if let Some(res) = self.try_openat2(rel, OFlags::RDONLY | OFlags::CLOEXEC) {
            let fd = res?;
            let st = fstat(&fd).map_err(|e| r_io_err("Cannot open the file", e))?;
            self.policy.check_not_denied(&st)?;
            let mut display_path = self.root_lex.clone();
            for c in rel {
                display_path.push(c);
            }
            return Ok((fd, st, display_path));
        }
        let rp = self.resolve_parent_and_leaf(rel, true)?;
        let fd = openat(
            &rp.parent_fd,
            &rp.leaf,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| r_io_err("Cannot open the file", e))?;
        let st = fstat(&fd).map_err(|e| r_io_err("Cannot open the file", e))?;
        self.policy.check_not_denied(&st)?;
        Ok((fd, st, rp.display_path))
    }
}

/// Resolve `path` for reading or modifying a file or folder that may or may not exist yet.
#[cfg_attr(not(test), allow(dead_code))]
pub fn resolve(path: &str, jail: Option<&str>, must_exist: bool) -> Result<PathBuf, PrivError> {
    let (ctx, rel) = RootCtx::open(path, jail)?;
    if rel.is_empty() {
        let st = fstat(&ctx.root_fd).map_err(|e| r_io_err("Cannot open the path", e))?;
        ctx.policy.check_not_denied(&st)?;
        return Ok(ctx.root_lex);
    }
    let rp = ctx.resolve_parent_and_leaf(&rel, true)?;
    match statat(&rp.parent_fd, &rp.leaf, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(st) => {
            ctx.policy.check_not_denied(&st)?;
            Ok(rp.display_path)
        }
        Err(_) if !must_exist => Ok(rp.display_path),
        Err(e) => Err(r_io_err("Cannot open the path", e)),
    }
}

// ------------------------------------------------------------ small helpers

fn mode_string_raw(m: u32, is_dir: bool, is_link: bool) -> String {
    let kind = if is_dir {
        'd'
    } else if is_link {
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

/// `ls`-style mode string.
#[cfg_attr(not(test), allow(dead_code))]
pub fn mode_string(meta: &Metadata) -> String {
    mode_string_raw(meta.mode(), meta.is_dir(), meta.file_type().is_symlink())
}

fn mode_string_stat(st: &Stat) -> String {
    let ft = FileType::from_raw_mode(st.st_mode as _);
    mode_string_raw(stat_mode(st), ft.is_dir(), ft.is_symlink())
}

fn stat_mtime(st: &Stat) -> i64 {
    st.st_mtime
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

// ------------------------------------------------------------------ list

pub(super) fn list(path: &str, jail: Option<&str>) -> Result<DirListing, PrivError> {
    let (ctx, rel) = RootCtx::open(path, jail)?;
    let resolved = ctx.resolve_dir(&rel)?;
    let mut dir =
        Dir::read_from(&resolved.dir_fd).map_err(|e| r_io_err("Cannot read the folder", e))?;
    let mut entries = Vec::new();
    while let Some(res) = dir.read() {
        if entries.len() >= MAX_ENTRIES {
            break;
        }
        let Ok(e) = res else { continue };
        let cname = e.file_name();
        let name = cname.to_string_lossy().into_owned();
        if name == "." || name == ".." {
            continue;
        }
        let Ok(lst) = statat(&resolved.dir_fd, cname, AtFlags::SYMLINK_NOFOLLOW) else {
            continue;
        };
        let is_link = FileType::from_raw_mode(lst.st_mode as _).is_symlink();
        let (st, kind) = if is_link {
            match statat(&resolved.dir_fd, cname, AtFlags::empty()) {
                Ok(m) => {
                    let k = if FileType::from_raw_mode(m.st_mode as _).is_dir() {
                        FileKind::Dir
                    } else {
                        FileKind::File
                    };
                    (m, k)
                }
                Err(_) => (lst, FileKind::BrokenLink),
            }
        } else {
            let k = if FileType::from_raw_mode(lst.st_mode as _).is_dir() {
                FileKind::Dir
            } else {
                FileKind::File
            };
            (lst, k)
        };
        let mode_bits = stat_mode(&st);
        entries.push(FileEntry {
            path: resolved
                .display_path
                .join(&name)
                .to_string_lossy()
                .into_owned(),
            kind,
            mode: mode_string_stat(&st),
            mtime: stat_mtime(&st),
            size: (kind != FileKind::Dir).then_some(st.st_size.max(0) as u64),
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
        path: resolved.display_path.to_string_lossy().into_owned(),
        entries,
    })
}

// ------------------------------------------------------------------ read

pub(super) fn read(path: &str, jail: Option<&str>) -> Result<FileContent, PrivError> {
    let (ctx, rel) = RootCtx::open(path, jail)?;
    let (fd, st, display_path) = ctx.resolve_open_file(&rel)?;
    if !FileType::from_raw_mode(st.st_mode as _).is_file() {
        return Err(invalid("That is not a regular file."));
    }
    let size = st.st_size.max(0) as u64;
    if size > MAX_EDIT_BYTES {
        return Err(invalid(format!(
            "This file is {:.1} MB, too big to edit in the browser (limit {} MB). Download it instead.",
            size as f64 / 1_048_576.0,
            MAX_EDIT_BYTES / 1_048_576
        )));
    }
    let mut file = fs::File::from(fd);
    let mut bytes = Vec::with_capacity(size as usize);
    file.read_to_end(&mut bytes)
        .map_err(|e| io_err("Cannot read the file", &e))?;
    if is_binary(&bytes) {
        return Err(invalid(
            "This file is not text, so it cannot be edited here. Download it instead.",
        ));
    }
    Ok(FileContent {
        path: display_path.to_string_lossy().into_owned(),
        content: String::from_utf8(bytes).unwrap_or_default(),
        size,
        mode: mode_string_stat(&st),
        mtime: stat_mtime(&st),
    })
}

// ----------------------------------------------------------------- write

/// Write `bytes` to a temporary file inside `parent_fd` with `O_EXCL`, `fsync`, and `renameat`.
fn write_atomic_at(
    parent_fd: impl AsFd,
    target_name: &str,
    bytes: &[u8],
    mode: u32,
    uid: u32,
    gid: u32,
) -> Result<(), PrivError> {
    let dir = parent_fd.as_fd();
    let mut rnd = [0u8; 6];
    getrandom::fill(&mut rnd)
        .map_err(|_| err(PrivErrorCode::Internal, "random number generator failed"))?;
    let suffix: String = rnd.iter().map(|b| format!("{b:02x}")).collect();
    let tmp_name = format!(".{target_name}.opsbox-{suffix}.tmp");
    let result = (|| {
        let tmp_fd = openat(
            dir,
            &tmp_name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|e| r_io_err("Cannot create the file", e))?;
        let mut f = fs::File::from(tmp_fd);
        f.write_all(bytes)
            .map_err(|e| io_err("Cannot write the file", &e))?;
        f.sync_all()
            .map_err(|e| io_err("Cannot flush the file", &e))?;
        fchmod(&f, Mode::from_raw_mode((mode & 0o7777) as _))
            .map_err(|e| r_io_err("Cannot set the permissions", e))?;
        fchown(&f, Some(Uid::from_raw(uid)), Some(Gid::from_raw(gid)))
            .map_err(|e| r_io_err("Cannot set the owner", e))?;
        renameat(dir, &tmp_name, dir, target_name)
            .map_err(|e| r_io_err("Cannot replace the file", e))
    })();
    if result.is_err() {
        let _ = unlinkat(dir, &tmp_name, AtFlags::empty());
    }
    result?;
    // Make the rename itself durable; failure here is not worth failing the save for.
    let _ = fsync(dir);
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
    let (ctx, rel) = RootCtx::open(path, jail)?;
    if rel.is_empty() {
        return Err(invalid("That is not a regular file."));
    }
    let rp = ctx.resolve_parent_and_leaf(&rel, true)?;
    check_unmodifiable_flag(rp.unmodifiable)?;
    let (mode, uid, gid) = match statat(&rp.parent_fd, &rp.leaf, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(st) => {
            ctx.policy.check_not_denied(&st)?;
            ctx.policy.check_modifiable_stat(&st)?;
            if !FileType::from_raw_mode(st.st_mode as _).is_file() {
                return Err(invalid("That is not a regular file."));
            }
            if let Some(expected) = expect_mtime
                && stat_mtime(&st) != expected
            {
                return Err(err(
                    PrivErrorCode::Conflict,
                    "This file changed on the server since you opened it. Reload it, then make your edit again so nothing is lost.",
                ));
            }
            (stat_mode(&st), st.st_uid, st.st_gid)
        }
        Err(Errno::NOENT) => {
            let parent_st =
                fstat(&rp.parent_fd).map_err(|e| r_io_err("Cannot open the folder", e))?;
            ctx.policy.check_not_denied(&parent_st)?;
            (0o644, parent_st.st_uid, parent_st.st_gid)
        }
        Err(e) => return Err(r_io_err("Cannot open the file", e)),
    };
    write_atomic_at(&rp.parent_fd, &rp.leaf, content.as_bytes(), mode, uid, gid)?;
    Ok(statat(&rp.parent_fd, &rp.leaf, AtFlags::SYMLINK_NOFOLLOW)
        .map(|st| stat_mtime(&st))
        .unwrap_or(0))
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
    let (ctx, rel) = RootCtx::open(path, jail)?;
    if rel.is_empty() {
        let st = fstat(&ctx.root_fd).map_err(|e| r_io_err("Cannot open the path", e))?;
        ctx.policy.check_not_denied(&st)?;
        ctx.policy.check_modifiable_stat(&st)?;
        return fchmod(&ctx.root_fd, Mode::from_raw_mode(bits as _))
            .map_err(|e| r_io_err("Cannot change the permissions", e));
    }
    let rp = ctx.resolve_parent_and_leaf(&rel, true)?;
    check_unmodifiable_flag(rp.unmodifiable)?;
    let st = statat(&rp.parent_fd, &rp.leaf, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|e| r_io_err("Cannot open the path", e))?;
    ctx.policy.check_not_denied(&st)?;
    ctx.policy.check_modifiable_stat(&st)?;
    match openat(
        &rp.parent_fd,
        &rp.leaf,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => {
            let fst = fstat(&fd).map_err(|e| r_io_err("Cannot open the path", e))?;
            ctx.policy.check_not_denied(&fst)?;
            ctx.policy.check_modifiable_stat(&fst)?;
            fchmod(&fd, Mode::from_raw_mode(bits as _))
                .map_err(|e| r_io_err("Cannot change the permissions", e))
        }
        Err(Errno::ACCESS) => chmodat(
            &rp.parent_fd,
            &rp.leaf,
            Mode::from_raw_mode(bits as _),
            AtFlags::empty(),
        )
        .map_err(|e| r_io_err("Cannot change the permissions", e)),
        Err(e) => Err(r_io_err("Cannot change the permissions", e)),
    }
}

pub(super) fn mkdir(path: &str, jail: Option<&str>) -> Result<(), PrivError> {
    let (ctx, rel) = RootCtx::open(path, jail)?;
    if rel.is_empty() {
        return Err(err(
            PrivErrorCode::Conflict,
            "Something with that name already exists.",
        ));
    }
    let rp = ctx.resolve_parent_and_leaf(&rel, false)?;
    check_unmodifiable_flag(rp.unmodifiable)?;
    if statat(&rp.parent_fd, &rp.leaf, AtFlags::SYMLINK_NOFOLLOW).is_ok() {
        return Err(err(
            PrivErrorCode::Conflict,
            "Something with that name already exists.",
        ));
    }
    if !valid_name(&rp.leaf) {
        return Err(invalid("That is not a valid folder name."));
    }
    let parent_st = fstat(&rp.parent_fd).map_err(|e| r_io_err("Cannot open the folder", e))?;
    ctx.policy.check_not_denied(&parent_st)?;
    mkdirat(&rp.parent_fd, &rp.leaf, Mode::from_raw_mode(0o755))
        .map_err(|e| r_io_err("Cannot create the folder", e))?;
    let new_fd = openat(
        &rp.parent_fd,
        &rp.leaf,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| r_io_err("Cannot open the new folder", e))?;
    fchmod(&new_fd, Mode::from_raw_mode(0o755))
        .map_err(|e| r_io_err("Cannot set the permissions", e))?;
    fchown(
        &new_fd,
        Some(Uid::from_raw(parent_st.st_uid)),
        Some(Gid::from_raw(parent_st.st_gid)),
    )
    .map_err(|e| r_io_err("Cannot set the owner", e))
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
        let (ctx, rel) = RootCtx::open(raw, jail)?;
        if rel.is_empty() {
            return Err(err(PrivErrorCode::Refused, "That cannot be removed."));
        }
        let rp = ctx.resolve_parent_and_leaf(&rel, false)?;
        check_unmodifiable_flag(rp.unmodifiable)?;
        let st = statat(&rp.parent_fd, &rp.leaf, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|e| r_io_err("Cannot find it", e))?;
        ctx.policy.check_not_denied(&st)?;
        ctx.policy.check_modifiable_stat(&st)?;
        if ctx.policy.is_undeletable_stat(&st, rp.in_opsbox_non_apps) {
            return Err(err(
                PrivErrorCode::Refused,
                format!(
                    "{} is part of the system or of opsbox and is never deleted from the panel.",
                    rp.display_path.display()
                ),
            ));
        }
        targets.push(rp);
    }
    let mut n = 0;
    for rp in targets {
        if permanent {
            delete_forever_at(&rp.parent_fd, &rp.leaf, &rp.display_path)?;
        } else {
            move_to_trash_at(&rp.parent_fd, &rp.leaf, &rp.display_path, dirs)?;
        }
        n += 1;
    }
    Ok(n)
}

fn remove_dir_all_at(parent_fd: impl AsFd, name: &str) -> Result<(), PrivError> {
    let dir_fd = openat(
        parent_fd.as_fd(),
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| r_io_err(&format!("Cannot delete {name}"), e))?;
    let mut dir =
        Dir::read_from(&dir_fd).map_err(|e| r_io_err(&format!("Cannot read {name}"), e))?;
    while let Some(entry) = dir.read() {
        let entry = entry.map_err(|e| r_io_err(&format!("Cannot read {name}"), e))?;
        let cname = entry.file_name();
        let entry_name = cname.to_string_lossy();
        if entry_name == "." || entry_name == ".." {
            continue;
        }
        let st = statat(&dir_fd, cname, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|e| r_io_err(&format!("Cannot find {entry_name}"), e))?;
        if FileType::from_raw_mode(st.st_mode as _).is_dir() {
            remove_dir_all_at(&dir_fd, &entry_name)?;
        } else {
            unlinkat(&dir_fd, cname, AtFlags::empty())
                .map_err(|e| r_io_err(&format!("Cannot delete {entry_name}"), e))?;
        }
    }
    unlinkat(parent_fd.as_fd(), name, AtFlags::REMOVEDIR)
        .map_err(|e| r_io_err(&format!("Cannot delete {name}"), e))
}

fn delete_forever_at(
    parent_fd: impl AsFd,
    leaf: &str,
    display_path: &Path,
) -> Result<(), PrivError> {
    let st = statat(parent_fd.as_fd(), leaf, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|e| r_io_err("Cannot find it", e))?;
    if FileType::from_raw_mode(st.st_mode as _).is_dir() {
        remove_dir_all_at(parent_fd.as_fd(), leaf).map_err(|_| {
            err(
                PrivErrorCode::Internal,
                format!("Cannot delete {}", display_path.display()),
            )
        })
    } else {
        unlinkat(parent_fd.as_fd(), leaf, AtFlags::empty())
            .map_err(|e| r_io_err(&format!("Cannot delete {}", display_path.display()), e))
    }
}

fn move_to_trash_at(
    parent_fd: impl AsFd,
    leaf: &str,
    display_path: &Path,
    dirs: &Dirs,
) -> Result<(), PrivError> {
    fs::create_dir_all(&dirs.trash).map_err(|e| io_err("Cannot create the trash", &e))?;
    let trash_fd = open(
        &dirs.trash,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| r_io_err("Cannot open the trash", e))?;
    let _ = fchmod(&trash_fd, Mode::from_raw_mode(0o700));
    let now = now_secs();
    let mut rnd = [0u8; 3];
    getrandom::fill(&mut rnd)
        .map_err(|_| err(PrivErrorCode::Internal, "random number generator failed"))?;
    let id = format!(
        "{now}-{}-{leaf}",
        rnd.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    let st = statat(parent_fd.as_fd(), leaf, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|e| r_io_err("Cannot find it", e))?;
    match renameat(parent_fd.as_fd(), leaf, &trash_fd, &id) {
        Ok(()) => {}
        Err(Errno::XDEV) => {
            if FileType::from_raw_mode(st.st_mode as _).is_dir() {
                return Err(invalid(
                    "This folder is on a different disk from the trash, so it cannot be moved there. Delete it permanently instead.",
                ));
            }
            let src_fd = openat(
                parent_fd.as_fd(),
                leaf,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|e| r_io_err("Cannot read the file", e))?;
            let dst_fd = openat(
                &trash_fd,
                &id,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(|e| r_io_err("Cannot copy to the trash", e))?;
            let mut src_file = fs::File::from(src_fd);
            let mut dst_file = fs::File::from(dst_fd);
            std::io::copy(&mut src_file, &mut dst_file)
                .map_err(|e| io_err("Cannot copy to the trash", &e))?;
            unlinkat(parent_fd.as_fd(), leaf, AtFlags::empty())
                .map_err(|e| r_io_err("Cannot remove the original", e))?;
        }
        Err(e) => {
            return Err(r_io_err(
                &format!("Cannot move {} to the trash", display_path.display()),
                e,
            ));
        }
    }
    let note = serde_json::json!({ "original": display_path.to_string_lossy(), "deleted_at": now })
        .to_string();
    if let Ok(meta_fd) = openat(
        &trash_fd,
        format!("{id}.meta"),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    ) {
        let mut f = fs::File::from(meta_fd);
        let _ = f.write_all(note.as_bytes());
    }
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

fn check_staging_at(staging: &str, dirs: &Dirs) -> Result<(OwnedFd, String, OwnedFd), PrivError> {
    let lexical = normalize_lexical(staging)?;
    let xfer_fd = open(
        &dirs.xfer,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| {
        err(
            PrivErrorCode::Internal,
            "The transfer folder does not exist.",
        )
    })?;
    let xfer_st = fstat(&xfer_fd).map_err(|_| {
        err(
            PrivErrorCode::Internal,
            "The transfer folder does not exist.",
        )
    })?;
    let name = lexical
        .file_name()
        .and_then(|n| n.to_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            err(
                PrivErrorCode::Refused,
                "That is not a file in the transfer folder.",
            )
        })?;
    let parent = lexical.parent().unwrap_or(Path::new("/"));
    let parent_fd = open(
        parent,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| {
        err(
            PrivErrorCode::Refused,
            "That is not a file in the transfer folder.",
        )
    })?;
    let parent_st = fstat(&parent_fd).map_err(|_| {
        err(
            PrivErrorCode::Refused,
            "That is not a file in the transfer folder.",
        )
    })?;
    if DevIno::from_stat(&parent_st) != DevIno::from_stat(&xfer_st) {
        return Err(err(
            PrivErrorCode::Refused,
            "That is not a file in the transfer folder.",
        ));
    }
    let file_fd = openat(
        &xfer_fd,
        &name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| r_io_err("Cannot find the transfer file", e))?;
    let file_st = fstat(&file_fd).map_err(|e| r_io_err("Cannot find the transfer file", e))?;
    if !FileType::from_raw_mode(file_st.st_mode as _).is_file() {
        return Err(err(
            PrivErrorCode::Refused,
            "The transfer file is not a regular file.",
        ));
    }
    Ok((xfer_fd, name, file_fd))
}

pub(super) fn stage_download(
    path: &str,
    jail: Option<&str>,
    dirs: &Dirs,
) -> Result<Staged, PrivError> {
    let (ctx, rel) = RootCtx::open(path, jail)?;
    let (src_fd, src_st, display_path) = ctx.resolve_open_file(&rel)?;
    if !FileType::from_raw_mode(src_st.st_mode as _).is_file() {
        return Err(invalid("Only files can be downloaded (not folders)."));
    }
    let xfer_fd = open(
        &dirs.xfer,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| {
        err(
            PrivErrorCode::Internal,
            "The transfer folder does not exist.",
        )
    })?;
    let xfer_st = fstat(&xfer_fd).map_err(|e| r_io_err("Cannot open the transfer folder", e))?;
    let mut rnd = [0u8; 8];
    getrandom::fill(&mut rnd)
        .map_err(|_| err(PrivErrorCode::Internal, "random number generator failed"))?;
    let dl_name = format!(
        "{}.dl",
        rnd.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    let out_fd = openat(
        &xfer_fd,
        &dl_name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|e| r_io_err("Cannot create the transfer file", e))?;
    let mut input = fs::File::from(src_fd);
    let mut out = fs::File::from(out_fd);
    if let Err(e) = std::io::copy(&mut input, &mut out).and_then(|_| out.sync_all()) {
        let _ = unlinkat(&xfer_fd, &dl_name, AtFlags::empty());
        return Err(io_err("Cannot copy the file", &e));
    }
    fchown(
        &out,
        Some(Uid::from_raw(xfer_st.st_uid)),
        Some(Gid::from_raw(xfer_st.st_gid)),
    )
    .map_err(|e| r_io_err("Cannot set the owner", e))?;
    let xfer_real = fs::canonicalize(&dirs.xfer).unwrap_or_else(|_| dirs.xfer.clone());
    Ok(Staged {
        staging: xfer_real.join(&dl_name).to_string_lossy().into_owned(),
        name: display_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size: src_st.st_size.max(0) as u64,
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
    let (xfer_fd, staged_name, input_fd) = check_staging_at(staging, dirs)?;
    let (ctx, mut rel) = RootCtx::open(dir, jail)?;
    rel.push(name.to_owned());
    let rp = ctx.resolve_parent_and_leaf(&rel, false)?;
    check_unmodifiable_flag(rp.unmodifiable)?;
    let dest_dir_st = fstat(&rp.parent_fd).map_err(|e| r_io_err("Cannot open the folder", e))?;
    ctx.policy.check_not_denied(&dest_dir_st)?;
    let (mode, uid, gid) = match statat(&rp.parent_fd, &rp.leaf, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(st) => {
            ctx.policy.check_not_denied(&st)?;
            ctx.policy.check_modifiable_stat(&st)?;
            if FileType::from_raw_mode(st.st_mode as _).is_file() && overwrite {
                (stat_mode(&st), st.st_uid, st.st_gid)
            } else if !overwrite {
                return Err(err(
                    PrivErrorCode::Conflict,
                    format!("A file named {name} already exists here."),
                ));
            } else {
                return Err(invalid(
                    "Something that is not a regular file already has that name.",
                ));
            }
        }
        Err(Errno::NOENT) => (0o644, dest_dir_st.st_uid, dest_dir_st.st_gid),
        Err(e) => return Err(r_io_err("Cannot open the destination", e)),
    };
    let mut input = fs::File::from(input_fd);
    let mut rnd = [0u8; 6];
    getrandom::fill(&mut rnd)
        .map_err(|_| err(PrivErrorCode::Internal, "random number generator failed"))?;
    let tmp_name = format!(
        ".{name}.opsbox-{}.tmp",
        rnd.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    let result = (|| {
        let out_fd = openat(
            &rp.parent_fd,
            &tmp_name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|e| r_io_err("Cannot create the file", e))?;
        let mut out = fs::File::from(out_fd);
        std::io::copy(&mut input, &mut out).map_err(|e| io_err("Cannot write the file", &e))?;
        out.sync_all()
            .map_err(|e| io_err("Cannot flush the file", &e))?;
        fchmod(&out, Mode::from_raw_mode((mode & 0o7777) as _))
            .map_err(|e| r_io_err("Cannot set the permissions", e))?;
        fchown(&out, Some(Uid::from_raw(uid)), Some(Gid::from_raw(gid)))
            .map_err(|e| r_io_err("Cannot set the owner", e))?;
        renameat(&rp.parent_fd, &tmp_name, &rp.parent_fd, &rp.leaf)
            .map_err(|e| r_io_err("Cannot move the file into place", e))
    })();
    if result.is_err() {
        let _ = unlinkat(&rp.parent_fd, &tmp_name, AtFlags::empty());
    }
    result?;
    let _ = unlinkat(&xfer_fd, &staged_name, AtFlags::empty());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
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

    #[test]
    fn concurrent_parent_swap_during_read_write_and_delete_never_escapes_jail() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let (_t, root) = tmp();
        let d = dirs(&root);
        let jail = root.join("jail");
        let outside = root.join("outside");
        let real_sub = jail.join("real_sub");
        fs::create_dir_all(&real_sub).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(real_sub.join("secret"), "inside").unwrap();
        fs::write(outside.join("secret"), "TOP_SECRET").unwrap();

        let sub = jail.join("sub");
        symlink(&real_sub, &sub).unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let stop_bg = Arc::clone(&stop);
        let sub_bg = sub.clone();
        let real_sub_bg = real_sub.clone();
        let outside_bg = outside.clone();
        let jail_bg = jail.clone();

        let swapper = std::thread::spawn(move || {
            let mut i = 0u64;
            while !stop_bg.load(Ordering::Relaxed) {
                let tmp_link = jail_bg.join(format!("swap-{i}.tmp"));
                let target = if i.is_multiple_of(2) {
                    &outside_bg
                } else {
                    &real_sub_bg
                };
                let _ = symlink(target, &tmp_link);
                let _ = fs::rename(&tmp_link, &sub_bg);
                if !real_sub_bg.join("secret").exists() {
                    let _ = fs::write(real_sub_bg.join("secret"), "inside");
                }
                i += 1;
            }
        });

        let j = Some(s(&jail));
        let target = s(&sub.join("secret"));
        for i in 0..200 {
            if let Ok(c) = read(&target, j.as_deref()) {
                assert_ne!(c.content, "TOP_SECRET", "read escaped jail at iter {i}");
            }
            let _ = write(&target, "pwned", None, j.as_deref());
            let _ = remove(std::slice::from_ref(&target), true, j.as_deref(), &d);
        }

        stop.store(true, Ordering::Relaxed);
        swapper.join().unwrap();

        assert_eq!(
            fs::read_to_string(outside.join("secret")).unwrap(),
            "TOP_SECRET",
            "outside secret must never be modified or deleted"
        );
    }

    #[test]
    fn denied_paths_via_symlink_and_dot_dot_fail() {
        let (_t, root) = tmp();
        let d = dirs(&root);
        symlink("/dev/null", root.join("to_dev_null")).unwrap();
        symlink("/dev", root.join("to_dev")).unwrap();
        let mut rel = String::new();
        for _ in 0..16 {
            rel.push_str("../");
        }
        rel.push_str("dev/null");
        symlink(&rel, root.join("rel_dev_null")).unwrap();

        for p in [
            s(&root.join("to_dev_null")),
            s(&root.join("to_dev/null")),
            s(&root.join("rel_dev_null")),
            "/tmp/../dev/null".to_owned(),
        ] {
            let read_error = read(&p, None).unwrap_err();
            assert_eq!(
                read_error.code,
                PrivErrorCode::Refused,
                "read {p}: {read_error:?}"
            );
            assert_eq!(
                write(&p, "x", None, None).unwrap_err().code,
                PrivErrorCode::Refused,
                "write {p}"
            );
            assert_eq!(
                chmod(&p, "600", None).unwrap_err().code,
                PrivErrorCode::Refused,
                "chmod {p}"
            );
            let download_error = stage_download(&p, None, &d).unwrap_err();
            assert_eq!(
                download_error.code,
                PrivErrorCode::Refused,
                "stage_download {p}: {download_error:?}"
            );
        }
        for p in [s(&root.join("to_dev/null")), "/tmp/../dev/null".to_owned()] {
            assert_eq!(
                remove(std::slice::from_ref(&p), true, None, &d)
                    .unwrap_err()
                    .code,
                PrivErrorCode::Refused,
                "remove {p}"
            );
        }
        assert_eq!(
            mkdir(&s(&root.join("to_dev/opsbox_nope")), None)
                .unwrap_err()
                .code,
            PrivErrorCode::Refused
        );
    }

    #[test]
    fn staging_cannot_escape_xfer() {
        let (_t, root) = tmp();
        let d = dirs(&root);
        let dest = root.join("dest");
        let outside = root.join("outside");
        let xfer_sibling = root.join("xfer_evil");
        fs::create_dir_all(&dest).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(&xfer_sibling).unwrap();
        fs::create_dir_all(d.xfer.join("sub")).unwrap();

        fs::write(outside.join("payload.part"), "secret").unwrap();
        fs::write(xfer_sibling.join("payload.part"), "secret").unwrap();
        symlink(outside.join("payload.part"), d.xfer.join("link_out.part")).unwrap();
        symlink(&outside, d.xfer.join("dir_link")).unwrap();

        let bad_stagings = [
            format!("{}/../outside/payload.part", d.xfer.display()),
            format!("{}/sub/../../outside/payload.part", d.xfer.display()),
            s(&d.xfer.join("link_out.part")),
            s(&d.xfer.join("dir_link/payload.part")),
            s(&xfer_sibling.join("payload.part")),
        ];
        for staging in bad_stagings {
            let res = install_upload(&staging, &s(&dest), "out.txt", false, None, &d);
            assert!(res.is_err(), "staging {staging} must be refused");
            assert!(
                !dest.join("out.txt").exists(),
                "staging {staging} must not create destination file"
            );
        }
    }
}
