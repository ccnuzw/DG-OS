//! Durable ownership evidence and a minimal, in-group EOF guardian.
//!
//! The journal contains only machine/boot identities and a process-group ID. A
//! held flock makes an unfinished reservation distinguishable from no execution.
use std::{path::Path, sync::Arc};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Observation { NotStarted, Stopped, Running, Unavailable }

#[cfg(unix)]
mod unix {
    use super::*;
    use std::{
        ffi::CString,
        fs::File,
        io::{Read, Write},
        os::{fd::{AsRawFd, FromRawFd, OwnedFd, RawFd}, unix::{ffi::OsStrExt, fs::MetadataExt}},
        path::Component,
        sync::atomic::{AtomicBool, Ordering},
    };

    const MAGIC: &[u8; 8] = b"DGOSOWN1";
    const SIZE: usize = 48;
    const STATE_OFFSET: libc::off_t = 40;
    const GROUP_OFFSET: libc::off_t = 44;
    const ERROR: &str = "process_ownership_unavailable";

    pub(crate) struct OwnershipJournal {
        file: File,
        started: AtomicBool,
        directory: File,
    }

    // Keep guardian sources above stdio, including hosts launched with closed
    // stdin/stdout. Duplication is atomically close-on-exec.
    fn owned(fd: RawFd) -> Result<OwnedFd, String> {
        if fd < 0 { return Err(ERROR.into()); }
        let original = unsafe { OwnedFd::from_raw_fd(fd) };
        if fd >= 3 { return Ok(original); }
        let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
        if duplicate < 0 { return Err(ERROR.into()); }
        Ok(unsafe { OwnedFd::from_raw_fd(duplicate) })
    }

