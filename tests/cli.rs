//! End-to-end tests that run the built `td` binary.
//!
//! Every tree lives in a fresh temporary directory, and the binary runs with
//! its current directory inside one, so a bug can only ever delete scratch
//! data. Tests of the root guard never pass `--yes`: if the guard failed, the
//! "input is not a terminal" refusal would still stop the deletion (the test
//! would then fail on the error message, with nothing deleted).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

/// A `td` command with stdin closed, running inside `cwd`.
fn td(cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_td"));
    command.current_dir(cwd).stdin(Stdio::null());
    command
}

fn run(cwd: &Path, args: &[&std::ffi::OsStr]) -> Output {
    td(cwd).args(args).output().expect("failed to run td")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "td failed: {:?}\nstdout: {}\nstderr: {}",
        output.status,
        stdout(output),
        stderr(output)
    );
}

fn assert_refused(output: &Output, message: &str) {
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(output));
    assert!(
        stderr(output).contains(message),
        "expected {message:?} in stderr: {}",
        stderr(output)
    );
}

/// Runs `td --yes <path>` from a separate scratch directory.
fn delete_yes(path: &Path) -> Output {
    let cwd = TempDir::new().unwrap();
    run(cwd.path(), &["--yes".as_ref(), path.as_os_str()])
}

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Creates `depth` nested directories named `d` under `root`; returns the
/// innermost one.
fn nest(root: &Path, depth: usize) -> PathBuf {
    let mut path = root.to_path_buf();
    for _ in 0..depth {
        path.push("d");
    }
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn deletes_nested_tree_and_reports_counts() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("tree");
    write(&root.join("a.txt"), "a");
    write(&root.join(".hidden"), "h");
    write(&root.join("sub/b.txt"), "b");
    write(&root.join("sub/deeper/c.txt"), "c");
    write(&root.join("ünïcødé/файл.txt"), "u");
    fs::create_dir_all(root.join("empty/also-empty")).unwrap();
    for i in 0..500 {
        write(&root.join(format!("wide/{i}.bin")), "x");
    }

    let output = delete_yes(&root);
    assert_success(&output);
    assert!(!root.exists());
    assert!(tmp.path().exists());
    // Files: a, .hidden, b, c, файл, 500 in wide. Folders: tree, sub, deeper,
    // ünïcødé, empty, also-empty, wide.
    assert!(
        stdout(&output).starts_with("Deleted 505 files, 7 folders in "),
        "stdout: {}",
        stdout(&output)
    );
}

#[test]
fn deletes_deep_nesting() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("deep");
    write(&nest(&root, 300).join("leaf.txt"), "leaf");

    assert_success(&delete_yes(&root));
    assert!(!root.exists());
}

/// Paths deeper than PATH_MAX (4096 bytes on Linux, 1024 on macOS) cannot be
/// passed to the kernel at all; td must work relative to directory handles.
#[cfg(unix)]
#[test]
fn deletes_paths_longer_than_path_max() {
    use rustix::fs::{CWD, Mode, OFlags, mkdirat, openat};
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("long");
    fs::create_dir(&root).unwrap();
    let flags = OFlags::RDONLY | OFlags::DIRECTORY;
    let mut dir = openat(CWD, &root, flags, Mode::empty()).unwrap();
    // 1200 levels of "/dddd" is 6000 bytes.
    for _ in 0..1200 {
        mkdirat(&dir, "dddd", Mode::from_raw_mode(0o755)).unwrap();
        dir = openat(&dir, "dddd", flags, Mode::empty()).unwrap();
    }
    let file = OFlags::WRONLY | OFlags::CREATE;
    drop(openat(&dir, "leaf.txt", file, Mode::from_raw_mode(0o644)).unwrap());
    drop(dir);

    let output = delete_yes(&root);
    assert_success(&output);
    assert!(!root.exists());
    assert!(
        stdout(&output).starts_with("Deleted 1 files, 1201 folders in "),
        "stdout: {}",
        stdout(&output)
    );
}

