//! `RootFs` against hostile trees: symlink chains and loops, `..` clamping, FIFOs, permissions and
//! a parent directory swapped for a symlink while it is being read (decision 0017).

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use collectors::{FsError, RootFs};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A scratch directory under the cargo target directory: `<tmp>/<name>/root` is the root under
/// test, `<tmp>/<name>/outside` holds files the root must never reveal.
struct Sandbox {
    top: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Result<Self, std::io::Error> {
        let top = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("rootfs-{name}"));
        let _ = fs::remove_dir_all(&top);
        fs::create_dir_all(top.join("root"))?;
        fs::create_dir_all(top.join("outside"))?;
        fs::write(top.join("outside/secret"), "OUTSIDE")?;
        Ok(Self { top })
    }

    fn root_path(&self) -> PathBuf {
        self.top.join("root")
    }

    fn outside(&self) -> PathBuf {
        self.top.join("outside")
    }

    fn fs(&self) -> RootFs {
        RootFs::new(self.root_path())
    }

    fn write(&self, rel: &str, text: &str) -> Result<(), std::io::Error> {
        let path = self.root_path().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, text)
    }

    fn link(&self, target: impl AsRef<Path>, rel: &str) -> Result<(), std::io::Error> {
        let path = self.root_path().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        symlink(target, path)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // A test may leave a directory unreadable on purpose.
        let _ = fs::set_permissions(
            self.root_path().join("locked"),
            fs::Permissions::from_mode(0o755),
        );
        let _ = fs::remove_dir_all(&self.top);
    }
}

fn text(root: &RootFs, rel: &str) -> Result<String, FsError> {
    root.read_to_string(rel, 1024).map(|file| file.text)
}

#[test]
fn deep_symlink_chain_within_the_limit_resolves() -> TestResult {
    let sb = Sandbox::new("chain")?;
    sb.write("data/file", "end")?;
    sb.link("data/file", "l0")?;
    for i in 1..39 {
        sb.link(format!("l{}", i - 1), &format!("l{i}"))?;
    }
    // 39 links to follow: under the limit of 40.
    assert_eq!(text(&sb.fs(), "/l38")?, "end");
    Ok(())
}

#[test]
fn symlink_chain_over_the_limit_is_a_loop_error() -> TestResult {
    let sb = Sandbox::new("toodeep")?;
    sb.write("file", "end")?;
    sb.link("file", "l0")?;
    for i in 1..45 {
        sb.link(format!("l{}", i - 1), &format!("l{i}"))?;
    }
    assert_eq!(text(&sb.fs(), "/l44"), Err(FsError::SymlinkLoop));
    Ok(())
}

#[test]
fn symlink_loops_are_errors() -> TestResult {
    let sb = Sandbox::new("loop")?;
    sb.link("b", "a")?;
    sb.link("a", "b")?;
    sb.link("self", "self")?;
    for path in ["/a", "/b", "/self", "/a/x"] {
        assert_eq!(text(&sb.fs(), path), Err(FsError::SymlinkLoop), "{path}");
    }
    assert_eq!(sb.fs().read_dir("/a", 10), Err(FsError::SymlinkLoop));
    Ok(())
}

#[test]
fn absolute_link_targets_are_taken_inside_the_root() -> TestResult {
    let sb = Sandbox::new("abs")?;
    sb.write("real/etc/passwd", "inside-passwd")?;
    sb.link("/real/etc", "etc")?; // absolute: means <root>/real/etc
    sb.link("/etc/passwd", "shortcut")?; // via the link above, still inside the root
    assert_eq!(text(&sb.fs(), "/etc/passwd")?, "inside-passwd");
    assert_eq!(text(&sb.fs(), "/shortcut")?, "inside-passwd");

    // An absolute link to a host path that exists on the host but not in the root: not found.
    let host_file = sb.outside().join("secret");
    sb.link(&host_file, "escape")?;
    assert_eq!(text(&sb.fs(), "/escape"), Err(FsError::NotFound));
    Ok(())
}

