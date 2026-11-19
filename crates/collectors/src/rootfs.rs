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
//! Symlinks are resolved inside the root, chroot style, one component at a time: an absolute
//! target is taken relative to the root and `..` never climbs above it. With `--root /mnt/old-disk`
//! a link such as `/etc/crontab -> /etc/shadow` therefore reads the old disk's file (or is not
//! found), never the host's. More than 40 links in one path is [`FsError::SymlinkLoop`].
//! [`RootFs::read_link`] resolves only the parent components and returns the last link's text.
//!
//! Limit (accepted, see decision 0013): a path is resolved and then opened, so another process that
//! can **write inside the root while it is being read** could swap a component for a symlink in
//! between (time-of-check to time-of-use) and make the open follow it out of the root. Closing that
//! gap needs `openat2(RESOLVE_IN_ROOT)` or a per-component `openat` walk, which requires `unsafe`
//! or a new dependency; both are outside the current rules. Collect from a root nobody can modify
//! concurrently (the live `/` is not writable by unprivileged users where it matters, `/proc` is
//! kernel-managed, and a mounted old disk or a snapshot is quiescent).

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, PathBuf};

/// Most symbolic links followed while resolving one path (Linux uses the same limit).
const MAX_SYMLINKS: usize = 40;

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
    pub fn validate(&self) -> Result<(), FsError> {
        let meta = fs::metadata(&self.base).map_err(|err| FsError::from(&err))?;
        if meta.is_dir() {
            Ok(())
        } else {
            Err(FsError::WrongType)
        }
    }

    /// Resolves `rel` inside the root: every symlink is expanded in chroot style. With
    /// `follow_last == false` the final component is left alone (for `read_link`).
    fn resolve(&self, rel: &str, follow_last: bool) -> Result<PathBuf, FsError> {
        if !rel.starts_with('/') || rel.contains('\0') {
            return Err(FsError::InvalidPath(rel.to_owned()));
        }
        let mut pending: Vec<Step> = Vec::new();
        for part in rel.split('/').rev() {
            match part {
                "" | "." => {}
                ".." => return Err(FsError::InvalidPath(rel.to_owned())),
                part => pending.push(Step::Name(OsString::from(part))),
            }
        }
        let mut resolved: Vec<OsString> = Vec::new();
        let mut links = 0usize;
        while let Some(step) = pending.pop() {
            let name = match step {
                Step::Parent => {
                    resolved.pop(); // at the root, `..` stays at the root
                    continue;
                }
                Step::Name(name) => name,
            };
            let mut candidate = self.base.clone();
            candidate.extend(&resolved);
            candidate.push(&name);
            if pending.is_empty() && !follow_last {
                return Ok(candidate);
            }
            let meta = fs::symlink_metadata(&candidate).map_err(|err| FsError::from(&err))?;
            if !meta.file_type().is_symlink() {
                resolved.push(name);
                continue;
            }
            links += 1;
            if links > MAX_SYMLINKS {
                return Err(FsError::SymlinkLoop);
            }
            let target = fs::read_link(&candidate).map_err(|err| FsError::from(&err))?;
            if target.has_root() {
                resolved.clear(); // absolute targets are relative to the root, not the host
            }
            let steps: Vec<Step> = target
                .components()
                .filter_map(|component| match component {
                    Component::Normal(name) => Some(Step::Name(name.to_owned())),
                    Component::ParentDir => Some(Step::Parent),
                    Component::RootDir | Component::CurDir | Component::Prefix(_) => None,
                })
                .collect();
            pending.extend(steps.into_iter().rev());
        }
        let mut path = self.base.clone();
        path.extend(&resolved);
        Ok(path)
    }

    /// Reads a regular file (following symlinks inside the root) as text, reading at most
    /// `max_bytes`.
    ///
    /// # Errors
    /// [`FsError`]; non-regular files (directories, FIFOs, devices) are refused so a planted FIFO
    /// cannot block the collector.
    pub fn read_to_string(&self, rel: &str, max_bytes: usize) -> Result<FileText, FsError> {
        let path = self.resolve(rel, true)?;
        // Cheap early refusal; the authoritative check is on the opened file below, so a file
        // swapped for a FIFO in between cannot block the open (it is opened non-blocking where the
        // flag is known).
        let meta = fs::metadata(&path).map_err(|err| FsError::from(&err))?;
        if !meta.is_file() {
            return Err(FsError::WrongType);
        }
        let file = open_nonblocking(&path).map_err(|err| FsError::from(&err))?;
        let opened = file.metadata().map_err(|err| FsError::from(&err))?;
        if !opened.is_file() {
            return Err(FsError::WrongType);
        }
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

    /// Lists a directory: entry names sorted, at most `max_entries`.
    ///
    /// # Errors
    /// [`FsError`]. Entries that vanish while listing are skipped; names that are not valid UTF-8
    /// are counted in [`DirNames::non_utf8`].
    pub fn read_dir(&self, rel: &str, max_entries: usize) -> Result<DirNames, FsError> {
        let path = self.resolve(rel, true)?;
        let entries = fs::read_dir(&path).map_err(|err| FsError::from(&err))?;
        let mut names = Vec::new();
        let mut truncated = false;
        let mut non_utf8 = 0usize;
        for entry in entries.flatten() {
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
    pub fn read_link(&self, rel: &str) -> Result<String, FsError> {
        let path = self.resolve(rel, false)?;
        let target = fs::read_link(&path).map_err(|err| FsError::from(&err))?;
        Ok(target.to_string_lossy().into_owned())
    }
}

/// One pending path component during resolution.
enum Step {
    Name(OsString),
    Parent,
}

/// `O_NONBLOCK` for the platforms where its value is known without a C library binding; elsewhere
/// the metadata checks above are the only guard against a FIFO.
#[cfg(unix)]
const O_NONBLOCK: Option<i32> = if cfg!(any(
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
)) {
    Some(0x0004)
} else if cfg!(all(
    any(target_os = "linux", target_os = "android"),
    any(
        target_arch = "x86",
        target_arch = "x86_64",
        target_arch = "arm",
        target_arch = "aarch64",
        target_arch = "riscv32",
        target_arch = "riscv64",
        target_arch = "powerpc",
        target_arch = "powerpc64",
        target_arch = "s390x",
        target_arch = "loongarch64"
    )
)) {
    Some(0o4000)
} else {
    None
};

#[cfg(unix)]
fn open_nonblocking(path: &std::path::Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    if let Some(flag) = O_NONBLOCK {
        options.custom_flags(flag);
    }
    options.open(path)
}

#[cfg(not(unix))]
fn open_nonblocking(path: &std::path::Path) -> io::Result<fs::File> {
    fs::File::open(path)
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
}
