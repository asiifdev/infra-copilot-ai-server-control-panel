//! A read-only view of a filesystem root.
//!
//! Production uses `/`; tests use a fixture tree. There is deliberately no method that writes,
//! creates, renames or deletes anything: a collector that only holds a [`RootFs`] cannot change the
//! machine (docs/design.md 4: collectors are read-only).
//!
//! Paths passed in are absolute **relative to the root** (`/proc/net/tcp`), contain no `..`
//! component and no NUL byte; anything else is [`FsError::InvalidPath`]. Reads are bounded and
//! lossy on invalid UTF-8, so hostile or broken files cannot exhaust memory or cause a panic.
//!
//! Symlinks are resolved inside the root, chroot style: an absolute target is taken relative to
//! the root and `..` never climbs above it. With `--root /mnt/old-disk` a link such as
//! `/etc/crontab -> /etc/shadow` therefore reads the old disk's file (or is not found), never the
//! host's. More than 40 links in one path is [`FsError::SymlinkLoop`]. [`RootFs::read_link`]
//! resolves only the parent components and returns the last link's text.
//!
//! # Guarantee against concurrent changes (decision 0017)
//!
//! Resolution never works on path strings that are opened later. Every component is looked up
//! relative to a directory file descriptor that is already open: intermediate directories are
//! opened with `O_NOFOLLOW | O_DIRECTORY`, a symlink is read with `readlinkat` and expanded inside
//! the root, and the final component is opened with `O_NOFOLLOW` (and `O_NONBLOCK`, so a FIFO
//! cannot block the collector). A process that can write inside the root while it is read can
//! therefore not swap a parent directory for a symlink between the check and the open: the swap
//! makes the open fail (closed), it never makes it leave the root. On Linux the kernel does the
//! whole walk atomically with `openat2(RESOLVE_IN_ROOT | RESOLVE_NO_MAGICLINKS)`; the per-component
//! walk is used where `openat2` is missing (kernel before 5.6, macOS and other Unix) or blocked
//! (`ENOSYS`, `EPERM`, for example by an old container seccomp profile).
//!
//! Remaining limits, stated plainly:
//! - The root itself is opened by path (`base`); if that path is replaced while the process runs,
//!   the new tree is used from then on. Everything inside is safe, the root's own name is trusted.
//! - Mount points inside the root are crossed (`RESOLVE_NO_XDEV` is not set), so `/proc` and bind
//!   mounts work; a mount that someone else controls inside the root is followed.
//! - [`RootFs::read_link`] reads a link's text; what the text names is not validated.
//! - The `openat2` branch (Linux only) is compiled and tested in CI on Linux; it was written from
//!   the documentation and could not be run on the macOS development machine. The walk is tested
//!   everywhere, and on Linux a contract test compares both branches on the same tree.

#[cfg(unix)]
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::File;
use std::io::{self, Read};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

#[cfg(unix)]
use rustix::fd::OwnedFd;
#[cfg(unix)]
use rustix::fs::{self, AtFlags, CWD, Dir, FileType, Mode, OFlags};
#[cfg(unix)]
use rustix::io::Errno;

/// Most symbolic links followed while resolving one path (Linux uses the same limit).
#[cfg(unix)]
const MAX_SYMLINKS: usize = 40;

/// Open flags for a file read: never blocks on a FIFO, never becomes a controlling terminal,
/// never leaks into children.
#[cfg(unix)]
const FILE_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::NONBLOCK)
    .union(OFlags::NOCTTY)
    .union(OFlags::CLOEXEC);
/// Open flags for a directory.
#[cfg(unix)]
const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::CLOEXEC);

/// Why a read failed. Callers turn this into a warning, never into a silent "nothing there".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsError {
    /// The path is not absolute, contains `..` or a NUL byte.
    InvalidPath(String),
    /// Nothing exists at the path.
    NotFound,
    /// The OS refused access.
    PermissionDenied,
    /// A directory where a file was expected, or the reverse (also FIFOs, sockets, devices).
    WrongType,
    /// Too many symbolic links in one path (a loop, or an absurdly deep chain).
    SymlinkLoop,
    /// Any other I/O failure (e.g. a symlink loop), with the OS message.
    Other(String),
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(path) => write!(f, "invalid path {path:?}"),
            Self::NotFound => f.write_str("not found"),
            Self::PermissionDenied => f.write_str("permission denied"),
            Self::WrongType => f.write_str("unexpected file type"),
            Self::SymlinkLoop => f.write_str("too many levels of symbolic links"),
            Self::Other(msg) => write!(f, "I/O error: {msg}"),
        }
    }
}