/// Each directory being deleted holds a file descriptor; a tree deeper than
/// the descriptor limit must still be deleted completely.
#[cfg(unix)]
#[test]
fn deletes_trees_deeper_than_the_file_descriptor_limit() {
    use std::os::unix::process::CommandExt;
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("deep");
    // Deeper than the limit set below, but shorter than macOS's PATH_MAX.
    write(&nest(&root, 400).join("leaf.txt"), "leaf");

    let cwd = TempDir::new().unwrap();
    let mut command = td(cwd.path());
    command.arg("--yes").arg(&root);
    // SAFETY: setrlimit is async-signal-safe and allocates nothing.
    unsafe {
        command.pre_exec(|| {
            use rustix::process::{Resource, Rlimit, setrlimit};
            let limit = Rlimit {
                current: Some(256),
                maximum: Some(256),
            };
            setrlimit(Resource::Nofile, limit).map_err(std::io::Error::from)
        });
    }
    let output = command.output().unwrap();
    assert_success(&output);
    assert!(!root.exists());
    assert!(
        stdout(&output).starts_with("Deleted 1 files, 401 folders in "),
        "stdout: {}",
        stdout(&output)
    );
}

#[test]
fn deletes_read_only_files_and_folders() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("ro");
    let file = root.join("file.txt");
    let ro_dir = root.join("ro-dir");
    write(&file, "x");
    write(&ro_dir.join("inner.txt"), "y");
    set_read_only(&file);
    set_read_only(&ro_dir.join("inner.txt"));
    set_read_only(&ro_dir);
    #[cfg(unix)]
    {
        // Unreadable and unsearchable: listing it needs a permission fix too.
        use std::os::unix::fs::PermissionsExt;
        let locked = root.join("locked");
        write(&locked.join("secret.txt"), "z");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    }
    set_read_only(&root);

    let output = delete_yes(&root);
    assert_success(&output);
    assert!(!root.exists());
}

fn set_read_only(path: &Path) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions).unwrap();
}

#[test]
fn missing_path_fails_cleanly() {
    let tmp = TempDir::new().unwrap();
    let output = delete_yes(&tmp.path().join("does-not-exist"));
    assert_refused(&output, "cannot delete");
    assert!(!stderr(&output).contains("panicked"));
}

#[test]
fn deletes_a_plain_file() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("file.txt");
    write(&file, "x");

    let output = delete_yes(&file);
    assert_success(&output);
    assert!(!file.exists());
    assert!(stdout(&output).starts_with("Deleted 1 files, 0 folders"));
}

#[test]
fn path_after_double_dash_may_start_with_a_dash() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("-dash/file"), "x");

    let output = run(
        tmp.path(),
        &["--yes".as_ref(), "--".as_ref(), "-dash".as_ref()],
    );
    assert_success(&output);
    assert!(!tmp.path().join("-dash").exists());
}

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).unwrap();
    true
}

/// Directory symlinks need a privilege (or Developer Mode) on Windows, so
/// symlink tests are skipped when creating one fails.
#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> bool {
    match std::os::windows::fs::symlink_dir(target, link) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("skipping: cannot create a directory symlink: {e}");
            false
        }
    }
}

/// An outside folder with some content, to check that it survives.
fn outside_dir(tmp: &Path) -> PathBuf {
    let outside = tmp.join("outside");
    write(&outside.join("keep.txt"), "keep");
    write(&outside.join("nested/keep2.txt"), "keep");
    outside
}

fn assert_outside_intact(outside: &Path) {
    assert_eq!(
        fs::read_to_string(outside.join("keep.txt")).unwrap(),
        "keep"
    );
    assert_eq!(
        fs::read_to_string(outside.join("nested/keep2.txt")).unwrap(),
        "keep"
    );
}

