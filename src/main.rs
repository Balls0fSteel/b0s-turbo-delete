/*
  Copyright 2022 Tejas Ravishankar

  Licensed under the Apache License, Version 2.0 (the "License");
  you may not use this file except in compliance with the License.
  You may obtain a copy of the License at

      http://www.apache.org/licenses/LICENSE-2.0

  Unless required by applicable law or agreed to in writing, software
  distributed under the License is distributed on an "AS IS" BASIS,
  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
  See the License for the specific language governing permissions and
  limitations under the License.
*/

mod delete;

use delete::Stats;
use indicatif::{ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;
use std::ffi::{OsStr, OsString};
use std::fmt::Display;
use std::fs::{self, Metadata};
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const HELP: &str = "\
td - fast, parallel folder delete

Usage: td <path> [-y|--yes]

Deletes <path> and everything inside it. Symlinks and junctions inside the
folder are removed without touching what they point to.

Asks for confirmation first, unless --yes is given. Refuses to delete a drive
or filesystem root, your home folder, or the current folder or any folder
containing it.

Options:
  -y, --yes      Delete without asking for confirmation
  -h, --help     Print this help
  -V, --version  Print the version";

/// Rayon worker stack size. Directories are deleted recursively; a release
/// build needs well under 1 KiB of stack per level, so this covers the
/// deepest paths Windows allows (32,767 characters) with a wide margin. It is
/// reserved address space, not memory that is actually used.
const WORKER_STACK_SIZE: usize = 16 * 1024 * 1024;

enum Command {
    Help,
    Version,
    Delete { path: OsString, yes: bool },
}

fn main() -> ExitCode {
    let command = match parse_args(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(message) => {
            print_error(format!(
                "{message}\n\nUsage: td <path> [-y|--yes]  (see td --help)"
            ));
            return ExitCode::from(2);
        }
    };
    match command {
        Command::Help => println!("{HELP}"),
        Command::Version => println!("td {}", env!("CARGO_PKG_VERSION")),
        Command::Delete { path, yes } => {
            if let Err(message) = run(&path, yes) {
                print_error(message);
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

fn parse_args(args: impl Iterator<Item = OsString>) -> Result<Command, String> {
    let mut path = None;
    let mut yes = false;
    let mut only_paths = false;
    for arg in args {
        let is_option = !only_paths && arg.to_string_lossy().starts_with('-');
        match arg.to_str() {
            Some("-h" | "--help") if is_option => return Ok(Command::Help),
            Some("-V" | "--version") if is_option => return Ok(Command::Version),
            Some("-y" | "--yes") if is_option => yes = true,
            Some("--") if is_option => only_paths = true,
            _ if is_option => return Err(format!("unknown option {}", arg.to_string_lossy())),
            _ if path.is_some() => return Err("expected exactly one path".into()),
            _ => path = Some(arg),
        }
    }
    match path {
        Some(path) => Ok(Command::Delete { path, yes }),
        None => Err("no path given".into()),
    }
}

fn run(arg: &OsStr, yes: bool) -> Result<(), String> {
    let target = resolve_target(arg)?;
    let shown = target.display();
    // The guards run before the prompt and before anything is modified.
    let metadata = fs::symlink_metadata(&target);
    check_safe(&target, metadata.as_ref().ok())?;
    let metadata = metadata.map_err(|e| format!("cannot delete \"{shown}\": {e}"))?;

    if !yes {
        confirm(&target)?;
    }

    // If the pool cannot be configured, rayon falls back to its defaults.
    let _ = rayon::ThreadPoolBuilder::new()
        .stack_size(WORKER_STACK_SIZE)
        .build_global();

    let stats = Stats::default();
    let start = Instant::now();
    with_spinner(&stats, || {
        delete::delete(&target, metadata.file_type(), &stats)
    });
    let elapsed = start.elapsed().as_secs_f64();

    let files = stats.files.load(Relaxed);
    let dirs = stats.dirs.load(Relaxed);
    println!("Deleted {files} files, {dirs} folders in {elapsed:.2}s");

    let errors = stats.errors.load(Relaxed);
    if errors == 0 {
        return Ok(());
    }
    let mut message = format!("{errors} item(s) could not be deleted:");
    let first = stats.first_errors();
    for line in &first {
        message.push_str("\n  ");
        message.push_str(line);
    }
    if errors > first.len() as u64 {
        message.push_str(&format!("\n  ... and {} more", errors - first.len() as u64));
    }
    Err(message)
}

/// Turns the argument into the absolute, lexically normalized path that is
/// checked, shown in the prompt, and deleted: all three are the same path.
fn resolve_target(arg: &OsStr) -> Result<PathBuf, String> {
    let arg = fix_windows_quote(arg);
    if arg.is_empty() {
        return Err("the path is empty".into());
    }
    let absolute = std::path::absolute(&arg)
        .map_err(|e| format!("cannot resolve \"{}\": {e}", Path::new(&arg).display()))?;
    Ok(normalize(&absolute))
}

/// Windows passes `"C:\some dir\"` to programs as `C:\some dir"`, because
/// `\"` is an escaped quote. `"` cannot appear in Windows file names, so a
/// trailing one always came from a trailing backslash: put the backslash back.
/// (Dropping it instead would turn `"D:\"` into `D:`, which means "the current
/// folder on drive D".)
fn fix_windows_quote(arg: &OsStr) -> OsString {
    if cfg!(windows) {
        if let Some(stripped) = arg.to_str().and_then(|s| s.strip_suffix('"')) {
            return format!("{stripped}\\").into();
        }
    }
    arg.to_owned()
}

/// Removes `.` and `..` components and trailing separators without touching
/// the filesystem. A trailing separator matters: `link/` would resolve to the
/// link's target, while `link` is the link itself.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Refuses filesystem roots, the home folder and its ancestors, and the
/// current folder and its ancestors.
///
/// Both the target as given and, for a real directory, its canonical form
/// (with links in its parents resolved) are checked, so a path that reaches
/// a protected folder through a symlink is refused too.
fn check_safe(target: &Path, metadata: Option<&Metadata>) -> Result<(), String> {
    let refuse = |why: &str| {
        Err(format!(
            "refusing to delete \"{}\": {why}",
            target.display()
        ))
    };

    let mut targets = vec![target.to_path_buf()];
    if metadata.is_some_and(Metadata::is_dir) {
        targets.extend(fs::canonicalize(target).ok());
    }

    let mut protected = Vec::new();
    // Read from the environment (not the user database) so tests can point it
    // at a scratch folder.
    let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    if let Some(home) = std::env::var_os(home_var).map(PathBuf::from) {
        if home.is_absolute() {
            protected.push((normalize(&home), "it is or contains your home folder"));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        protected.push((normalize(&cwd), "it is or contains the current folder"));
    }
    let canonical: Vec<_> = protected
        .iter()
        .filter_map(|(path, why)| Some((fs::canonicalize(path).ok()?, *why)))
        .collect();
    protected.extend(canonical);

    for target in &targets {
        if target.parent().is_none() {
            return refuse("it is a drive or filesystem root");
        }
        for (path, why) in &protected {
            if is_same_or_ancestor(target, path) {
                return refuse(why);
            }
        }
    }
    Ok(())
}

/// Whether `ancestor` is `path` or one of its parents, comparing whole
/// components (case-insensitively on Windows).
fn is_same_or_ancestor(ancestor: &Path, path: &Path) -> bool {
    let ancestor = comparable_components(ancestor);
    let path = comparable_components(path);
    path.starts_with(&ancestor)
}

#[cfg(not(windows))]
fn comparable_components(path: &Path) -> Vec<OsString> {
    path.components()
        .map(|c| c.as_os_str().to_owned())
        .collect()
}

/// Lower-cased components, with `\\?\C:` treated as `C:` and `\\?\UNC\server\share`
/// as `\\server\share`, so the different spellings of a path compare equal.
#[cfg(windows)]
fn comparable_components(path: &Path) -> Vec<OsString> {
    use std::path::Prefix;
    path.components()
        .map(|component| {
            let text = match component {
                Component::Prefix(prefix) => match prefix.kind() {
                    Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                        format!("{}:", drive as char)
                    }
                    Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => format!(
                        r"\\{}\{}",
                        server.to_string_lossy(),
                        share.to_string_lossy()
                    ),
                    _ => component.as_os_str().to_string_lossy().into_owned(),
                },
                other => other.as_os_str().to_string_lossy().into_owned(),
            };
            OsString::from(text.to_lowercase())
        })
        .collect()
}

fn confirm(target: &Path) -> Result<(), String> {
    let stdin = io::stdin();
    if !stdin.is_terminal() {
        return Err(
            "not deleting without confirmation: input is not a terminal \
                    (pass --yes to delete without asking)"
                .into(),
        );
    }
    eprint!("Delete \"{}\"? [y/N] ", target.display());
    let _ = io::stderr().flush();
    let mut answer = String::new();
    stdin
        .lock()
        .read_line(&mut answer)
        .map_err(|e| format!("cannot read answer: {e}"))?;
    match answer.trim().to_lowercase().as_str() {
        "y" | "yes" => Ok(()),
        _ => Err("aborted, nothing was deleted".into()),
    }
}

/// Runs `work` while a spinner (drawn only when stderr is a terminal) shows
/// the live counts.
fn with_spinner<T>(stats: &Stats, work: impl FnOnce() -> T) -> T {
    let spinner = ProgressBar::new_spinner();
    if let Ok(style) = ProgressStyle::with_template("{spinner} {msg}") {
        // ASCII, so it also renders in consoles without braille glyphs.
        spinner.set_style(style.tick_chars("|/-\\ "));
    }
    let (done, stopped) = mpsc::channel::<()>();
    let result = std::thread::scope(|scope| {
        let spinner = &spinner;
        scope.spawn(move || {
            while stopped.recv_timeout(Duration::from_millis(100))
                == Err(mpsc::RecvTimeoutError::Timeout)
            {
                spinner.set_message(format!(
                    "Deleted {} files, {} folders",
                    stats.files.load(Relaxed),
                    stats.dirs.load(Relaxed)
                ));
                spinner.tick();
            }
        });
        let result = work();
        drop(done);
        result
    });
    spinner.finish_and_clear();
    result
}

fn print_error(message: impl Display) {
    // Only color a terminal, and on Windows only one known to understand ANSI
    // codes (Windows Terminal); the legacy console would print them raw.
    let color = io::stderr().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && (cfg!(not(windows)) || std::env::var_os("WT_SESSION").is_some());
    if color {
        eprintln!("{} {message}", "error:".bright_red().bold());
    } else {
        eprintln!("error: {message}");
    }
}
