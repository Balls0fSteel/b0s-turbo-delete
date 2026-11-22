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

//! Unix engine: deletes relative to open directory handles.
//!
//! Each directory is opened once (`O_DIRECTORY | O_NOFOLLOW`), listed through
//! that handle, and its children are removed with `unlinkat` on it. Compared
//! with full paths this means:
//!
//! - the kernel resolves one name per call instead of the whole path from the
//!   root, so the cost per file does not grow with depth;
//! - no path is allocated per entry (full paths are built only for errors);
//! - a directory replaced by a symlink while we work is never entered:
//!   `O_NOFOLLOW` refuses it and the link itself is unlinked.

use super::Stats;
use rayon::prelude::*;
use rustix::fs::{self as rfs, AtFlags, CWD, Dir, FileType, Mode, OFlags, RawMode};
use rustix::io::Errno;
use std::ffi::{CStr, CString, OsStr};
use std::fs;
use std::io;
use std::os::fd::BorrowedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering::Relaxed};

const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// Directories with no subdirectories and at most this many entries are
/// deleted on the current thread: splitting them would cost more than it saves.
const SEQUENTIAL_MAX: usize = 16;

/// Removed files are added to the shared counter in batches of this size,
/// instead of every thread hitting the same cache line once per file.
const COUNT_BATCH: u64 = 256;

/// Where an entry is. The full path is only built when it is needed (to
/// report an error, or for the by-path fallback).
enum Loc<'a> {
    Root(&'a Path),
    Child(&'a Loc<'a>, &'a CStr),
}

impl Loc<'_> {
    fn path(&self) -> PathBuf {
        match self {
            Loc::Root(path) => path.to_path_buf(),
            Loc::Child(parent, name) => parent.path().join(OsStr::from_bytes(name.to_bytes())),
        }
    }
}

pub fn delete(root: &Path, file_type: fs::FileType, stats: &Stats) {
    if !file_type.is_dir() {
        // A single file or link: nothing to gain from handles.
        super::delete_by_path(root, file_type, stats);
        return;
    }
    let Ok(name) = CString::new(root.as_os_str().as_bytes()) else {
        stats.fail(root, &io::Error::from(io::ErrorKind::InvalidInput));
        return;
    };
    FD_BUDGET.store(fd_budget(), Relaxed);
    // The root is opened and removed by its full path relative to the current
    // directory; its parent is outside the tree, so it is never modified.
    delete_dir_at(CWD, &name, &Loc::Root(root), false, stats);
}

/// Deletes the directory `name` inside `parent`, then the directory itself.
/// Returns whether it is gone.
fn delete_dir_at(
    parent: BorrowedFd<'_>,
    name: &CStr,
    loc: &Loc<'_>,
    parent_in_tree: bool,
    stats: &Stats,
) -> bool {
    // Each directory holds a descriptor until its subdirectories are done.
    // When they run short (a tree deeper than the limit), finish this subtree
    // by path, which holds none for longer than a listing.
    let Some(_slot) = FdSlot::take() else {
        return delete_by_path(loc, parent_in_tree, stats);
    };
    let mut dir = match open_dir(parent, name) {
        Ok(dir) => dir,
        Err(Errno::NOENT) => return true,
        // No longer a directory (replaced by a link or a file since it was
        // listed; FreeBSD reports a link as EMLINK): remove the entry itself.
        Err(Errno::LOOP | Errno::NOTDIR | Errno::MLINK) => {
            return match remove_at(parent, name, AtFlags::empty(), parent_in_tree) {
                Ok(()) => {
                    stats.files.fetch_add(1, Relaxed);
                    true
                }
                Err(e) => fail(stats, loc, e),
            };
        }
        // Out of descriptors anyway (other users of the limit).
        Err(Errno::MFILE | Errno::NFILE) => return delete_by_path(loc, parent_in_tree, stats),
        Err(e) => return fail(stats, loc, e),
    };

    let emptied = delete_contents(&mut dir, loc, stats);
    drop(dir);
    // A child that could not be deleted is already reported; removing this
    // directory would only add a "directory not empty" error.
    if !emptied {
        return false;
    }
    match remove_at(parent, name, AtFlags::REMOVEDIR, parent_in_tree) {
        Ok(()) => {
            stats.dirs.fetch_add(1, Relaxed);
            true
        }
        Err(e) => fail(stats, loc, e),
    }
}

