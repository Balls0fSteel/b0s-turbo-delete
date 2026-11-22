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

//! Parallel, bottom-up deletion of a directory tree.
//!
//! Every directory lists its entries once and deletes them in parallel
//! (rayon work-stealing); subdirectories recurse the same way, and a
//! directory is removed only after all of its children are gone. Each entry
//! is touched by exactly one thread, so nothing races.
//!
//! Links are never followed: the type of every entry comes from the directory
//! listing (which reports a symlink or junction as a link, not as what it
//! points to), and links are unlinked, leaving their targets untouched.

use rayon::prelude::*;
use std::fs::{self, FileType};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// How many failures are kept for the final report (all are counted).
pub const MAX_REPORTED_ERRORS: usize = 10;

/// Counters shared by all worker threads.
#[derive(Default)]
pub struct Stats {
    /// Files and links removed.
    pub files: AtomicU64,
    /// Directories removed.
    pub dirs: AtomicU64,
    /// Entries that could not be removed (or listed).
    pub errors: AtomicU64,
    first_errors: Mutex<Vec<String>>,
}

impl Stats {
    fn fail(&self, path: &Path, err: &io::Error) {
        self.errors.fetch_add(1, Relaxed);
        let mut first = self.first_errors.lock().unwrap_or_else(|e| e.into_inner());
        if first.len() < MAX_REPORTED_ERRORS {
            first.push(format!("{}: {err}", path.display()));
        }
    }