impl std::error::Error for FsError {}

impl From<&io::Error> for FsError {
    fn from(err: &io::Error) -> Self {
        match err.kind() {
            io::ErrorKind::NotFound => Self::NotFound,
            io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            io::ErrorKind::IsADirectory | io::ErrorKind::NotADirectory => Self::WrongType,
            _ => Self::Other(err.to_string()),
        }
    }
}

#[cfg(unix)]
impl From<Errno> for FsError {
    fn from(errno: Errno) -> Self {
        if errno == Errno::LOOP {
            Self::SymlinkLoop
        } else {
            Self::from(&io::Error::from(errno))
        }
    }
}

/// Text read from a file, at most the requested number of bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileText {
    /// The content (invalid UTF-8 replaced by U+FFFD).
    pub text: String,
    /// The file was longer than the limit and the rest was not read.
    pub truncated: bool,
}

/// Directory entry names, sorted, at most the requested number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirNames {
    /// Entry names (not paths).
    pub names: Vec<String>,
    /// The directory had more entries than the limit.
    pub truncated: bool,
    /// Entries skipped because their name is not valid UTF-8 (they cannot be addressed by the
    /// `&str` paths of this API); callers should warn about them.
    pub non_utf8: usize,
}

/// A read-only filesystem root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootFs {
    base: PathBuf,
}

impl RootFs {
    /// A view rooted at `base` (a fixture directory in tests).
    #[must_use]
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    /// The real machine: root `/`.
    #[must_use]
    pub fn host() -> Self {
        Self::new("/")
    }

    /// Checks the root itself is an accessible directory, so a mistyped fixture path is an error
    /// rather than an inventory full of warnings.
    ///
    /// # Errors
    /// [`FsError`] when the root is missing, not a directory, or unreadable.
    #[cfg(unix)]
    pub fn validate(&self) -> Result<(), FsError> {
        let stat = fs::statat(CWD, &self.base, AtFlags::empty())?;
        if FileType::from_raw_mode(stat.st_mode) == FileType::Directory {
            Ok(())
        } else {
            Err(FsError::WrongType)
        }
    }

    /// Checks the root itself is an accessible directory (portable fallback for non-Unix).
    ///
    /// # Errors
    /// [`FsError`] when the root is missing, not a directory, or unreadable.
    #[cfg(not(unix))]
    pub fn validate(&self) -> Result<(), FsError> {
        let meta = std::fs::metadata(&self.base).map_err(|e| FsError::from(&e))?;
        if meta.is_dir() {
            Ok(())
        } else {
            Err(FsError::WrongType)
        }
    }

    /// Opens the root directory (a new descriptor per call, so `RootFs` stays plain data).
    #[cfg(unix)]
    fn open_root(&self) -> Result<OwnedFd, FsError> {
        Ok(fs::openat(CWD, &self.base, DIR_FLAGS, Mode::empty())?)
    }

    /// Opens `rel` inside the root. With `follow == false` the final component must not be a
    /// symbolic link (the open fails with [`FsError::WrongType`]).
    #[cfg(unix)]
    fn open_in_root(&self, rel: &str, follow: bool, flags: OFlags) -> Result<OwnedFd, FsError> {
        let names = split_path(rel)?;
        let root = self.open_root()?;
        #[cfg(target_os = "linux")]
        if let Some(result) = open_with_openat2(&root, &names, follow, flags) {
            return result;
        }
        open_with_walk(root, &names, follow, flags)
    }

    /// Resolves `rel` safely inside `self.base` on non-Unix platforms, refusing any symlink or
    /// reparse point along the path so nothing can escape `self.base`.
    #[cfg(not(unix))]
    fn resolve_portable(&self, rel: &str) -> Result<PathBuf, FsError> {
        let names = split_path(rel)?;
        self.validate()?;
        let mut cur = self.base.clone();
        for name in names {
            cur.push(name);
            let meta = std::fs::symlink_metadata(&cur).map_err(|e| FsError::from(&e))?;
            if meta.file_type().is_symlink() {
                return Err(FsError::WrongType);
            }
        }
        Ok(cur)
    }

