//! The project reader's path confinement, byte budgets, and observed-byte cache.
//! This policy is deliberately local to project loading, not shared with writers.

use super::{Limits, Result, error};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub(super) fn relative(value: &str) -> Result<()> {
    if value.is_empty()
        || value.contains(['\\', ':', '\0'])
        || value
            .split('/')
            .any(|c| c.is_empty() || c == "." || c == "..")
        || Path::new(value).is_absolute()
    {
        return Err(error(
            "UNSAFE_PATH",
            format!("expected a confined relative path: {value:?}"),
        ));
    }
    Ok(())
}

pub(super) fn join(base: &str, child: &str) -> Result<String> {
    relative(base)?;
    relative(child)?;
    Ok(format!("{base}/{child}"))
}

/// A read-only logical object tree. Implementations must never fall back to
/// working files; Reader still owns all project byte/file/discovery budgets.
pub(crate) trait ProjectSource {
    fn read_blob(&self, path: &str, limit: usize) -> Result<Vec<u8>>;
    fn directory(&self, path: &str, limit: usize) -> Result<Vec<String>>;
}

enum Source<'a> {
    Filesystem(File),
    Objects(&'a dyn ProjectSource),
}

/// Descriptor-relative filesystem access. No symlink inside the repository is followed.
/// Individual observations are protected; this is not a transactional tree snapshot.
pub(super) struct Reader<'a> {
    root: Source<'a>,
    pub(super) limits: Limits,
    total_bytes: usize,
    entries: usize,
    files: usize,
    pub(super) contents: BTreeMap<String, Vec<u8>>,
}

impl<'a> Reader<'a> {
    pub(super) fn new(root: &Path, limits: Limits) -> Result<Self> {
        Ok(Self {
            root: Source::Filesystem(confined::root(root)?),
            limits,
            total_bytes: 0,
            entries: 0,
            files: 0,
            contents: BTreeMap::new(),
        })
    }

    pub(super) fn from_source(source: &'a dyn ProjectSource, limits: Limits) -> Self {
        Self {
            root: Source::Objects(source),
            limits,
            total_bytes: 0,
            entries: 0,
            files: 0,
            contents: BTreeMap::new(),
        }
    }

    pub(super) fn charge(&mut self, count: usize) -> Result<()> {
        self.entries = self
            .entries
            .checked_add(count)
            .ok_or_else(|| error("LIMIT_EXCEEDED", "entry count"))?;
        if self.entries > self.limits.entries {
            return Err(error("LIMIT_EXCEEDED", "entry count"));
        }
        Ok(())
    }

    pub(super) fn read(&mut self, path: &str) -> Result<Vec<u8>> {
        self.read_limited(path, self.limits.file_bytes)
    }

    pub(super) fn read_limited(&mut self, path: &str, file_limit: usize) -> Result<Vec<u8>> {
        relative(path)?;
        if let Some(bytes) = self.contents.get(path) {
            if bytes.len() > file_limit {
                return Err(error("LIMIT_EXCEEDED", format!("{path}: file byte limit")));
            }
            return Ok(bytes.clone());
        }
        if self.files >= self.limits.files {
            return Err(error("LIMIT_EXCEEDED", "file count"));
        }
        let remaining = self.limits.total_bytes.saturating_sub(self.total_bytes);
        let limit = file_limit.min(remaining);
        let bytes = match &self.root {
            Source::Filesystem(root) => {
                let mut file = confined::open(root, path, false)?;
                if file
                    .metadata()
                    .map_err(|_| error("READ_FAILED", path))?
                    .len()
                    > limit as u64
                {
                    return Err(error(
                        "LIMIT_EXCEEDED",
                        format!("{path}: file or aggregate byte limit"),
                    ));
                }
                let mut bytes = Vec::new();
                file.by_ref()
                    .take((limit as u64).saturating_add(1))
                    .read_to_end(&mut bytes)
                    .map_err(|_| error("READ_FAILED", path))?;
                if bytes.len() > limit {
                    return Err(error(
                        "LIMIT_EXCEEDED",
                        format!("{path}: file or aggregate byte limit"),
                    ));
                }
                bytes
            }
            Source::Objects(source) => {
                let bytes = source.read_blob(path, limit)?;
                if bytes.len() > limit {
                    return Err(error(
                        "LIMIT_EXCEEDED",
                        format!("{path}: file or aggregate byte limit"),
                    ));
                }
                bytes
            }
        };
        self.total_bytes += bytes.len();
        self.files += 1;
        self.contents.insert(path.to_owned(), bytes.clone());
        Ok(bytes)
    }

    pub(super) fn directory(&mut self, path: &str) -> Result<Vec<String>> {
        relative(path)?;
        let limit = self.limits.entries.saturating_sub(self.entries);
        let names = match &self.root {
            Source::Filesystem(root) => confined::names(confined::open(root, path, true)?, limit)?,
            Source::Objects(source) => source.directory(path, limit)?,
        };
        if names.len() > limit {
            return Err(error("LIMIT_EXCEEDED", "directory entry count"));
        }
        self.charge(names.len())?;
        Ok(names)
    }
}