#[test]
fn relative_dotdot_in_links_never_leaves_the_root() -> TestResult {
    let sb = Sandbox::new("dotdot")?;
    sb.write("a/b/inside", "inside")?;
    // From /a/b, `../../../../..` is clamped at the root, then `a/b/inside` is found.
    sb.link("../../../../../a/b/inside", "a/b/climb")?;
    assert_eq!(text(&sb.fs(), "/a/b/climb")?, "inside");

    // A link that tries to reach the sibling directory of the root by climbing out.
    sb.link("../../../outside/secret", "a/b/out")?;
    assert_eq!(text(&sb.fs(), "/a/b/out"), Err(FsError::NotFound));
    // `..` as the very last step of a link resolves to a directory.
    sb.link("..", "a/b/up")?;
    assert_eq!(
        sb.fs().read_dir("/a/b/up/b", 10)?.names,
        ["climb", "inside", "out", "up"]
    );
    Ok(())
}

#[test]
fn read_link_and_nolink_keep_their_semantics() -> TestResult {
    let sb = Sandbox::new("nolink")?;
    sb.write("real/file", "x")?;
    sb.link("real", "dir")?;
    sb.link("real/file", "alias")?;
    sb.link("missing", "dangling")?;
    let root = sb.fs();
    // The last component is a link: refused, whether it dangles or not.
    assert_eq!(
        root.read_to_string_nolink("/alias", 10),
        Err(FsError::WrongType)
    );
    assert_eq!(
        root.read_to_string_nolink("/dangling", 10),
        Err(FsError::WrongType)
    );
    // A link in a parent is fine and stays inside the root.
    assert_eq!(root.read_to_string_nolink("/dir/file", 10)?.text, "x");
    assert_eq!(root.read_link("/alias")?, "real/file");
    assert_eq!(
        root.read_link("/dir/../alias").err(),
        Some(FsError::InvalidPath("/dir/../alias".into()))
    );
    assert_eq!(
        root.read_link("/real/file")
            .err()
            .map(|e| matches!(e, FsError::Other(_))),
        Some(true)
    );
    Ok(())
}

#[test]
fn nolink_reports_a_loop_in_a_parent_as_a_loop() -> TestResult {
    let sb = Sandbox::new("nolink-loop")?;
    sb.link("b", "a")?;
    sb.link("a", "b")?;
    assert_eq!(
        sb.fs().read_to_string_nolink("/a/file", 10),
        Err(FsError::SymlinkLoop)
    );
    Ok(())
}

#[test]
fn fifo_is_refused_without_blocking() -> TestResult {
    let sb = Sandbox::new("fifo")?;
    let fifo = sb.root_path().join("pipe");
    assert!(Command::new("mkfifo").arg(&fifo).status()?.success());
    sb.link("pipe", "via-link")?;
    let root = sb.fs();
    // No reader or writer ever shows up: a blocking open would hang this test forever.
    assert_eq!(root.read_to_string("/pipe", 10), Err(FsError::WrongType));
    assert_eq!(
        root.read_to_string("/via-link", 10),
        Err(FsError::WrongType)
    );
    assert_eq!(
        root.read_to_string_nolink("/pipe", 10),
        Err(FsError::WrongType)
    );
    Ok(())
}

#[test]
fn directories_are_not_files_and_files_are_not_directories() -> TestResult {
    let sb = Sandbox::new("types")?;
    sb.write("d/f", "x")?;
    let root = sb.fs();
    assert_eq!(root.read_to_string("/d", 10), Err(FsError::WrongType));
    assert_eq!(root.read_dir("/d/f", 10), Err(FsError::WrongType));
    assert_eq!(
        root.read_to_string("/d/f/deeper", 10),
        Err(FsError::WrongType)
    );
    assert_eq!(root.read_to_string("/d/none", 10), Err(FsError::NotFound));
    assert!(root.read_dir("/", 10)?.names.contains(&"d".to_owned()));
    Ok(())
}