    /// Reads a regular file (following symlinks inside the root) as text, reading at most
    /// `max_bytes`.
    ///
    /// # Errors
    /// [`FsError`]; non-regular files (directories, FIFOs, devices) are refused so a planted FIFO
    /// cannot block the collector.
    #[cfg(unix)]
    pub fn read_to_string(&self, rel: &str, max_bytes: usize) -> Result<FileText, FsError> {
        read_file(self.open_in_root(rel, true, FILE_FLAGS)?, max_bytes)
    }

    /// Reads a regular file inside the root on non-Unix platforms, refusing symlinks/reparse
    /// points and reading at most `max_bytes`.
    ///
    /// # Errors
    /// [`FsError`].
    #[cfg(not(unix))]
    pub fn read_to_string(&self, rel: &str, max_bytes: usize) -> Result<FileText, FsError> {
        self.read_to_string_nolink(rel, max_bytes)
    }

    /// Like [`RootFs::read_to_string`], but the **last** path component must not be a symbolic
    /// link: it is refused with [`FsError::WrongType`]. The final open uses `O_NOFOLLOW`, so the
    /// link cannot be swapped in between a check and the open; there is no separate check. Parent
    /// components are resolved inside the root as usual.
    ///
    /// # Errors
    /// [`FsError`]; see [`RootFs::read_to_string`].
    #[cfg(unix)]
    pub fn read_to_string_nolink(&self, rel: &str, max_bytes: usize) -> Result<FileText, FsError> {
        read_file(self.open_in_root(rel, false, FILE_FLAGS)?, max_bytes)
    }

    /// Reads a regular file inside the root on non-Unix platforms, refusing symlinks/reparse
    /// points and reading at most `max_bytes`.
    ///
    /// # Errors
    /// [`FsError`].
    #[cfg(not(unix))]
    pub fn read_to_string_nolink(&self, rel: &str, max_bytes: usize) -> Result<FileText, FsError> {
        let path = self.resolve_portable(rel)?;
        let meta = std::fs::symlink_metadata(&path).map_err(|e| FsError::from(&e))?;
        if !meta.is_file() {
            return Err(FsError::WrongType);
        }
        let file = File::open(&path).map_err(|e| FsError::from(&e))?;
        read_bounded(file, max_bytes)
    }

    /// Lists a directory: entry names sorted, at most `max_entries`.
    ///
    /// # Errors
    /// [`FsError`]. Entries that vanish while listing are skipped; names that are not valid UTF-8
    /// are counted in [`DirNames::non_utf8`].
    #[cfg(unix)]
    pub fn read_dir(&self, rel: &str, max_entries: usize) -> Result<DirNames, FsError> {
        let fd = self.open_in_root(rel, true, DIR_FLAGS)?;
        let dir = Dir::read_from(&fd)?;
        let mut names = Vec::new();
        let mut truncated = false;
        let mut non_utf8 = 0usize;
        for entry in dir.flatten() {
            let raw = entry.file_name().to_bytes();
            if raw == b"." || raw == b".." {
                continue;
            }
            if names.len() >= max_entries {
                truncated = true;
                break;
            }
            match String::from_utf8(raw.to_vec()) {
                Ok(name) => names.push(name),
                Err(_) => non_utf8 += 1,
            }
        }
        names.sort();
        Ok(DirNames {
            names,
            truncated,
            non_utf8,
        })
    }

    /// Lists a directory inside the root on non-Unix platforms.
    ///
    /// # Errors
    /// [`FsError`].
    #[cfg(not(unix))]
    pub fn read_dir(&self, rel: &str, max_entries: usize) -> Result<DirNames, FsError> {
        let path = self.resolve_portable(rel)?;
        let rd = std::fs::read_dir(&path).map_err(|e| FsError::from(&e))?;
        let mut names = Vec::new();
        let mut truncated = false;
        let mut non_utf8 = 0usize;
        for entry in rd.flatten() {
            if names.len() >= max_entries {
                truncated = true;
                break;
            }
            match entry.file_name().into_string() {
                Ok(name) => names.push(name),
                Err(_) => non_utf8 += 1,
            }
        }
        names.sort();
        Ok(DirNames {
            names,
            truncated,
            non_utf8,
        })
    }