#[test]
fn never_follows_symlinks_out_of_the_tree() {
    let tmp = TempDir::new().unwrap();
    let outside = outside_dir(tmp.path());
    let root = tmp.path().join("tree");
    write(&root.join("own.txt"), "x");
    if !symlink_dir(&outside, &root.join("link-to-outside")) {
        return;
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.join("keep.txt"), root.join("file-link")).unwrap();

    let output = delete_yes(&root);
    assert_success(&output);
    assert!(!root.exists());
    assert_outside_intact(&outside);
}

#[test]
fn symlinked_root_removes_only_the_link() {
    let tmp = TempDir::new().unwrap();
    let outside = outside_dir(tmp.path());
    let link = tmp.path().join("link");
    if !symlink_dir(&outside, &link) {
        return;
    }

    let output = delete_yes(&link);
    assert_success(&output);
    assert!(fs::symlink_metadata(&link).is_err(), "link still exists");
    assert_outside_intact(&outside);
}

/// `link/` would make the OS resolve the link; it must still mean the link.
#[cfg(unix)]
#[test]
fn symlinked_root_with_trailing_slash_removes_only_the_link() {
    let tmp = TempDir::new().unwrap();
    let outside = outside_dir(tmp.path());
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&outside, &link).unwrap();

    let mut arg = link.clone().into_os_string();
    arg.push("/");
    let output = delete_yes(Path::new(&arg));
    assert_success(&output);
    assert!(fs::symlink_metadata(&link).is_err(), "link still exists");
    assert_outside_intact(&outside);
}

#[cfg(windows)]
#[test]
fn never_follows_junctions_out_of_the_tree() {
    let tmp = TempDir::new().unwrap();
    let outside = outside_dir(tmp.path());
    let root = tmp.path().join("tree");
    write(&root.join("own.txt"), "x");
    let junction = root.join("junction");
    let status = Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(&junction)
        .arg(&outside)
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "mklink /J failed");

    let output = delete_yes(&root);
    assert_success(&output);
    assert!(!root.exists());
    assert_outside_intact(&outside);
}

/// Explorer passes `"C:\dir\"`, which arrives as `C:\dir"`.
#[cfg(windows)]
#[test]
fn trailing_quote_from_explorer_is_handled() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("quoted");
    write(&root.join("file.txt"), "x");

    let mut arg = root.clone().into_os_string();
    arg.push("\"");
    let output = delete_yes(Path::new(&arg));
    assert_success(&output);
    assert!(!root.exists());
}

#[test]
fn refuses_filesystem_root() {
    let tmp = TempDir::new().unwrap();
    // `/` on Unix, the drive root (e.g. `C:\`) on Windows.
    let root = tmp.path().ancestors().last().unwrap();
    // Also reached through `..` from inside the scratch directory.
    let mut dotted = tmp.path().to_path_buf();
    for _ in 0..64 {
        dotted.push("..");
    }
    for target in [root, &dotted] {
        let output = run(tmp.path(), &[target.as_os_str()]);
        assert_refused(&output, "drive or filesystem root");
    }
}

#[test]
fn refuses_home_folder_and_its_ancestors() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    write(&home.join("precious.txt"), "x");
    let cwd = TempDir::new().unwrap();

    for target in [&home, tmp.path()] {
        let output = td(cwd.path())
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .args(["--yes".as_ref(), target.as_os_str()])
            .output()
            .unwrap();
        assert_refused(&output, "your home folder");
    }
    assert!(home.join("precious.txt").exists());
}

#[test]
fn refuses_current_folder_and_its_ancestors() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path().join("a/b");
    write(&cwd.join("precious.txt"), "x");

    let targets = [
        PathBuf::from("."),
        PathBuf::from(".."),
        PathBuf::from("../b/./"),
        cwd.clone(),
        tmp.path().join("a"),
    ];
    for target in &targets {
        let output = run(&cwd, &["--yes".as_ref(), target.as_os_str()]);
        assert_refused(&output, "the current folder");
    }
    assert!(cwd.join("precious.txt").exists());
}