#[test]
fn permission_denied_is_reported() -> TestResult {
    let sb = Sandbox::new("perm")?;
    sb.write("locked/file", "secret")?;
    fs::set_permissions(
        sb.root_path().join("locked"),
        fs::Permissions::from_mode(0o000),
    )?;
    if fs::read_dir(sb.root_path().join("locked")).is_ok() {
        return Ok(()); // running as root: permissions do not apply
    }
    let root = sb.fs();
    assert_eq!(
        root.read_to_string("/locked/file", 10),
        Err(FsError::PermissionDenied)
    );
    assert_eq!(root.read_dir("/locked", 10), Err(FsError::PermissionDenied));
    Ok(())
}

#[test]
fn size_cap_and_lossy_utf8_still_hold() -> TestResult {
    let sb = Sandbox::new("caps")?;
    sb.write("big", &"x".repeat(100))?;
    fs::write(sb.root_path().join("bad"), [b'a', 0xff, b'b'])?;
    let root = sb.fs();
    let big = root.read_to_string("/big", 10)?;
    assert_eq!((big.text.len(), big.truncated), (10, true));
    assert_eq!(root.read_to_string("/bad", 10)?.text, "a\u{fffd}b");
    Ok(())
}

/// A writer that keeps replacing a parent directory with a symlink out of the root (and back)
/// while a reader loops. The reader may see the real file, a not-found, or an error, but never the
/// outside content. With path-string resolution this leaks within milliseconds.
#[test]
fn swapping_a_parent_directory_for_a_symlink_never_leaks() -> TestResult {
    let sb = Sandbox::new("swap")?;
    sb.write("park/dir/f", "INSIDE")?;
    sb.write("../outside/dir/f", "OUTSIDE")?;
    fs::write(sb.outside().join("dir/g"), "OUTSIDE")?;
    let root_path = sb.root_path();
    let outside_dir = sb.outside().join("dir");
    let stop = AtomicBool::new(false);

    let mut inside_reads = 0u32;
    let mut leaks = Vec::new();
    thread::scope(|scope| {
        scope.spawn(|| {
            // `swap` is either the real directory (renamed in from `park`) or a link to outside.
            let real = root_path.join("park/dir");
            let swap = root_path.join("swap");
            let parked = root_path.join("parked");
            let _ = fs::rename(&real, &swap);
            while !stop.load(Ordering::Relaxed) {
                // directory -> link (the window a check-then-open race needs)
                if fs::rename(&swap, &parked).is_err() {
                    continue;
                }
                let _ = symlink(&outside_dir, &swap);
                let _ = fs::remove_file(&swap);
                let _ = fs::rename(&parked, &swap);
            }
        });
        let root = RootFs::new(&root_path);
        let deadline = Instant::now() + Duration::from_millis(1500);
        while Instant::now() < deadline {
            for result in [
                root.read_to_string("/swap/f", 64).map(|f| f.text),
                root.read_to_string_nolink("/swap/f", 64).map(|f| f.text),
            ] {
                match result {
                    Ok(text) if text == "INSIDE" => inside_reads += 1,
                    Ok(text) => leaks.push(text),
                    Err(_) => {}
                }
            }
            if let Ok(names) = root.read_dir("/swap", 16)
                && names.names.iter().any(|name| name == "g")
            {
                leaks.push("directory listing of the outside directory".to_owned());
            }
        }
        stop.store(true, Ordering::Relaxed);
    });
    assert!(
        leaks.is_empty(),
        "content leaked from outside the root: {leaks:?}"
    );
    assert!(
        inside_reads > 0,
        "the reader never saw the real file; the test proved nothing"
    );
    Ok(())
}

/// A device node is refused as the wrong type without ever being read (`/dev/zero` would
/// otherwise fill the size cap, `/dev/null` would look like an empty file).
#[cfg(unix)]
#[test]
fn device_nodes_are_refused_not_read() {
    let root = RootFs::new("/");
    for device in ["/dev/zero", "/dev/null"] {
        assert!(
            matches!(root.read_to_string(device, 1024), Err(FsError::WrongType)),
            "{device}"
        );
        assert!(
            matches!(
                root.read_to_string_nolink(device, 1024),
                Err(FsError::WrongType)
            ),
            "{device}"
        );
    }
}