    /// Reads the target of a symlink without following it. Only the parent components are
    /// resolved (inside the root); the last component's link text is returned as stored.
    ///
    /// # Errors
    /// [`FsError`].
    #[cfg(unix)]
    pub fn read_link(&self, rel: &str) -> Result<String, FsError> {
        let names = split_path(rel)?;
        let walked = walk(self.open_root()?, &names, false)?;
        let name = walked.last.as_deref().unwrap_or(OsStr::new("."));
        let target = fs::readlinkat(&walked.dir, name, Vec::new())?;
        Ok(String::from_utf8_lossy(target.to_bytes()).into_owned())
    }

    /// Reads a symlink target on non-Unix platforms (if supported by the filesystem).
    ///
    /// # Errors
    /// [`FsError`].
    #[cfg(not(unix))]
    pub fn read_link(&self, rel: &str) -> Result<String, FsError> {
        let _ = split_path(rel)?;
        Err(FsError::WrongType)
    }
}

/// Validates `rel` and returns its components (`.` and empty parts dropped).
fn split_path(rel: &str) -> Result<Vec<&str>, FsError> {
    if !rel.starts_with('/') || rel.contains('\0') {
        return Err(FsError::InvalidPath(rel.to_owned()));
    }
    let mut names = Vec::new();
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(FsError::InvalidPath(rel.to_owned())),
            part => names.push(part),
        }
    }
    Ok(names)
}

/// The final-component error: with `follow == false` a symlink (`ELOOP` from `O_NOFOLLOW`) is the
/// wrong type; otherwise it is a loop.
#[cfg(unix)]
fn final_error(errno: Errno, follow: bool) -> FsError {
    if errno == Errno::LOOP && !follow {
        FsError::WrongType
    } else {
        FsError::from(errno)
    }
}

/// Linux fast path: the kernel resolves the whole path atomically inside the root. `None` means
/// "use the walk" (the syscall is missing or blocked, or the answer needs the walk's precision).
#[cfg(target_os = "linux")]
fn open_with_openat2(
    root: &OwnedFd,
    names: &[&str],
    follow: bool,
    flags: OFlags,
) -> Option<Result<OwnedFd, FsError>> {
    use rustix::fs::ResolveFlags;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Set once `openat2` answered `ENOSYS` or `EPERM`, so later calls skip it.
    static UNAVAILABLE: AtomicBool = AtomicBool::new(false);

    if UNAVAILABLE.load(Ordering::Relaxed) {
        return None;
    }
    let rel = if names.is_empty() {
        ".".to_owned()
    } else {
        names.join("/")
    };
    // A file read never opens its target directly: the entry is first opened with `O_PATH`
    // (which has no side effects, even on a device node), its type is read from that descriptor,
    // and only a regular file is reopened for reading, through the very same descriptor
    // (`/proc/self/fd/N`), so it cannot be swapped for something else in between.
    let file_read = !flags.contains(OFlags::DIRECTORY) && flags.contains(OFlags::NONBLOCK);
    let mut oflags = if file_read {
        OFlags::PATH | OFlags::CLOEXEC
    } else {
        flags | OFlags::CLOEXEC
    };
    if !follow {
        oflags |= OFlags::NOFOLLOW;
    }
    let resolve = ResolveFlags::IN_ROOT | ResolveFlags::NO_MAGICLINKS;
    // `EAGAIN` means the kernel saw a rename or mount race during the walk; a few retries, then
    // the walk decides.
    for _ in 0..4 {
        match fs::openat2(root, rel.as_str(), oflags, Mode::empty(), resolve) {
            Ok(fd) if file_read => return reopen_regular(&fd, flags | OFlags::CLOEXEC),
            Ok(fd) => return Some(Ok(fd)),
            Err(errno) if errno == Errno::NOSYS || errno == Errno::PERM => {
                UNAVAILABLE.store(true, Ordering::Relaxed);
                return None;
            }
            Err(errno) if errno == Errno::AGAIN => {}
            // `ELOOP` with `O_NOFOLLOW` is either the final link or a loop in a parent; the walk
            // tells the two apart.
            Err(errno) if errno == Errno::LOOP && !follow => return None,
            Err(errno) => return Some(Err(final_error(errno, follow))),
        }
    }
    None
}