    /// The first few failures, as `path: error` lines.
    pub fn first_errors(&self) -> Vec<String> {
        self.first_errors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// Deletes `root`, whose own type (from `symlink_metadata`) is `file_type`.
///
/// A real directory is deleted recursively; anything else (file, symlink,
/// junction) is just unlinked. Failures are recorded in `stats`.
pub fn delete(root: &Path, file_type: FileType, stats: &Stats) {
    // The root's parent is outside the tree, so it is never modified.
    delete_entry(root, file_type, false, stats);
}

fn delete_entry(path: &Path, file_type: FileType, parent_in_tree: bool, stats: &Stats) -> bool {
    // `is_dir` is false for symlinks and junctions on every platform, and on
    // Windows `is_symlink` is true for both.
    if file_type.is_dir() {
        return delete_dir(path, parent_in_tree, stats);
    }
    let remove = |path: &Path| remove_link_or_file(path, file_type);
    match remove_with_retry(path, file_type.is_symlink(), parent_in_tree, remove) {
        Ok(()) => {
            stats.files.fetch_add(1, Relaxed);
            true
        }
        Err(e) => {
            stats.fail(path, &e);
            false
        }
    }
}

fn delete_dir(dir: &Path, parent_in_tree: bool, stats: &Stats) -> bool {
    let entries = match list_dir(dir, stats) {
        Ok(entries) => entries,
        Err(e) => {
            stats.fail(dir, &e);
            return false;
        }
    };

    // Delete all children in parallel. `count` (unlike `all`) does not stop
    // early, so one failure does not leave unrelated siblings undeleted.
    let failed = entries
        .into_par_iter()
        .filter(|(path, file_type)| !delete_entry(path, *file_type, true, stats))
        .count();

    // A child that could not be deleted is already reported; trying to
    // remove this directory would only add a "directory not empty" error.
    if failed > 0 {
        return false;
    }
    match remove_with_retry(dir, false, parent_in_tree, |dir| fs::remove_dir(dir)) {
        Ok(()) => {
            stats.dirs.fetch_add(1, Relaxed);
            true
        }
        Err(e) => {
            stats.fail(dir, &e);
            false
        }
    }
}

/// Lists `dir`. Entries that cannot be read are reported and skipped.
fn list_dir(dir: &Path, stats: &Stats) -> io::Result<Vec<(PathBuf, FileType)>> {
    let read_dir = match fs::read_dir(dir) {
        Err(e) if e.kind() == ErrorKind::PermissionDenied && make_listable(dir) => {
            fs::read_dir(dir)?
        }
        other => other?,
    };
    let mut entries = Vec::new();
    for entry in read_dir {
        // `DirEntry::file_type` never follows links.
        match entry.and_then(|entry| Ok((entry.path(), entry.file_type()?))) {
            Ok(entry) => entries.push(entry),
            Err(e) => stats.fail(dir, &e),
        }
    }
    Ok(entries)
}

/// Runs `remove(path)`, treating "not found" as success. On "permission
/// denied" it tries once to make `path` deletable and retries; on Windows it
/// also retries briefly on errors caused by other processes (antivirus,
/// indexers) that hold files open or have not finished deleting them.
fn remove_with_retry(
    path: &Path,
    is_link: bool,
    parent_in_tree: bool,
    remove: impl Fn(&Path) -> io::Result<()>,
) -> io::Result<()> {
    const TRANSIENT_RETRIES: u64 = 5;
    let mut fixed_permissions = false;
    let mut transient_retries = 0;
    loop {
        let err = match remove(path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
            Err(e) => e,
        };
        if err.kind() == ErrorKind::PermissionDenied && !fixed_permissions {
            fixed_permissions = true;
            if make_deletable(path, is_link, parent_in_tree) {
                continue;
            }
        }
        if is_transient(&err) && transient_retries < TRANSIENT_RETRIES {
            transient_retries += 1;
            std::thread::sleep(std::time::Duration::from_millis(20 * transient_retries));
            continue;
        }
        return Err(err);
    }
}

/// Removes a file or a link (never anything a link points to).
fn remove_link_or_file(path: &Path, file_type: FileType) -> io::Result<()> {
    #[cfg(windows)]
    {
        // Directory symlinks and junctions are directories to Windows and
        // must be removed with `remove_dir`, which deletes only the link.
        use std::os::windows::fs::FileTypeExt;
        if file_type.is_symlink_dir() {
            return fs::remove_dir(path);
        }
    }
    #[cfg(not(windows))]
    let _ = file_type;
    fs::remove_file(path)
}

/// Unix: removing an entry needs write and search (x) permission on its
/// parent directory. Grants them to the owner, but only when the parent is
/// inside the tree being deleted.
#[cfg(unix)]
fn make_deletable(path: &Path, _is_link: bool, parent_in_tree: bool) -> bool {
    match path.parent() {
        Some(parent) if parent_in_tree => add_owner_bits(parent, 0o300),
        _ => false,
    }
}

/// Unix: listing a directory needs read and search (x) permission on it.
#[cfg(unix)]
fn make_listable(dir: &Path) -> bool {
    add_owner_bits(dir, 0o500)
}

#[cfg(unix)]
fn add_owner_bits(dir: &Path, bits: u32) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Ok(metadata) = fs::symlink_metadata(dir) else {
        return false;
    };
    if !metadata.is_dir() {
        return false;
    }
    let mode = metadata.permissions().mode() & 0o7777;
    if mode & bits == bits {
        return false; // Already set: permissions are not the problem.
    }
    fs::set_permissions(dir, fs::Permissions::from_mode(mode | bits)).is_ok()
}

/// Windows: the read-only attribute blocks deleting a file or directory.
/// Clears it on the entry itself. Links are skipped, because setting
/// attributes might reach through to the link's target.
#[cfg(windows)]
fn make_deletable(path: &Path, is_link: bool, _parent_in_tree: bool) -> bool {
    if is_link {
        return false;
    }
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    let mut permissions = metadata.permissions();
    if !permissions.readonly() {
        return false;
    }
    // On Windows this only clears FILE_ATTRIBUTE_READONLY.
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions).is_ok()
}

/// Windows: listing is governed by ACLs, which are not ours to change.
#[cfg(windows)]
fn make_listable(_dir: &Path) -> bool {
    false
}

#[cfg(windows)]
fn is_transient(err: &io::Error) -> bool {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;
    // A directory whose children were all deleted can still report "not
    // empty" while another process holds one of them open.
    err.kind() == ErrorKind::DirectoryNotEmpty
        || matches!(
            err.raw_os_error(),
            Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
        )
}

#[cfg(not(windows))]
fn is_transient(_err: &io::Error) -> bool {
    false
}