#[cfg(unix)]
mod confined {
    use super::*;
    use std::ffi::{CStr, CString};
    use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
    use std::os::unix::fs::OpenOptionsExt;

    pub fn root(path: &Path) -> Result<File> {
        // The caller chooses the root. Resolve its aliases once; confinement starts here.
        let path = path
            .canonicalize()
            .map_err(|_| error("INVALID_ROOT", "repository root is unavailable"))?;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|_| {
                error(
                    "INVALID_ROOT",
                    "repository root must be a readable directory",
                )
            })
    }

    pub fn open(root: &File, path: &str, directory: bool) -> Result<File> {
        relative(path)?;
        let mut parent = root.try_clone().map_err(|_| error("READ_FAILED", path))?;
        let components: Vec<_> = path.split('/').collect();
        for (index, component) in components.iter().enumerate() {
            let is_dir = index + 1 != components.len() || directory;
            let name = CString::new(*component).map_err(|_| error("UNSAFE_PATH", path))?;
            let mut before = std::mem::MaybeUninit::<libc::stat>::uninit();
            // SAFETY: name is terminated, parent is live, and fstatat initializes before on success.
            let rc = unsafe {
                libc::fstatat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    before.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if rc != 0 {
                return Err(error("PATH_UNAVAILABLE", path));
            }
            // SAFETY: successful fstatat above initialized the structure.
            let before = unsafe { before.assume_init() };
            let kind = before.st_mode & libc::S_IFMT;
            if kind == libc::S_IFLNK {
                return Err(error("SYMLINK_REJECTED", path));
            }
            if kind != if is_dir { libc::S_IFDIR } else { libc::S_IFREG } {
                return Err(error("INVALID_FILE_TYPE", path));
            }
            let flags = libc::O_RDONLY
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | libc::O_CLOEXEC
                | if is_dir { libc::O_DIRECTORY } else { 0 };
            // SAFETY: parent is a live directory descriptor and name is terminated.
            let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
            if fd < 0 {
                return Err(error("PATH_UNAVAILABLE", path));
            }
            // SAFETY: fd was newly opened and is uniquely owned by this File.
            let next = unsafe { File::from_raw_fd(fd) };
            use std::os::unix::fs::MetadataExt;
            let actual = next.metadata().map_err(|_| error("READ_FAILED", path))?;
            // libc's dev_t/ino_t widths differ across supported Unix targets.
            #[allow(clippy::unnecessary_cast)]
            let expected_identity = (before.st_dev as u64, before.st_ino as u64);
            if (actual.dev(), actual.ino()) != expected_identity
                || if is_dir {
                    !actual.is_dir()
                } else {
                    !actual.is_file()
                }
            {
                return Err(error("FILESYSTEM_CHANGED", path));
            }
            parent = next;
        }
        Ok(parent)
    }

    pub fn names(file: File, limit: usize) -> Result<Vec<String>> {
        struct Directory(*mut libc::DIR);
        impl Drop for Directory {
            fn drop(&mut self) {
                unsafe {
                    libc::closedir(self.0);
                }
            }
        }
        let fd = file.into_raw_fd();
        // SAFETY: fd is an owned directory descriptor; fdopendir takes it on success.
        let raw = unsafe { libc::fdopendir(fd) };
        if raw.is_null() {
            // SAFETY: fdopendir failed and did not take ownership.
            unsafe {
                libc::close(fd);
            }
            return Err(error("READ_FAILED", "cannot list projection directory"));
        }
        let dir = Directory(raw);
        let mut names = Vec::new();
        loop {
            // POSIX readdir requires clearing errno to distinguish EOF from failure.
            errno::set_errno(errno::Errno(0));
            // SAFETY: dir stays live and this function is its exclusive reader.
            let entry = unsafe { libc::readdir(dir.0) };
            if entry.is_null() {
                if errno::errno().0 != 0 {
                    return Err(error("READ_FAILED", "cannot list projection directory"));
                }
                break;
            }
            // SAFETY: readdir returns a terminated d_name valid until the next call.
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
                .to_str()
                .map_err(|_| error("INVALID_UTF8", "projection directory entry"))?;
            if name == "." || name == ".." {
                continue;
            }
            if names.len() >= limit {
                return Err(error("LIMIT_EXCEEDED", "directory entry count"));
            }
            relative(name)?;
            names.push(name.to_owned());
        }
        names.sort();
        Ok(names)
    }
}

#[cfg(not(unix))]
mod confined {
    use super::*;
    use crate::project::Error;

    fn unsupported() -> Error {
        error(
            "UNSUPPORTED_PLATFORM",
            "confined project inspection currently requires Unix",
        )
    }
    pub fn root(_: &Path) -> Result<File> {
        Err(unsupported())
    }
    pub fn open(_: &File, _: &str, _: bool) -> Result<File> {
        Err(unsupported())
    }
    pub fn names(_: File, _: usize) -> Result<Vec<String>> {
        Err(unsupported())
    }
}
