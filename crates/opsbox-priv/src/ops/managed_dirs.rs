//! Descriptor-based access to helper-owned state directories.
use std::os::fd::{AsFd, OwnedFd};
use std::path::Path;

use opsbox_proto::{PrivError, PrivErrorCode};
use rustix::fs::{Gid, Mode, OFlags, Uid, fchmod, fchown, fstat, mkdirat, open, openat};
use rustix::io::Errno;

const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

fn refused() -> PrivError {
    PrivError { code: PrivErrorCode::Refused, message: "Unsafe opsbox state directory. Re-run the installer to repair its ownership and permissions.".into() }
}

fn checked(fd: OwnedFd, group_write: bool) -> Result<OwnedFd, PrivError> {
    let st = fstat(&fd).map_err(|_| refused())?;
    // The helper runs as root. Using its effective uid also supports unprivileged
    // helper fixtures; a caller-controlled uid never enters this check.
    let forbidden = if group_write { 0o7007 } else { 0o7022 };
    if st.st_uid != nix::unistd::geteuid().as_raw() || st.st_mode & forbidden != 0 {
        return Err(refused());
    }
    Ok(fd)
}

pub(super) fn open_dir(path: &Path, group_write: bool) -> Result<OwnedFd, PrivError> {
    checked(
        open(path, DIR_FLAGS, Mode::empty()).map_err(|_| refused())?,
        group_write,
    )
}

pub(super) fn child(parent: impl AsFd, name: &str, mode: u32) -> Result<OwnedFd, PrivError> {
    let created = match mkdirat(parent.as_fd(), name, Mode::from_raw_mode(mode as _)) {
        Ok(()) => true,
        Err(Errno::EXIST) => false,
        Err(_) => return Err(refused()),
    };
    let fd = checked(
        openat(parent.as_fd(), name, DIR_FLAGS, Mode::empty()).map_err(|_| refused())?,
        false,
    )?;
    if created {
        let st = fstat(parent).map_err(|_| refused())?;
        fchown(&fd, None, Some(Gid::from_raw(st.st_gid))).map_err(|_| refused())?;
        // The helper's 0077 umask must not remove the service group's access.
        fchmod(&fd, Mode::from_raw_mode(mode as _)).map_err(|_| refused())?;
    }
    Ok(fd)
}

/// Create only beneath a trusted, non-writable or sticky helper-owned parent.
pub(super) fn ensure_dir(path: &Path, mode: u32) -> Result<OwnedFd, PrivError> {
    let parent = open(path.parent().ok_or_else(refused)?, DIR_FLAGS, Mode::empty())
        .map_err(|_| refused())?;
    let st = fstat(&parent).map_err(|_| refused())?;
    if st.st_uid != nix::unistd::geteuid().as_raw()
        || (st.st_mode & 0o022 != 0 && st.st_mode & 0o1000 == 0)
    {
        return Err(refused());
    }
    child(
        &parent,
        path.file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(refused)?,
        mode,
    )
}

/// Prepare fresh installs and migrate legacy state on helper startup after updates.
/// Only root calls this entry point; runtime operations merely validate ownership.
pub fn prepare(apps: &Path, xfer: &Path, trash: &Path) -> Result<(), PrivError> {
    if !nix::unistd::geteuid().is_root() {
        return Err(refused());
    }
    let group = nix::unistd::Group::from_name("opsbox")
        .map_err(|_| refused())?
        .ok_or_else(refused)?;
    for (path, mode, gid) in [
        (apps, 0o750, group.gid.as_raw()),
        (xfer, 0o770, group.gid.as_raw()),
        (trash, 0o700, 0),
    ] {
        prepare_at(path, mode, gid)?;
    }
    Ok(())
}

fn prepare_at(path: &Path, mode: u32, gid: u32) -> Result<(), PrivError> {
    let parent = open(path.parent().ok_or_else(refused)?, DIR_FLAGS, Mode::empty())
        .map_err(|_| refused())?;
    let st = fstat(&parent).map_err(|_| refused())?;
    if st.st_uid != nix::unistd::geteuid().as_raw()
        || (st.st_mode & 0o022 != 0 && st.st_mode & 0o1000 == 0)
    {
        return Err(refused());
    }
    // Legacy xfer is panel-owned: prevent renaming it during the migration.
    // Change only the pinned parent inode and restore its mode even on refusal.
    let original = Mode::from_raw_mode(st.st_mode as _);
    fchmod(&parent, Mode::from_raw_mode((st.st_mode & !0o022) as _)).map_err(|_| refused())?;
    let result = (|| {
        let name = path.file_name().ok_or_else(refused)?;
        match mkdirat(&parent, name, Mode::from_raw_mode(mode as _)) {
            Ok(()) | Err(Errno::EXIST) => {}
            Err(_) => return Err(refused()),
        }
        let fd = openat(&parent, name, DIR_FLAGS, Mode::empty()).map_err(|_| refused())?;
        fchown(
            &fd,
            Some(Uid::from_raw(nix::unistd::geteuid().as_raw())),
            Some(Gid::from_raw(gid)),
        )
        .map_err(|_| refused())?;
        fchmod(&fd, Mode::from_raw_mode(mode as _)).map_err(|_| refused())
    })();
    fchmod(&parent, original).map_err(|_| refused())?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    #[test]
    fn managed_prepare_sets_modes_and_refuses_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o1770)).unwrap();
        let gid = nix::unistd::getegid().as_raw();
        for (name, mode) in [("apps", 0o750), ("xfer", 0o770), ("trash", 0o700)] {
            let path = root.join(name);
            prepare_at(&path, mode, gid).unwrap();
            // An upgrade repairs legacy modes without removing data.
            std::fs::write(path.join("sentinel"), "keep").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            prepare_at(&path, mode, gid).unwrap();
            let st = std::fs::metadata(&path).unwrap();
            assert_eq!(st.uid(), nix::unistd::geteuid().as_raw());
            assert_eq!(st.gid(), gid);
            assert_eq!(st.mode() & 0o7777, mode);
            let alias = root.join(format!("link-{name}"));
            symlink(&path, &alias).unwrap();
            assert!(prepare_at(&alias, 0o777, gid).is_err());
            assert_eq!(std::fs::metadata(&path).unwrap().mode(), st.mode());
            assert_eq!(
                std::fs::read_to_string(path.join("sentinel")).unwrap(),
                "keep"
            );
            assert_eq!(std::fs::metadata(&root).unwrap().mode() & 0o7777, 0o1770);
        }
    }
}