/// A path that reaches the current folder through a symlinked parent.
#[cfg(unix)]
#[test]
fn refuses_current_folder_reached_through_a_symlink() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path().join("real/work");
    write(&cwd.join("precious.txt"), "x");
    std::os::unix::fs::symlink(tmp.path().join("real"), tmp.path().join("alias")).unwrap();

    let target = tmp.path().join("alias/work");
    let output = run(&cwd, &["--yes".as_ref(), target.as_os_str()]);
    assert_refused(&output, "the current folder");
    assert!(cwd.join("precious.txt").exists());
}

#[cfg(windows)]
#[test]
fn guards_ignore_case_on_windows() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path().join("Mixed");
    write(&cwd.join("precious.txt"), "x");

    let upper = cwd.to_string_lossy().to_uppercase();
    let output = run(&cwd, &["--yes".as_ref(), upper.as_ref()]);
    assert_refused(&output, "the current folder");
    assert!(cwd.join("precious.txt").exists());
}

#[test]
fn refuses_without_yes_when_input_is_not_a_terminal() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("tree");
    write(&root.join("file.txt"), "x");

    let output = run(tmp.path(), &[root.as_os_str()]);
    assert_refused(&output, "--yes");
    assert!(root.join("file.txt").exists());
}

/// Answers the prompt through a pseudo-terminal provided by util-linux
/// `script`; skipped where it is not available.
#[cfg(target_os = "linux")]
#[test]
fn prompt_answers_are_respected() {
    use std::io::Write;

    let has_script = Command::new("script")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !has_script {
        eprintln!("skipping: util-linux `script` not found");
        return;
    }
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("tree");
    write(&root.join("file.txt"), "x");

    let answer = |answer: &str| {
        let command = format!("'{}' '{}'", env!("CARGO_BIN_EXE_td"), root.display());
        let mut child = Command::new("script")
            .args(["-qec", &command, "/dev/null"])
            .current_dir(tmp.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(answer.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };

    let output = answer("n\n");
    assert!(stdout(&output).contains("[y/N]"), "{}", stdout(&output));
    assert!(!output.status.success());
    assert!(root.join("file.txt").exists());

    let output = answer("y\n");
    assert!(output.status.success(), "{}", stdout(&output));
    assert!(!root.exists());
}

/// When the target's own parent is read-only, deletion must fail rather than
/// change the permissions of a folder outside the target.
#[cfg(unix)]
#[test]
fn never_changes_permissions_outside_the_target() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let parent = tmp.path().join("parent");
    let target = parent.join("target");
    write(&target.join("file.txt"), "x");
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).unwrap();
    let mode = || fs::metadata(&parent).unwrap().permissions().mode() & 0o7777;

    // Root (or CAP_DAC_OVERRIDE) ignores permissions, so this cannot fail.
    let privileged = fs::write(parent.join("probe"), "").is_ok();
    if privileged {
        eprintln!("skipping: running with permission-bypassing privileges");
    } else {
        let output = delete_yes(&target);
        assert_refused(&output, "could not be deleted");
        assert!(stdout(&output).starts_with("Deleted 1 files, 0 folders"));
        assert_eq!(mode(), 0o555);
        assert!(target.exists());
    }
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn usage_errors() {
    let tmp = TempDir::new().unwrap();
    for args in [&[][..], &["--bogus"], &["a", "b"]] {
        let output = td(tmp.path()).args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "args {args:?}");
        assert!(stderr(&output).contains("Usage"));
    }
    let output = td(tmp.path()).arg("").output().unwrap();
    assert_refused(&output, "empty");
}

#[test]
fn help_and_version() {
    let tmp = TempDir::new().unwrap();
    let output = td(tmp.path()).arg("--help").output().unwrap();
    assert_success(&output);
    assert!(stdout(&output).contains("Usage: td <path>"));

    let output = td(tmp.path()).arg("-V").output().unwrap();
    assert_success(&output);
    assert_eq!(
        stdout(&output).trim(),
        format!("td {}", env!("CARGO_PKG_VERSION"))
    );
}
