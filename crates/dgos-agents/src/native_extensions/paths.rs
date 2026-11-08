use super::{config::CONFIG_LIMIT, Error};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{ffi::CString, fs::{File, Metadata}, io::{Read, Write}, os::{fd::{AsRawFd, FromRawFd}, unix::{ffi::OsStrExt, fs::MetadataExt}}, path::{Component, Path}};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Identity { pub dev: u64, pub ino: u64 }
impl Identity { pub fn of(meta: &Metadata) -> Self { Self { dev: meta.dev(), ino: meta.ino() } } }
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Snapshot { pub identity: Identity, pub hash: String, pub mode: u32 }
pub(super) fn hash(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn name(value: &std::ffi::OsStr) -> Result<CString, Error> { CString::new(value.as_bytes()).map_err(|_| Error::UnsafePath) }
fn checked_fd(fd: libc::c_int) -> Result<File, Error> {
    if fd < 0 { return Err(std::io::Error::last_os_error().into()); }
    Ok(unsafe { File::from_raw_fd(fd) })
}
// Fail closed on every extended ACE, including inherited deny entries. POSIX
// mode alone does not describe effective macOS access; never strip a user's ACL.
#[cfg(target_os = "macos")]
pub(super) fn no_extended_acl(file: &File) -> Result<(), Error> {
    unsafe extern "C" {
        fn acl_get_fd_np(fd: libc::c_int, kind: libc::c_int) -> *mut libc::c_void;
        fn acl_free(acl: *mut libc::c_void) -> libc::c_int;
    }
    let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), 0x100) };
    if acl.is_null() {
        // Darwin filesec_get_property(FILESEC_ACL) reports an absent ACL as
        // ENOENT even on a valid open FD. Capture errno before any other call.
        let error = std::io::Error::last_os_error();
        return if error.raw_os_error() == Some(libc::ENOENT) { Ok(()) } else { Err(Error::AccessDenied) };
    }
    // A returned ACL (even an explicit empty one) is conservatively unsupported.
    // Never interpret acl_get_entry's ambiguous EINVAL as proof of no entries.
    unsafe { acl_free(acl); }
    Err(Error::AccessDenied)
}
#[cfg(not(target_os = "macos"))]
pub(super) fn no_extended_acl(_file: &File) -> Result<(), Error> { Err(Error::Unsupported) }
pub(super) struct Directory { file: File }
pub(super) struct DirectoryLock { file: File }
impl Drop for DirectoryLock {
    fn drop(&mut self) {
        // A concurrent fork can retain this open file description until exec.
        // Release the transaction lock before closing our descriptor.
        unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN); }
    }
}
impl Directory {
    pub fn open(path: &Path) -> Result<Self, Error> {
        if !path.is_absolute() { return Err(Error::UnsafePath); }
        let root = CString::new("/").map_err(|_| Error::UnsafePath)?;
        let mut dir = Self { file: checked_fd(unsafe { libc::open(root.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) })? };
        for component in path.components() {
            match component {
                Component::RootDir => {},
                Component::Normal(part) => dir = dir.child(part, false)?,
                _ => return Err(Error::UnsafePath),
            }
        }
        Ok(dir)
    }
    pub fn nearest(path: &Path) -> Result<(std::path::PathBuf, Self), Error> {
        let mut path = path.to_path_buf();
        loop { match Self::open(&path) { Ok(dir) => return Ok((path, dir)), Err(Error::NotFound) => { if !path.pop() { return Err(Error::UnsafePath); } }, Err(e) => return Err(e) } }
    }
    pub fn ensure(path: &Path) -> Result<Self, Error> {
        let (base, mut dir) = Self::nearest(path)?;
        for component in path.strip_prefix(base).map_err(|_| Error::UnsafePath)?.components() {
            let Component::Normal(part) = component else { return Err(Error::UnsafePath); };
            dir.writable_owner()?;
            dir = dir.child(part, true)?;
        }
        Ok(dir)
    }
    pub fn child(&self, child: &std::ffi::OsStr, create: bool) -> Result<Self, Error> {
        if Path::new(child).components().count() != 1 || !matches!(Path::new(child).components().next(), Some(Component::Normal(_))) { return Err(Error::UnsafePath); }
        let child = name(child)?;
        if create {
            self.writable_owner()?;
            let result = unsafe { libc::mkdirat(self.file.as_raw_fd(), child.as_ptr(), 0o700) };
            if result != 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists { return Err(std::io::Error::last_os_error().into()); }
            self.sync()?;
        }
        let file = checked_fd(unsafe { libc::openat(self.file.as_raw_fd(), child.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) })?;
        let dir = Self { file };
        if create { dir.writable_owner()?; }
        Ok(dir)
    }
    pub fn identity(&self) -> Result<Identity, Error> { Ok(Identity::of(&self.file.metadata()?)) }
    pub fn private(&self) -> Result<(), Error> {
        let m = self.file.metadata()?;
        no_extended_acl(&self.file)?;
        if m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 { return Err(Error::AccessDenied); }
        Ok(())
    }
    pub fn writable_owner(&self) -> Result<(), Error> {
        let m = self.file.metadata()?;
        no_extended_acl(&self.file)?;
        if m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o022 != 0 { return Err(Error::AccessDenied); }
        Ok(())
    }
    pub fn kind(&self, child: &str) -> Result<&'static str, Error> {
        let child = name(child.as_ref())?;
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::fstatat(self.file.as_raw_fd(), child.as_ptr(), metadata.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) } != 0 {
            let error = std::io::Error::last_os_error();
            return if error.kind() == std::io::ErrorKind::NotFound { Ok("missing") } else { Err(error.into()) };
        }
        let metadata = unsafe { metadata.assume_init() };
        Ok(match metadata.st_mode & libc::S_IFMT {
            libc::S_IFREG => "file", libc::S_IFDIR => "directory", libc::S_IFLNK => "symlink", _ => "special",
        })
    }
    pub fn read_link(&self, child: &str, limit: usize) -> Result<(String, Identity), Error> {
        let child = name(child.as_ref())?;
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::fstatat(self.file.as_raw_fd(), child.as_ptr(), metadata.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) } != 0 { return Err(std::io::Error::last_os_error().into()); }
        let metadata = unsafe { metadata.assume_init() };
        if metadata.st_mode & libc::S_IFMT != libc::S_IFLNK { return Err(Error::Conflict); }
        let mut bytes = vec![0u8; limit.min(4096) + 1];
        let count = unsafe { libc::readlinkat(self.file.as_raw_fd(), child.as_ptr(), bytes.as_mut_ptr().cast(), bytes.len()) };
        if count < 0 { return Err(std::io::Error::last_os_error().into()); }
        if count as usize >= bytes.len() { return Err(Error::Limit); }
        bytes.truncate(count as usize);
        let target = String::from_utf8(bytes).map_err(|_| Error::InvalidFormat)?;
        let mut after = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::fstatat(self.file.as_raw_fd(), child.as_ptr(), after.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) } != 0 { return Err(std::io::Error::last_os_error().into()); }
        let after = unsafe { after.assume_init() };
        if metadata.st_dev != after.st_dev || metadata.st_ino != after.st_ino { return Err(Error::Conflict); }
        Ok((target, Identity { dev: metadata.st_dev as u64, ino: metadata.st_ino as u64 }))
    }
    pub fn read(&self, child: &str) -> Result<Option<(String, Snapshot)>, Error> {
        self.read_bounded(child, CONFIG_LIMIT)
    }
    pub fn read_bounded(&self, child: &str, limit: usize) -> Result<Option<(String, Snapshot)>, Error> {
        self.read_bytes(child, limit)?.map(|(bytes, snapshot)| Ok((String::from_utf8(bytes).map_err(|_| Error::InvalidFormat)?, snapshot))).transpose()
    }
    pub fn read_bytes(&self, child: &str, limit: usize) -> Result<Option<(Vec<u8>, Snapshot)>, Error> {
        let limit = limit.min(CONFIG_LIMIT);
        let child = name(child.as_ref())?;
        let fd = unsafe { libc::openat(self.file.as_raw_fd(), child.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            return if error.kind() == std::io::ErrorKind::NotFound { Ok(None) } else { Err(Error::UnsafePath) };
        }
        let mut file = checked_fd(fd)?; let before = file.metadata()?;
        if !before.is_file() || before.nlink() != 1 || before.uid() != unsafe { libc::geteuid() } { return Err(Error::UnsafePath); }
        #[cfg(target_os = "macos")]
        no_extended_acl(&file)?;
        if before.len() > limit as u64 { return Err(Error::Limit); }
        let mut bytes = Vec::new();
        (&mut file).take(limit as u64 + 1).read_to_end(&mut bytes)?;
        let after = file.metadata()?;
        if bytes.len() > limit || before.len() != after.len() || before.mtime() != after.mtime() || before.mtime_nsec() != after.mtime_nsec() { return Err(Error::Conflict); }
        let snapshot = Snapshot { identity: Identity::of(&after), hash: hash(&bytes), mode: after.mode() & 0o777 };
        Ok(Some((bytes, snapshot)))
    }
    pub fn write_new(&self, child: &str, bytes: &[u8], mode: u32) -> Result<(), Error> {
        self.writable_owner()?;
        let child = name(child.as_ref())?;
        let mut file = checked_fd(unsafe { libc::openat(self.file.as_raw_fd(), child.as_ptr(), libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC, 0o600) })?;
        no_extended_acl(&file)?;
        if mode & 0o022 != 0 { return Err(Error::AccessDenied); }
        file.write_all(bytes)?;
        if unsafe { libc::fchmod(file.as_raw_fd(), mode as libc::mode_t) } != 0 { return Err(std::io::Error::last_os_error().into()); }
        file.sync_all()?;
        self.sync()
    }
    pub fn replace_journal(&self, child: &str, bytes: &[u8]) -> Result<(), Error> {
        let temp = format!("journal-{}", uuid::Uuid::new_v4());
        self.write_new(&temp, bytes, 0o600)?;
        let source = name(temp.as_ref())?; let dest = name(child.as_ref())?;
        if unsafe { libc::renameat(self.file.as_raw_fd(), source.as_ptr(), self.file.as_raw_fd(), dest.as_ptr()) } != 0 { return Err(std::io::Error::last_os_error().into()); }
        self.sync()
    }
    pub fn sync(&self) -> Result<(), Error> { self.file.sync_all().map_err(Into::into) }
    pub fn lock(&self) -> Result<DirectoryLock, Error> {
        let child = CString::new(".dgos-native.lock").map_err(|_| Error::UnsafePath)?;
        self.writable_owner()?;
        let file = checked_fd(unsafe { libc::openat(self.file.as_raw_fd(), child.as_ptr(), libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC, 0o600) })?;
        let m = file.metadata()?;
        if !m.is_file() || m.nlink() != 1 || m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 { return Err(Error::UnsafePath); }
        no_extended_acl(&file)?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 { return Err(Error::Busy); }
        // The lock file is persistent. Never unlink a lock while another process
        // may still hold its inode, which would create two independent locks.
        Ok(DirectoryLock { file })
    }
    #[cfg(target_os = "macos")]
    pub fn publish(&self, stage: &Directory, staged: &str, target: &str, exists: bool) -> Result<(), Error> {
        self.writable_owner()?; stage.writable_owner()?;
        let source = name(staged.as_ref())?; let dest = name(target.as_ref())?;
        // RENAME_EXCL = 4, RENAME_SWAP = 2 (Darwin renameatx_np).
        let flags = if exists { 2 } else { 4 };
        unsafe extern "C" { fn renameatx_np(fromfd: libc::c_int, from: *const libc::c_char, tofd: libc::c_int, to: *const libc::c_char, flags: libc::c_uint) -> libc::c_int; }
        if unsafe { renameatx_np(stage.file.as_raw_fd(), source.as_ptr(), self.file.as_raw_fd(), dest.as_ptr(), flags) } != 0 { return Err(std::io::Error::last_os_error().into()); }
        stage.sync()?; self.sync()
    }
    #[cfg(not(target_os = "macos"))]
    pub fn publish(&self, _stage: &Directory, _staged: &str, _target: &str, _exists: bool) -> Result<(), Error> { Err(Error::Unsupported) }
    /// Move an accurate instance into private recovery without replacing anything.
    #[cfg(target_os = "macos")]
    pub fn move_to(&self, child: &str, destination: &Directory, name_in_destination: &str) -> Result<(), Error> {
        destination.publish(self, child, name_in_destination, false)
    }
    #[cfg(not(target_os = "macos"))]
    pub fn move_to(&self, _child: &str, _destination: &Directory, _name: &str) -> Result<(), Error> { Err(Error::Unsupported) }
    pub fn names(&self, limit: usize) -> Result<(Vec<String>, bool), Error> {
        // fdopendir consumes its descriptor; duplicate it and close via closedir.
        let fd = unsafe { libc::dup(self.file.as_raw_fd()) };
        if fd < 0 { return Err(std::io::Error::last_os_error().into()); }
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() { unsafe { libc::close(fd); } return Err(Error::Unavailable); }
        struct Stream(*mut libc::DIR);
        impl Drop for Stream { fn drop(&mut self) { unsafe { libc::closedir(self.0); } } }
        let stream = Stream(stream);
        // dup shares an offset, so rewind before every bounded enumeration.
        unsafe { libc::rewinddir(stream.0); }
        let mut names = Vec::new(); let mut incomplete = false;
        let mut visited = 0;
        loop {
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() { break; }
            let bytes = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if bytes == b"." || bytes == b".." { continue; }
            if visited == limit { incomplete = true; break; }
            visited += 1;
            match std::str::from_utf8(bytes) { Ok(name) => names.push(name.into()), Err(_) => incomplete = true }
        }
        names.sort();
        Ok((names, incomplete))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_transaction_releases_lock_with_duplicate_descriptor_alive() {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!("dgos-native-lock-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let dir = Directory::open(&root).unwrap();
        let held = dir.lock().unwrap();
        // A concurrent fork inherits the same open file description until exec.
        let inherited = held.file.try_clone().unwrap();
        drop(held);
        let next = dir.lock().expect("completed transaction must release its lock");
        drop(next);
        drop(inherited);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