/// Turns an `O_PATH` descriptor into a readable one when it is a regular file. `None` means
/// "use the walk" (no usable `/proc`, so the descriptor cannot be reopened).
#[cfg(target_os = "linux")]
fn reopen_regular(path_fd: &OwnedFd, flags: OFlags) -> Option<Result<OwnedFd, FsError>> {
    use std::os::fd::AsRawFd;

    let stat = match fs::fstat(path_fd) {
        Ok(stat) => stat,
        Err(errno) => return Some(Err(FsError::from(errno))),
    };
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Some(Err(FsError::WrongType)); // a device, FIFO, socket, directory or link
    }
    let proc_path = format!("/proc/self/fd/{}", path_fd.as_raw_fd());
    fs::openat(CWD, proc_path.as_str(), flags, Mode::empty())
        .ok()
        .map(Ok)
}

/// The per-component fallback (and the only path off Linux).
#[cfg(unix)]
fn open_with_walk(
    root: OwnedFd,
    names: &[&str],
    follow: bool,
    flags: OFlags,
) -> Result<OwnedFd, FsError> {
    let walked = walk(root, names, follow)?;
    // Everything on the way is resolved, so `O_NOFOLLOW` is always right for the last open: a link
    // swapped in since the lookup makes it fail instead of leaving the root.
    let name = walked.last.as_deref().unwrap_or(OsStr::new("."));
    // Opening a device node can have side effects, so a file read opens nothing but a regular file
    // (a link is left to `O_NOFOLLOW`, which reports it). The type is checked again on the opened
    // descriptor in `read_file`.
    if !flags.contains(OFlags::DIRECTORY) && walked.last.is_some() {
        let stat = fs::statat(&walked.dir, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|errno| final_error(errno, follow))?;
        if !matches!(
            FileType::from_raw_mode(stat.st_mode),
            FileType::RegularFile | FileType::Symlink
        ) && flags.contains(OFlags::NONBLOCK)
        {
            return Err(FsError::WrongType);
        }
    }
    fs::openat(
        &walked.dir,
        name,
        flags | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|errno| final_error(errno, follow))
}

/// Where a walk ended: `dir` is open, and `last` (if any) is an entry of it, not yet opened.
#[cfg(unix)]
struct Walked {
    dir: OwnedFd,
    last: Option<OsString>,
}

/// One pending path component.
#[cfg(unix)]
enum Step {
    Name(OsString),
    Parent,
}

/// Walks `names` from `root`, one directory descriptor at a time. Symlinks are read with
/// `readlinkat` and expanded in place (absolute targets restart at the root, `..` pops a
/// descriptor and never goes below the root). The last component is returned unopened (and, with
/// `follow == false`, unexamined).
#[cfg(unix)]
fn walk(root: OwnedFd, names: &[&str], follow: bool) -> Result<Walked, FsError> {
    let mut current = root;
    let mut parents: Vec<OwnedFd> = Vec::new();
    let mut pending: Vec<Step> = names
        .iter()
        .rev()
        .map(|name| Step::Name(OsString::from(name)))
        .collect();
    let mut links = 0usize;
    while let Some(step) = pending.pop() {
        let name = match step {
            Step::Parent => {
                if let Some(parent) = parents.pop() {
                    current = parent; // at the root, `..` stays at the root
                }
                continue;
            }
            Step::Name(name) => name,
        };
        let is_last = pending.is_empty();
        if is_last && !follow {
            return Ok(Walked {
                dir: current,
                last: Some(name),
            });
        }
        let stat = fs::statat(&current, name.as_os_str(), AtFlags::SYMLINK_NOFOLLOW)?;
        match FileType::from_raw_mode(stat.st_mode) {
            FileType::Symlink => {
                links += 1;
                if links > MAX_SYMLINKS {
                    return Err(FsError::SymlinkLoop);
                }
                let target = fs::readlinkat(&current, name.as_os_str(), Vec::new())?;
                let bytes = target.to_bytes();
                if bytes.first() == Some(&b'/') {
                    // Absolute targets are relative to the root, not the host.
                    if let Some(root) = parents.drain(..).next() {
                        current = root;
                    }
                }
                for part in bytes.rsplit(|byte| *byte == b'/') {
                    match part {
                        b"" | b"." => {}
                        b".." => pending.push(Step::Parent),
                        part => pending.push(Step::Name(OsStr::from_bytes(part).to_owned())),
                    }
                }
            }
            _ if is_last => {
                return Ok(Walked {
                    dir: current,
                    last: Some(name),
                });
            }
            FileType::Directory => {
                let next = fs::openat(
                    &current,
                    name.as_os_str(),
                    DIR_FLAGS | OFlags::NOFOLLOW,
                    Mode::empty(),
                )?;
                parents.push(std::mem::replace(&mut current, next));
            }
            _ => return Err(FsError::WrongType),
        }
    }
    Ok(Walked {
        dir: current,
        last: None,
    })
}