/// Deletes everything in `dir`. Returns whether all of it is gone.
fn delete_contents(dir: &mut Dir, loc: &Loc<'_>, stats: &Stats) -> bool {
    let mut entries = Vec::new();
    let mut has_dirs = false;
    let mut listed = true;
    while let Some(entry) = dir.read() {
        match entry {
            Ok(entry) => {
                let name = entry.file_name();
                if name == c"." || name == c".." {
                    continue;
                }
                // The type comes from the listing and is never a link's target.
                let file_type = entry.file_type();
                has_dirs |= !matches!(file_type, FileType::RegularFile | FileType::Symlink);
                entries.push((name.to_owned(), file_type));
            }
            Err(e) => {
                fail(stats, loc, e);
                listed = false;
                break;
            }
        }
    }
    let fd = match dir.fd() {
        Ok(fd) => fd,
        Err(e) => return fail(stats, loc, e),
    };

    let delete_one = |mut acc: Acc, (name, file_type): &(CString, FileType)| {
        if !delete_child(fd, name, *file_type, loc, stats, &mut acc) {
            acc.failed += 1;
        }
        acc
    };
    // `fold` (unlike `all`) does not stop early, so one failure does not
    // leave unrelated siblings undeleted.
    let acc = if !has_dirs && entries.len() <= SEQUENTIAL_MAX {
        entries.iter().fold(Acc::default(), delete_one)
    } else {
        entries
            .par_iter()
            .fold(Acc::default, delete_one)
            .reduce(Acc::default, Acc::merge)
    };
    stats.files.fetch_add(acc.uncounted_files, Relaxed);
    listed && acc.failed == 0
}

fn delete_child(
    dir: BorrowedFd<'_>,
    name: &CStr,
    mut file_type: FileType,
    parent: &Loc<'_>,
    stats: &Stats,
    acc: &mut Acc,
) -> bool {
    let loc = Loc::Child(parent, name);
    if file_type == FileType::Unknown {
        // Some filesystems do not report types in listings.
        match rfs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => file_type = FileType::from_raw_mode(stat.st_mode as RawMode),
            Err(Errno::NOENT) => return true,
            Err(e) => return fail(stats, &loc, e),
        }
    }
    if file_type == FileType::Directory {
        return delete_dir_at(dir, name, &loc, true, stats);
    }
    match remove_at(dir, name, AtFlags::empty(), true) {
        Ok(()) => {
            acc.add_file(stats);
            true
        }
        Err(e) => fail(stats, &loc, e),
    }
}

/// Opens a directory without following links. If listing it is not
/// permitted, grants the owner read and search permission once and retries
/// (`name` is in the tree being deleted: the root itself or below it).
fn open_dir(parent: BorrowedFd<'_>, name: &CStr) -> Result<Dir, Errno> {
    let open = || rfs::openat(parent, name, DIR_FLAGS, Mode::empty());
    let fd = match open() {
        Err(Errno::ACCESS) if add_owner_bits_at(parent, name, 0o500) => open(),
        other => other,
    }?;
    Dir::new(fd)
}

/// `unlinkat`, treating "not found" as success. On "permission denied" it
/// grants the owner write and search permission on the parent once and
/// retries, but only when the parent is inside the tree being deleted.
fn remove_at(
    parent: BorrowedFd<'_>,
    name: &CStr,
    flags: AtFlags,
    parent_in_tree: bool,
) -> Result<(), Errno> {
    let remove = || match rfs::unlinkat(parent, name, flags) {
        Err(Errno::NOENT) => Ok(()),
        other => other,
    };
    match remove() {
        Err(Errno::ACCESS | Errno::PERM) if parent_in_tree && add_owner_bits(parent, 0o300) => {
            remove()
        }
        other => other,
    }
}

/// Adds `bits` to the owner permissions of the open directory `dir`.
/// Returns false if they were already set (so they are not the problem).
fn add_owner_bits(dir: BorrowedFd<'_>, bits: RawMode) -> bool {
    let Ok(stat) = rfs::fstat(dir) else {
        return false;
    };
    let mode = stat.st_mode as RawMode & 0o7777;
    if mode & bits == bits {
        return false;
    }
    rfs::fchmod(dir, Mode::from_raw_mode(mode | bits)).is_ok()
}