    fn lifeline(directory: &File) -> Result<(OwnedFd, OwnedFd), String> {
        let name = CString::new(format!("{}.lifeline", uuid::Uuid::new_v4())).map_err(|_| ERROR)?;
        if unsafe { libc::mkfifoat(directory.as_raw_fd(), name.as_ptr(), 0o600) } != 0 { return Err(ERROR.into()); }
        // Darwin has no pipe2: a FIFO opened with O_CLOEXEC avoids the
        // pipe()+fcntl() race with unrelated concurrent host spawns.
        let result = (|| {
            let reader = owned(unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW) })?;
            let writer = owned(unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW) })?;
            let flags = unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_SETFL, flags & !libc::O_NONBLOCK) } != 0 { return Err(ERROR.into()); }
            Ok((reader, writer))
        })();
        let unlinked = unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) } == 0;
        if !unlinked { return Err(ERROR.into()); }
        result
    }

    fn directory(root: &Path, create: bool) -> Result<File, String> {
        let absolute = if root.is_absolute() { root.to_owned() } else {
            std::env::current_dir().map_err(|_| ERROR)?.join(root)
        };
        // macOS' standard temporary/data parents can include /var ->
        // /private/var. Resolve the trusted parent, never the ownership
        // directory itself; its final open still rejects a symlink.
        let parent = absolute.parent().ok_or(ERROR)?.canonicalize().map_err(|_| ERROR)?;
        let absolute = parent.join(absolute.file_name().ok_or(ERROR)?);
        let fd = unsafe { libc::open(c"/".as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
        if fd < 0 { return Err(ERROR.into()); }
        let mut current = unsafe { File::from_raw_fd(fd) };
        for component in absolute.components() {
            let Component::Normal(name) = component else {
                if matches!(component, Component::ParentDir) { return Err(ERROR.into()); }
                continue;
            };
            let name = CString::new(name.as_bytes()).map_err(|_| ERROR)?;
            if create && unsafe { libc::mkdirat(current.as_raw_fd(), name.as_ptr(), 0o700) } != 0
                && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
                return Err(ERROR.into());
            }
            let fd = unsafe { libc::openat(current.as_raw_fd(), name.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
            if fd < 0 { return Err(ERROR.into()); }
            current = unsafe { File::from_raw_fd(fd) };
        }
        let metadata = current.metadata().map_err(|_| ERROR)?;
        if metadata.uid() != unsafe { libc::geteuid() } { return Err(ERROR.into()); }
        if create && unsafe { libc::fchmod(current.as_raw_fd(), 0o700) } != 0 { return Err(ERROR.into()); }
        if !create && metadata.mode() & 0o777 != 0o700 { return Err(ERROR.into()); }
        Ok(current)
    }

    fn name(binding_id: &str) -> Result<CString, String> {
        let id = uuid::Uuid::parse_str(binding_id).map_err(|_| ERROR)?;
        CString::new(format!("{}.ownership", id.hyphenated())).map_err(|_| ERROR.into())
    }

    #[cfg(target_os = "linux")]
    fn identity() -> Result<([u8; 16], [u8; 16]), String> {
        let machine = std::fs::read_to_string("/etc/machine-id").map_err(|_| ERROR)?;
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").map_err(|_| ERROR)?;
        Ok((*uuid::Uuid::parse_str(machine.trim()).map_err(|_| ERROR)?.as_bytes(),
            *uuid::Uuid::parse_str(boot.trim()).map_err(|_| ERROR)?.as_bytes()))
    }

    #[cfg(target_os = "macos")]
    fn identity() -> Result<([u8; 16], [u8; 16]), String> {
        fn value(key: &std::ffi::CStr) -> Result<[u8; 16], String> {
            let mut bytes = [0u8; 128];
            let mut size = bytes.len();
            if unsafe { libc::sysctlbyname(key.as_ptr(), bytes.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) } != 0 || size > bytes.len() {
                return Err(ERROR.into());
            }
            let text = std::str::from_utf8(&bytes[..size]).map_err(|_| ERROR)?.trim_end_matches('\0');
            Ok(*uuid::Uuid::parse_str(text).map_err(|_| ERROR)?.as_bytes())
        }
        let mut machine = [0u8; 16];
        let timeout = libc::timespec { tv_sec: 1, tv_nsec: 0 };
        if unsafe { libc::gethostuuid(machine.as_mut_ptr(), &timeout) } != 0 { return Err(ERROR.into()); }
        Ok((machine, value(c"kern.bootsessionuuid")?))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn identity() -> Result<([u8; 16], [u8; 16]), String> { Err("process_ownership_unsupported".into()) }

    impl OwnershipJournal {
        pub(crate) fn reserve(root: &Path, binding_id: &str) -> Result<Arc<Self>, String> {
            let directory = directory(root, true)?;
            let name = name(binding_id)?;
            let (machine, boot) = identity()?;
            let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC, 0o600) };
            let mut file = File::from(owned(fd)?);
            let fd = file.as_raw_fd();
            // Empty or partially initialized files never prove NotStarted.
            if unsafe { libc::fchmod(fd, 0o600) } != 0 || unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) } != 0 { return Err(ERROR.into()); }
            let mut bytes = [0u8; SIZE];
            bytes[..8].copy_from_slice(MAGIC);
            bytes[8..24].copy_from_slice(&machine);
            bytes[24..40].copy_from_slice(&boot);
            file.write_all(&bytes).and_then(|_| file.sync_all()).and_then(|_| directory.sync_all()).map_err(|_| ERROR)?;
            Ok(Arc::new(Self { file, directory, started: AtomicBool::new(false) }))
        }

        /// Installs only raw, async-signal-safe operations in the post-fork child.
        pub(crate) fn prepare(self: &Arc<Self>, command: &mut tokio::process::Command) -> Result<Lifeline, String> {
            if self.started.swap(true, Ordering::AcqRel) { return Err(ERROR.into()); }
            let (reader, writer) = lifeline(&self.directory)?;
            let pipe = [reader.as_raw_fd(), writer.as_raw_fd()];
            let journal = self.file.as_raw_fd();
            let mut limit: libc::rlimit = unsafe { std::mem::zeroed() };
            if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 { return Err(ERROR.into()); }
            let max_fd = limit.rlim_cur.min(i32::MAX as _) as i32;
            unsafe {
                command.pre_exec(move || {
                    // No writer is inherited by the native process or guardian.
                    libc::close(pipe[1]);
                    if libc::setpgid(0, 0) != 0 { return Err(std::io::Error::last_os_error()); }
                    let group = libc::getpgrp().to_ne_bytes();
                    if !persist(journal, group.as_ptr(), group.len(), GROUP_OFFSET) { return Err(std::io::Error::last_os_error()); }
                    // The committed start marker MUST precede exec; a failed or
                    // interrupted write can never be interpreted as no execution.
                    let started = [1u8];
                    if !persist(journal, started.as_ptr(), 1, STATE_OFFSET) { return Err(std::io::Error::last_os_error()); }
                    let guardian = libc::fork();
                    if guardian < 0 { return Err(std::io::Error::last_os_error()); }
                    if guardian == 0 { guard(pipe[0], journal, max_fd); }
                    libc::close(pipe[0]);
                    Ok(())
                });
            }
            Ok(Lifeline { reader: Some(reader), writer: Some(writer), _ownership: self.clone() })
        }

        /// Called only after kernel group inspection AND reaping the native child.
        pub(crate) fn record_stopped(&self) -> Result<(), String> {
            if unsafe { persist(self.file.as_raw_fd(), [2u8].as_ptr(), 1, STATE_OFFSET) } { Ok(()) } else { Err(ERROR.into()) }
        }
    }
    #[cfg(target_os = "macos")]
    unsafe fn errno() -> i32 { *libc::__error() }
    #[cfg(target_os = "linux")]
    unsafe fn errno() -> i32 { *libc::__errno_location() }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    unsafe fn errno() -> i32 { std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO) }


    // Called after fork: no allocation, Rust destructors, tracing, or locks.
    unsafe fn persist(fd: RawFd, bytes: *const u8, count: usize, offset: libc::off_t) -> bool {
        let mut done = 0;
        while done < count {
            let written = libc::pwrite(fd, bytes.add(done).cast(), count - done, offset + done as libc::off_t);
            if written < 0 {
                if errno() == libc::EINTR { continue; }
                return false;
            }
            if written == 0 { return false; }
            done += written as usize;
        }
        loop {
            if libc::fsync(fd) == 0 { return true; }
            if errno() != libc::EINTR { return false; }
        }
    }

    unsafe fn guard(reader: RawFd, journal: RawFd, max_fd: i32) -> ! {
        // Retaining the journal's flock also covers host death during spawn.
        if libc::dup2(reader, 0) < 0 || libc::dup2(journal, 1) < 0 {
            libc::kill(0, libc::SIGKILL);
            libc::_exit(127);
        }
        #[cfg(target_os = "linux")]
        if libc::syscall(libc::SYS_close_range, 2u32, u32::MAX, 0u32) != 0 {
            for fd in 2..max_fd { libc::close(fd); }
        }
        #[cfg(not(target_os = "linux"))]
        for fd in 2..max_fd { libc::close(fd); }
        let mut byte = 0u8;
        loop {
            let count = libc::read(0, (&mut byte as *mut u8).cast(), 1);
            if count > 0 { continue; }
            if count < 0 && errno() == libc::EINTR { continue; }
            // Signal our CURRENT group, never a cached/recyclable PGID. This
            // guardian remains a live member until SIGKILL kills it as well.
            libc::kill(0, libc::SIGKILL);
            libc::_exit(127);
        }
    }

    pub(crate) struct Lifeline { reader: Option<OwnedFd>, writer: Option<OwnedFd>, _ownership: Arc<OwnershipJournal> }
    impl Lifeline {
        pub(crate) fn spawned(&mut self) { self.reader.take(); }
        pub(crate) fn stop(&mut self) { self.writer.take(); }
    }

    pub(crate) fn observe(root: &Path, binding_id: &str) -> Result<Observation, String> {
        let name = name(binding_id)?;
        let directory = match directory(root, false) { Ok(directory) => directory, Err(_) => return Ok(Observation::Unavailable) };
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK) };
        if fd < 0 { return Ok(Observation::Unavailable); }
        let mut file = unsafe { File::from_raw_fd(fd) };
        let metadata = file.metadata().map_err(|_| ERROR)?;
        if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o777 != 0o600 || metadata.nlink() != 1 {
            return Ok(Observation::Unavailable);
        }
        // Lock BEFORE reading: reading a reserved marker before its owner
        // commits spawn and releases the lock could otherwise yield stale
        // NotStarted evidence after the process has actually executed.
        let locked = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) } == 0;
        let mut bytes = [0u8; SIZE];
        if metadata.len() != SIZE as u64 || file.read_exact(&mut bytes).is_err() || &bytes[..8] != MAGIC { return Ok(Observation::Unavailable); }
        let (machine, boot) = identity()?;
        if bytes[8..24] != machine { return Ok(Observation::Unavailable); }
        if bytes[24..40] != boot { return Ok(Observation::Stopped); }
        match bytes[40] {
            0 => Ok(if locked { Observation::NotStarted } else { Observation::Unavailable }),
            1 => {
                let group = i32::from_ne_bytes(bytes[44..48].try_into().unwrap());
                if group <= 1 { return Ok(Observation::Unavailable); }
                Ok(match group_executable(group) {
                    Ok(true) => Observation::Running,
                    Ok(false) => Observation::Stopped,
                    Err(_) => Observation::Unavailable,
                })
            }
            2 => Ok(Observation::Stopped),
            _ => Ok(Observation::Unavailable),
        }
    }

    /// Read-only kernel observation. Zombies cannot execute or create descendants.
    pub(crate) fn group_executable(group: i32) -> Result<bool, String> {
        if group <= 1 { return Err(ERROR.into()); }
        if unsafe { libc::kill(-group, 0) } != 0 {
            match std::io::Error::last_os_error().raw_os_error() {
                Some(libc::ESRCH) => return Ok(false),
                Some(libc::EPERM) => {
                    // Darwin also reports EPERM for zombie-only groups; its
                    // complete group inventory can prove this below. Linux
                    // permission failures are never absence evidence.
                    #[cfg(not(target_os = "macos"))]
                    return Err(ERROR.into());
                },
                _ => return Err(ERROR.into()),
            }
        }
        members_executable(group)
    }

    #[cfg(target_os = "linux")]
    fn members_executable(group: i32) -> Result<bool, String> {
        // hidepid=2 can omit members entirely, rather than returning an error
        // when stat is read. An incomplete inventory cannot prove inactivity.
        let mounts = std::fs::read_to_string("/proc/mounts").map_err(|_| ERROR)?;
        let visible = mounts.lines().any(|line| {
            let mut fields = line.split_whitespace();
            fields.next();
            fields.next() == Some("/proc") && fields.next() == Some("proc") && fields.next().is_some_and(|options|
                !options.split(',').any(|option| option.starts_with("hidepid=") && option != "hidepid=0" && option != "hidepid=off"))
        });
        if !visible { return Err(ERROR.into()); }
        fn snapshot(group: i32) -> Result<Option<Vec<(u32, u64)>>, String> {
            let mut zombies = Vec::new();
            for entry in std::fs::read_dir("/proc").map_err(|_| ERROR)? {
                let entry = entry.map_err(|_| ERROR)?;
                let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue; };
                let stat = match std::fs::read_to_string(entry.path().join("stat")) {
                    Ok(stat) => stat,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(_) => return Err(ERROR.into()),
                };
                let (_, fields) = stat.rsplit_once(')').ok_or(ERROR)?;
                let mut fields = fields.split_whitespace();
                let state = fields.next().ok_or(ERROR)?;
                fields.next().ok_or(ERROR)?;
                let member_group: i32 = fields.next().ok_or(ERROR)?.parse().map_err(|_| ERROR)?;
                if member_group != group { continue; }
                if state != "Z" && state != "X" { return Ok(None); }
                let start: u64 = fields.nth(16).ok_or(ERROR)?.parse().map_err(|_| ERROR)?;
                zombies.push((pid, start));
            }
            zombies.sort_unstable();
            Ok(Some(zombies))
        }
        let Some(first) = snapshot(group)? else { return Ok(true); };
        let Some(second) = snapshot(group)? else { return Ok(true); };
        // Once all members in the first inventory are zombies they cannot fork.
        // Reinventory catches a child forked just before its parent exited.
        if first != second { return Ok(true); }
        if first.is_empty() {
            if unsafe { libc::kill(-group, 0) } != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) { return Ok(false); }
            return Err(ERROR.into());
        }
        Ok(false)
    }

    #[cfg(target_os = "macos")]
    fn members_executable(group: i32) -> Result<bool, String> {
        fn snapshot(group: i32) -> Result<Vec<i32>, String> {
            let mut pids = vec![0i32; 32];
            loop {
                let count = unsafe { libc::proc_listpgrppids(group, pids.as_mut_ptr().cast(), std::mem::size_of_val(pids.as_slice()) as i32) };
                if count <= 0 {
                    if unsafe { libc::kill(-group, 0) } != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) { return Ok(Vec::new()); }
                    return Err(ERROR.into());
                }
                if count as usize >= pids.len() { pids.resize(pids.len() * 2, 0); continue; }
                pids.truncate(count as usize);
                pids.sort_unstable();
                return Ok(pids);
            }
        }
        let first = snapshot(group)?;
        if first.is_empty() { return Ok(false); }
        for &pid in &first {
            let mut info: libc::proc_bsdshortinfo = unsafe { std::mem::zeroed() };
            let size = std::mem::size_of_val(&info) as i32;
            // Nonzero arg explicitly includes zombies in Darwin's lookup;
            // the default proc_pidinfo arg=0 excludes them with ESRCH.
            if unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDT_SHORTBSDINFO, 1, (&mut info as *mut libc::proc_bsdshortinfo).cast(), size) } != size {
                if unsafe { libc::kill(pid, 0) } != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) { continue; }
                return Err(ERROR.into());
            }
            if info.pbsi_pgid == group as u32 && info.pbsi_status != libc::SZOMB { return Ok(true); }
        }
        let second = snapshot(group)?;
        Ok(!second.is_empty() && first != second)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn members_executable(_group: i32) -> Result<bool, String> { Err("process_ownership_unsupported".into()) }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{os::unix::fs::{symlink, FileExt, PermissionsExt}, process::Stdio, time::Duration};
        use tokio::process::Command;

        fn setup() -> (tempfile::TempDir, std::path::PathBuf, String) {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("ownership");
            (temp, root, uuid::Uuid::new_v4().to_string())
        }

        async fn eventually_stopped(root: &Path, binding: &str) {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if observe(root, binding).unwrap() == Observation::Stopped { break; }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await.expect("guardian left executable group members alive");
        }

        #[test]
        fn unfinished_reservations_and_partial_files_never_prove_no_execution() {
            let (_temp, root, binding) = setup();
            assert_eq!(observe(&root, &binding).unwrap(), Observation::Unavailable);
            let journal = OwnershipJournal::reserve(&root, &binding).unwrap();
            let path = root.join(format!("{binding}.ownership"));
            assert_eq!(root.metadata().unwrap().permissions().mode() & 0o777, 0o700);
            assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(observe(&root, &binding).unwrap(), Observation::Unavailable);
            assert!(OwnershipJournal::reserve(&root, &binding).is_err());
            let mut command = Command::new("/bin/sleep");
            command.arg("30");
            let pending_spawn = journal.prepare(&mut command).unwrap();
            assert_eq!(observe(&root, &binding).unwrap(), Observation::Unavailable);
            drop(pending_spawn);
            drop(command);
            drop(journal);
            assert_eq!(observe(&root, &binding).unwrap(), Observation::NotStarted);
            let complete = std::fs::read(&path).unwrap();
            for length in 0..SIZE {
                std::fs::write(&path, &complete[..length]).unwrap();
                assert_eq!(observe(&root, &binding).unwrap(), Observation::Unavailable, "partial length {length}");
            }
        }

        #[tokio::test]
        async fn committed_exec_failure_is_never_reported_not_started() {
            let (_temp, root, binding) = setup();
            let journal = OwnershipJournal::reserve(&root, &binding).unwrap();
            let mut command = Command::new("/dgos-no-such-native-executable");
            let mut life = journal.prepare(&mut command).unwrap();
            assert!(command.spawn().is_err());
            assert_ne!(observe(&root, &binding).unwrap(), Observation::NotStarted);
            life.stop();
            life.spawned();
            eventually_stopped(&root, &binding).await;
        }

        #[tokio::test]
        async fn eof_kills_term_resistant_descendants_without_waiting_for_the_leader() {
            let (_temp, root, binding) = setup();
            let journal = OwnershipJournal::reserve(&root, &binding).unwrap();
            let receipt = root.join("native-executed");
            let mut command = Command::new("/bin/sh");
            command.arg("-c").arg("trap '' TERM; (trap '' TERM; exec /bin/sleep 30) & printf '%s %s' \"$$\" \"$!\" > \"$1\"; exec /bin/sleep 30").arg("fixture").arg(&receipt)
                .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
            let mut life = journal.prepare(&mut command).unwrap();
            let mut child = command.spawn().unwrap();
            life.spawned();
            tokio::time::timeout(Duration::from_secs(3), async {
                while !receipt.exists() { tokio::time::sleep(Duration::from_millis(10)).await; }
            }).await.unwrap();
            let ids: Vec<i32> = std::fs::read_to_string(&receipt).unwrap().split_whitespace().map(|pid| pid.parse().unwrap()).collect();
            assert_eq!(observe(&root, &binding).unwrap(), Observation::Running);
            assert!(ids.iter().all(|pid| unsafe { libc::getpgid(*pid) } == child.id().unwrap() as i32));
            let bytes = std::fs::read(root.join(format!("{binding}.ownership"))).unwrap();
            assert_eq!(bytes[40], 1, "native exec preceded durable start marker");
            assert_eq!(i32::from_ne_bytes(bytes[44..48].try_into().unwrap()), child.id().unwrap() as i32);
            life.stop();
            eventually_stopped(&root, &binding).await;
            tokio::time::timeout(Duration::from_secs(3), child.wait()).await.unwrap().unwrap();
            journal.record_stopped().unwrap();
            assert_eq!(observe(&root, &binding).unwrap(), Observation::Stopped);
        }

        #[tokio::test]
        async fn zombie_only_group_is_inactive_before_the_owned_leader_is_reaped() {
            let mut child = Command::new("/bin/sh").arg("-c").arg("exit 0").process_group(0).spawn().unwrap();
            let pid = child.id().unwrap() as i32;
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    let mut status: libc::siginfo_t = unsafe { std::mem::zeroed() };
                    assert_eq!(unsafe { libc::waitid(libc::P_PID, pid as libc::id_t, &mut status, libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) }, 0);
                    if unsafe { status.si_pid() } == pid { break; }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await.unwrap();
            assert!(!group_executable(pid).unwrap());
            child.wait().await.unwrap();
        }

        #[tokio::test]
        async fn observation_never_signals_unavailable_or_unrelated_groups() {
            let (temp, root, binding) = setup();
            let journal = OwnershipJournal::reserve(&root, &binding).unwrap();
            drop(journal);
            let mut other = Command::new("/bin/sleep").arg("30").process_group(0).spawn().unwrap();
            let path = root.join(format!("{binding}.ownership"));
            let file = std::fs::OpenOptions::new().read(true).write(true).open(&path).unwrap();
            file.write_all_at(&(other.id().unwrap() as i32).to_ne_bytes(), GROUP_OFFSET as u64).unwrap();
            file.write_all_at(&[1], STATE_OFFSET as u64).unwrap();
            file.sync_all().unwrap();
            assert_eq!(observe(&root, &binding).unwrap(), Observation::Running);
            let mut identity_byte = [0u8];
            file.read_exact_at(&mut identity_byte, 8).unwrap();
            file.write_all_at(&[identity_byte[0] ^ 1], 8).unwrap();
            assert_eq!(observe(&root, &binding).unwrap(), Observation::Unavailable);
            file.write_all_at(&identity_byte, 8).unwrap();
            file.read_exact_at(&mut identity_byte, 24).unwrap();
            file.write_all_at(&[identity_byte[0] ^ 1], 24).unwrap();
            assert_eq!(observe(&root, &binding).unwrap(), Observation::Stopped);
            assert!(other.try_wait().unwrap().is_none());
            let absent = uuid::Uuid::new_v4().to_string();
            assert_eq!(observe(&root, &absent).unwrap(), Observation::Unavailable);
            symlink(&path, root.join(format!("{absent}.ownership"))).unwrap();
            assert_eq!(observe(&root, &absent).unwrap(), Observation::Unavailable);
            let alias = temp.path().join("alias");
            symlink(&root, &alias).unwrap();
            assert!(OwnershipJournal::reserve(&alias, &uuid::Uuid::new_v4().to_string()).is_err());
            assert!(other.try_wait().unwrap().is_none());
            other.kill().await.unwrap();
            other.wait().await.unwrap();
        }

        #[tokio::test]
        async fn guardian_reclaims_group_after_host_loss_on_both_sides_of_exec() {
            const ROOT: &str = "DGOS_OWNERSHIP_TEST_HOST_ROOT";
            const BINDING: &str = "DGOS_OWNERSHIP_TEST_BINDING";
            const BOUNDARY: &str = "DGOS_OWNERSHIP_TEST_PRE_EXEC";
            if let Some(root) = std::env::var_os(ROOT) {
                let root = std::path::PathBuf::from(root);
                let binding = std::env::var(BINDING).unwrap();
                let journal = OwnershipJournal::reserve(&root, &binding).unwrap();
                let mut command = Command::new("/bin/sh");
                command.arg("-c").arg("trap '' TERM; (trap '' TERM; exec /bin/sleep 30) & printf executed > \"$1\"; exec /bin/sleep 30").arg("fixture").arg(root.join("executed"))
                    .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
                let mut life = journal.prepare(&mut command).unwrap();
                if std::env::var_os(BOUNDARY).is_some() {
                    unsafe { command.pre_exec(|| { loop { libc::pause(); } }); }
                }
                let mut child = command.spawn().unwrap();
                life.spawned();
                // The parent test kills this entire HOST, not a transport task.
                child.wait().await.unwrap();
                panic!("native fixture exited before its host was killed");
            }
            for boundary in [false, true] {
                let (_temp, root, binding) = setup();
                let mut host = std::process::Command::new(std::env::current_exe().unwrap());
                host.arg("--exact").arg("process_ownership::unix::tests::guardian_reclaims_group_after_host_loss_on_both_sides_of_exec")
                    .arg("--nocapture").env(ROOT, &root).env(BINDING, &binding)
                    .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
                if boundary { host.env(BOUNDARY, "1"); } else { host.env_remove(BOUNDARY); }
                let mut host = host.spawn().unwrap();
                let ready = tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        let running = observe(&root, &binding).unwrap() == Observation::Running;
                        if running && (boundary || root.join("executed").exists()) { break; }
                        if let Some(status) = host.try_wait().unwrap() { panic!("fixture host exited: {status}"); }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }).await;
                // Cleanup happens even when the ready assertion fails.
                host.kill().unwrap();
                host.wait().unwrap();
                ready.unwrap();
                eventually_stopped(&root, &binding).await;
                assert_eq!(root.join("executed").exists(), !boundary);
            }
        }
    }
}

#[cfg(unix)]
pub(crate) use unix::{group_executable, observe, OwnershipJournal};

#[cfg(not(unix))]
pub(crate) struct OwnershipJournal;
#[cfg(not(unix))]
impl OwnershipJournal {
    pub(crate) fn reserve(_root: &Path, _binding_id: &str) -> Result<Arc<Self>, String> { Err("process_ownership_unsupported".into()) }
}
#[cfg(not(unix))]
pub(crate) fn observe(_root: &Path, _binding_id: &str) -> Result<Observation, String> { Ok(Observation::Unavailable) }