/// Reads at most `max_bytes` from `file`.
fn read_bounded(file: File, max_bytes: usize) -> Result<FileText, FsError> {
    let limit = u64::try_from(max_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut bytes = Vec::new();
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|err| FsError::from(&err))?;
    let truncated = bytes.len() > max_bytes;
    bytes.truncate(max_bytes);
    Ok(FileText {
        text: String::from_utf8_lossy(&bytes).into_owned(),
        truncated,
    })
}

/// Reads an opened regular file; the type is checked on the descriptor itself, so a file swapped
/// for something else after the open cannot matter.
#[cfg(unix)]
fn read_file(fd: OwnedFd, max_bytes: usize) -> Result<FileText, FsError> {
    let stat = fs::fstat(&fd)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(FsError::WrongType);
    }
    read_bounded(File::from(fd), max_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_relative_and_traversal_paths() {
        let root = RootFs::new("/nonexistent-root");
        for bad in ["etc/passwd", "/etc/../etc/passwd", "/..", "/a\0b", ""] {
            assert!(
                matches!(root.read_to_string(bad, 10), Err(FsError::InvalidPath(_))),
                "{bad:?}"
            );
            assert!(matches!(
                root.read_dir(bad, 10),
                Err(FsError::InvalidPath(_))
            ));
            assert!(matches!(root.read_link(bad), Err(FsError::InvalidPath(_))));
        }
    }

    #[test]
    fn missing_root_is_an_error_not_a_panic() {
        assert_eq!(
            RootFs::new("/nonexistent-root").validate(),
            Err(FsError::NotFound)
        );
    }

    /// Builds a small tree with links of every kind under a unique temporary directory.
    #[cfg(unix)]
    fn tree(name: &str) -> Result<PathBuf, io::Error> {
        let top = std::env::temp_dir().join(format!("rootfs-unit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&top);
        let root = top.join("root");
        std::fs::create_dir_all(root.join("a/b"))?;
        std::fs::write(root.join("a/b/file"), "hello")?;
        let link = std::os::unix::fs::symlink;
        link("b/file", root.join("a/rel"))?;
        link("/a/b", root.join("abs"))?;
        link("../../a/b/file", root.join("a/b/climb"))?;
        link("../../../../x/../a/b/file", root.join("a/b/deep"))?;
        link("loop2", root.join("loop1"))?;
        link("loop1", root.join("loop2"))?;
        link("missing", root.join("dangling"))?;
        Ok(root)
    }

    /// Both resolution strategies (the walk, and `openat2` on Linux) and the public API must give
    /// the same answer on the same tree, for every kind of path.
    #[cfg(unix)]
    #[test]
    fn walk_openat2_and_public_api_agree() -> Result<(), io::Error> {
        let root = tree("agree")?;
        let fs = RootFs::new(&root);
        let paths = [
            "/a/b/file",
            "/a/rel",
            "/abs/file",
            "/a/b/climb",
            "/a/b/deep",
            "/loop1",
            "/dangling",
            "/a/b",
            "/a/none",
            "/",
        ];
        for path in paths {
            for follow in [true, false] {
                let names = split_path(path).map_err(|e| io::Error::other(e.to_string()))?;
                let walk_result =
                    open_with_walk(fs.open_root().map_err(io_of)?, &names, follow, FILE_FLAGS)
                        .and_then(|fd| read_file(fd, 64));
                let public = if follow {
                    fs.read_to_string(path, 64)
                } else {
                    fs.read_to_string_nolink(path, 64)
                };
                assert_eq!(walk_result, public, "{path} follow={follow}");
                #[cfg(target_os = "linux")]
                if let Some(kernel) =
                    open_with_openat2(&fs.open_root().map_err(io_of)?, &names, follow, FILE_FLAGS)
                {
                    assert_eq!(
                        kernel.and_then(|fd| read_file(fd, 64)),
                        public,
                        "{path} openat2"
                    );
                }
            }
        }
        let _ = std::fs::remove_dir_all(root.parent().unwrap_or(&root));
        Ok(())
    }

    #[cfg(unix)]
    fn io_of(err: FsError) -> io::Error {
        io::Error::other(err.to_string())
    }
}