/// Adds `bits` to the owner permissions of the directory `name` in `parent`,
/// if it is a directory and not a link.
fn add_owner_bits_at(parent: BorrowedFd<'_>, name: &CStr, bits: RawMode) -> bool {
    let Ok(stat) = rfs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) else {
        return false;
    };
    if FileType::from_raw_mode(stat.st_mode as RawMode) != FileType::Directory {
        return false;
    }
    let mode = stat.st_mode as RawMode & 0o7777;
    if mode & bits == bits {
        return false;
    }
    // Linux cannot chmod without following links (`fchmodat` rejects
    // AT_SYMLINK_NOFOLLOW); the entry was just checked to be a directory.
    let flags = if cfg!(any(target_os = "linux", target_os = "android")) {
        AtFlags::empty()
    } else {
        AtFlags::SYMLINK_NOFOLLOW
    };
    rfs::chmodat(parent, name, Mode::from_raw_mode(mode | bits), flags).is_ok()
}

/// Finishes a subtree with the by-path engine.
fn delete_by_path(loc: &Loc<'_>, parent_in_tree: bool, stats: &Stats) -> bool {
    let path = loc.path();
    match fs::symlink_metadata(&path) {
        Ok(metadata) => super::delete_entry(&path, metadata.file_type(), parent_in_tree, stats),
        Err(e) if e.kind() == io::ErrorKind::NotFound => true,
        Err(e) => {
            stats.fail(&path, &e);
            false
        }
    }
}

fn fail(stats: &Stats, loc: &Loc<'_>, err: Errno) -> bool {
    stats.fail(&loc.path(), &err.into());
    false
}

/// Descriptors that open directories may still use.
static FD_BUDGET: AtomicI64 = AtomicI64::new(0);

/// One descriptor reserved from `FD_BUDGET`, returned on drop.
struct FdSlot;

impl FdSlot {
    fn take() -> Option<FdSlot> {
        if FD_BUDGET.fetch_sub(1, Relaxed) > 0 {
            Some(FdSlot)
        } else {
            FD_BUDGET.fetch_add(1, Relaxed);
            None
        }
    }
}

impl Drop for FdSlot {
    fn drop(&mut self) {
        FD_BUDGET.fetch_add(1, Relaxed);
    }
}

/// Raises the soft descriptor limit as far as allowed, and returns how many
/// descriptors open directories may hold: the limit minus a reserve for
/// stdio, the by-path fallback (one listing per thread) and the runtime.
fn fd_budget() -> i64 {
    raise_fd_limit();
    let limit = rustix::process::getrlimit(rustix::process::Resource::Nofile).current;
    let limit = limit.map_or(i64::MAX, |n| i64::try_from(n).unwrap_or(i64::MAX));
    let threads = i64::try_from(rayon::current_num_threads()).unwrap_or(i64::MAX);
    limit
        .saturating_sub(64)
        .saturating_sub(threads.saturating_mul(2))
}

fn raise_fd_limit() {
    use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
    let limit = getrlimit(Resource::Nofile);
    let Some(current) = limit.current else {
        return; // Unlimited already.
    };
    // macOS rejects values above OPEN_MAX (10240), whatever the hard limit.
    for want in [limit.maximum, Some(10240)] {
        if want.is_none_or(|want| want > current)
            && setrlimit(
                Resource::Nofile,
                Rlimit {
                    current: want,
                    maximum: limit.maximum,
                },
            )
            .is_ok()
        {
            return;
        }
    }
}

/// Per-thread tally of one directory's children.
#[derive(Default)]
struct Acc {
    failed: u64,
    uncounted_files: u64,
}

impl Acc {
    fn add_file(&mut self, stats: &Stats) {
        self.uncounted_files += 1;
        if self.uncounted_files >= COUNT_BATCH {
            stats.files.fetch_add(self.uncounted_files, Relaxed);
            self.uncounted_files = 0;
        }
    }

    fn merge(self, other: Acc) -> Acc {
        Acc {
            failed: self.failed + other.failed,
            uncounted_files: self.uncounted_files + other.uncounted_files,
        }
    }
}
