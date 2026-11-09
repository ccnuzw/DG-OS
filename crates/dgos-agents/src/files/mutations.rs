//! Explorer operations are bounded by 10,000 entries, 64 levels and 30 seconds
//! per invocation, including streamed hashing/copying. There is no per-file size
//! limit. Admission serializes host writers; external writers cannot participate
//! in a portable filesystem compare-and-swap. Rechecks narrow those races, and
//! any failure after a side effect is unknown, with no rollback or retry.
use super::*;
use std::{ffi::OsStr, io::{Seek, SeekFrom, Write}};
use tokio::sync::OwnedMutexGuard;
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectEntryInput {
    pub project_id: String, #[serde(default)] pub task_id: Option<String>, pub path: String,
}
#[derive(Debug, Serialize)]
pub struct InspectedEntry { pub path: String, pub kind: EntryKind, pub version: String }
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MutationAction { Copy, Rename, Delete }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MutateEntryInput {
    pub project_id: String, #[serde(default)] pub task_id: Option<String>, pub path: String,
    pub expected_version: String, pub action: MutationAction, pub destination: Option<String>, pub trust_directory: bool,
}

fn checked_path(path: &str) -> Result<(), String> {
    relative(path, false)?;
    if sensitive(path) { return Err("file_sensitive".into()); }
    if Path::new(path).components().count() > 64 { return Err("file_access_denied".into()); }
    Ok(())
}
pub(super) fn validate_input(input: &MutateEntryInput) -> Result<(), String> {
    checked_path(&input.path)?;
    if input.expected_version.len() != 64 || !input.expected_version.bytes().all(|c| c.is_ascii_hexdigit()) { return Err("invalid_request".into()); }
    match (input.action, input.destination.as_deref()) {
        (MutationAction::Delete, None) => {},
        (MutationAction::Copy | MutationAction::Rename, Some(destination)) => {
            checked_path(destination)?;
            if Path::new(destination).starts_with(&input.path) { return Err("file_access_denied".into()); }
        },
        _ => return Err("invalid_request".into()),
    }
    Ok(())
}
struct Budget { start: Instant, entries: usize, lease: CancellationToken }
impl Budget {
    fn new(lease: &CancellationToken) -> Self { Self { start: Instant::now(), entries: 0, lease: lease.clone() } }
    fn check(&self) -> Result<(), String> {
        if self.lease.is_cancelled() { return Err("session_invalid".into()); }
        if self.start.elapsed() > Duration::from_secs(30) { return Err("file_access_denied".into()); }
        Ok(())
    }
    fn visit(&mut self, depth: usize) -> Result<(), String> {
        self.check()?;
        self.entries += 1;
        if self.entries > ENTRY_LIMIT || depth > 64 { return Err("file_access_denied".into()); }
        Ok(())
    }
}
fn identity(metadata: &cap_std::fs::Metadata) -> String {
    #[cfg(unix)] {
        use cap_std::fs::MetadataExt;
        format!("{}:{}", metadata.dev(), metadata.ino())
    }
    #[cfg(not(unix))] { format!("{:?}", metadata.created()) }
}
fn stamp(metadata: &cap_std::fs::Metadata) -> String {
    #[cfg(unix)] {
        use cap_std::fs::MetadataExt;
        format!("{}:{}:{}:{}:{}:{}:{}:{}:{}", metadata.dev(), metadata.ino(), metadata.len(), metadata.mode(), metadata.nlink(), metadata.mtime(), metadata.mtime_nsec(), metadata.ctime(), metadata.ctime_nsec())
    }
    #[cfg(not(unix))] { format!("{}:{}:{:?}", identity(metadata), metadata.len(), metadata.modified()) }
}
fn regular(parent: &Dir, name: &OsStr) -> Result<cap_std::fs::File, String> {
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)] { use cap_std::fs::OpenOptionsExt; options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK); }
    let file = parent.open_with(name, &options).map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() { return Err("file_access_denied".into()); }
    #[cfg(unix)] { use cap_std::fs::MetadataExt; if metadata.nlink() != 1 { return Err("file_access_denied".into()); } }
    Ok(file)
}
fn mutation_root(project: &Project) -> Result<Dir, String> {
    if std::fs::canonicalize(&project.path).map_err(io_error)? != Path::new(&project.path) { return Err("workspace_changed".into()); }
    open_root(project)
}
fn directory(parent: &Dir, name: &OsStr) -> Result<Dir, String> {
    #[cfg(unix)] {
        use cap_std::fs::OpenOptionsExt;
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
        Ok(Dir::from_std_file(parent.open_with(name, &options).map_err(io_error)?.into_std()))
    }
    #[cfg(not(unix))] {
        if parent.symlink_metadata(name).map_err(io_error)?.file_type().is_symlink() { return Err("file_access_denied".into()); }
        parent.open_dir(name).map_err(io_error)
    }
}
fn hash_file(file: &mut cap_std::fs::File, budget: &Budget) -> Result<[u8; 32], String> {
    let before = stamp(&file.metadata().map_err(io_error)?);
    file.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        budget.check()?;
        let n = file.read(&mut buffer).map_err(io_error)?;
        if n == 0 { break; }
        hash.update(&buffer[..n]);
    }
    if before != stamp(&file.metadata().map_err(io_error)?) { return Err("file_changed".into()); }
    Ok(hash.finalize().into())
}
struct Node {
    name: String, path: String, metadata: cap_std::fs::Metadata, digest: [u8; 32],
    body: Body,
}
enum Body { File, Directory(Dir, Vec<Node>) }
impl Node {
    fn kind(&self) -> EntryKind { match &self.body { Body::File => EntryKind::File, Body::Directory(..) => EntryKind::Directory } }
}
fn capture(parent: &Dir, name: &str, path: &str, budget: &mut Budget, depth: usize) -> Result<Node, String> {
    budget.visit(depth)?;
    checked_path(path)?;
    let metadata = parent.symlink_metadata(name).map_err(io_error)?;
    if metadata.file_type().is_symlink() { return Err("file_access_denied".into()); }
    let (body, digest) = if metadata.is_file() {
        let mut file = regular(parent, OsStr::new(name))?;
        if stamp(&metadata) != stamp(&file.metadata().map_err(io_error)?) { return Err("file_changed".into()); }
        let digest = hash_file(&mut file, budget)?;
        (Body::File, digest)
    } else if metadata.is_dir() {
        let dir = directory(parent, OsStr::new(name))?;
        if stamp(&metadata) != stamp(&dir.dir_metadata().map_err(io_error)?) { return Err("file_changed".into()); }
        let mut names = Vec::new();
        for entry in dir.entries().map_err(io_error)? {
            budget.check()?;
            if budget.entries + names.len() >= ENTRY_LIMIT { return Err("file_access_denied".into()); }
            let entry = entry.map_err(io_error)?;
            let name = entry.file_name().into_string().map_err(|_| "file_access_denied")?;
            names.push(name);
        }
        names.sort();
        let mut children = Vec::with_capacity(names.len());
        let mut hash = Sha256::new();
        for child in names {
            let node = capture(&dir, &child, &format!("{path}/{child}"), budget, depth + 1)?;
            hash.update((child.len() as u64).to_le_bytes()); hash.update(child.as_bytes());
            hash.update(stamp(&node.metadata).as_bytes()); hash.update(node.digest);
            children.push(node);
        }
        if stamp(&metadata) != stamp(&dir.dir_metadata().map_err(io_error)?) { return Err("file_changed".into()); }
        (Body::Directory(dir, children), hash.finalize().into())
    } else { return Err("file_access_denied".into()); };
    if stamp(&metadata) != stamp(&parent.symlink_metadata(name).map_err(io_error)?) { return Err("file_changed".into()); }
    Ok(Node { name: name.into(), path: path.into(), metadata, digest, body })
}
fn version(node: &Node) -> String {
    let mut hash = Sha256::new(); hash.update(stamp(&node.metadata)); hash.update(node.digest);
    format!("{:x}", hash.finalize())
}
fn snapshot(project: &Project, path: &str, budget: &mut Budget) -> Result<(Dir, Node), String> {
    checked_path(path)?;
    let root = mutation_root(project)?;
    let parent = creation_parent(&root, Path::new(path))?;
    let name = Path::new(path).file_name().and_then(OsStr::to_str).ok_or("file_access_denied")?;
    let node = capture(&parent, name, path, budget, Path::new(path).components().count())?;
    verify_live(project, path, &parent)?;
    verify_tree(&parent, &node, budget)?;
    Ok((parent, node))
}
fn verify_live(project: &Project, path: &str, held: &Dir) -> Result<(), String> {
    let live = creation_parent(&mutation_root(project)?, Path::new(path))?;
    if identity(&live.dir_metadata().map_err(io_error)?) != identity(&held.dir_metadata().map_err(io_error)?) { return Err("workspace_changed".into()); }
    Ok(())
}
fn verify_tree(parent: &Dir, node: &Node, budget: &Budget) -> Result<(), String> {
    budget.check()?;
    if stamp(&parent.symlink_metadata(&node.name).map_err(io_error)?) != stamp(&node.metadata) { return Err("file_changed".into()); }
    if let Body::Directory(dir, children) = &node.body {
        for child in children { verify_tree(dir, child, budget)?; }
    }
    Ok(())
}
pub(super) fn inspect_at(project: &Project, path: &str, lease: &CancellationToken) -> Result<InspectedEntry, String> {
    let (_, node) = snapshot(project, path, &mut Budget::new(lease))?;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    Ok(InspectedEntry { path: path.into(), kind: node.kind(), version: version(&node) })
}

// Atomic no-replace is required. Never emulate it with exists()+rename().
fn rename_exclusive(from: &Dir, name: &str, to: &Dir, destination: &str) -> Result<(), String> {
    #[cfg(any(target_os = "macos", target_os = "linux"))] {
        use std::{ffi::CString, os::fd::AsRawFd};
        let name = CString::new(name).map_err(|_| "file_access_denied")?;
        let destination = CString::new(destination).map_err(|_| "file_access_denied")?;
        #[cfg(target_os = "macos")]
        let result = unsafe { libc::renameatx_np(from.as_raw_fd(), name.as_ptr(), to.as_raw_fd(), destination.as_ptr(), libc::RENAME_EXCL) };
        #[cfg(target_os = "linux")]
        let result = unsafe { libc::syscall(libc::SYS_renameat2, from.as_raw_fd(), name.as_ptr(), to.as_raw_fd(), destination.as_ptr(), libc::RENAME_NOREPLACE) as libc::c_int };
        if result != 0 { return Err(creation_error(std::io::Error::last_os_error())); }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))] {
        let _ = (from, name, to, destination);
        Err("file_access_denied".into())
    }
}
fn check_absent(parent: &Dir, name: &str) -> Result<(), String> {
    match parent.symlink_metadata(name) {
        Ok(_) => Err("file_exists".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(error)),
    }
}
fn copy_node(project: &Project, source_parent: &Dir, node: &mut Node, destination_parent: &Dir, destination: &str, destination_path: &str, budget: &Budget, changed: &mut bool) -> Result<(), String> {
    budget.check()?;
    verify_live(project, &node.path, source_parent)?;
    verify_live(project, destination_path, destination_parent)?;
    verify_tree(source_parent, node, budget)?;
    match &mut node.body {
        Body::File => {
            let mut source = regular(source_parent, OsStr::new(&node.name))?;
            if stamp(&source.metadata().map_err(io_error)?) != stamp(&node.metadata) { return Err("file_changed".into()); }
            let mut options = cap_std::fs::OpenOptions::new(); options.write(true).create_new(true);
            #[cfg(unix)] { use cap_std::fs::OpenOptionsExt; options.mode(0o600).custom_flags(libc::O_NOFOLLOW); }
            let mut target = destination_parent.open_with(destination, &options).map_err(creation_error)?;
            *changed = true;
            let created = target.metadata().map_err(io_error)?;
            source.seek(SeekFrom::Start(0)).map_err(io_error)?;
            let mut hash = Sha256::new(); let mut buffer = [0u8; 64 * 1024];
            loop {
                budget.check()?;
                verify_live(project, destination_path, destination_parent)?;
                let n = source.read(&mut buffer).map_err(io_error)?;
                if n == 0 { break; }
                hash.update(&buffer[..n]); target.write_all(&buffer[..n]).map_err(io_error)?;
            }
            let digest: [u8; 32] = hash.finalize().into();
            if digest != node.digest || stamp(&source.metadata().map_err(io_error)?) != stamp(&node.metadata) { return Err("file_changed".into()); }
            target.set_permissions(node.metadata.permissions()).map_err(io_error)?;
            target.sync_all().map_err(io_error)?;
            if identity(&created) != identity(&destination_parent.symlink_metadata(destination).map_err(io_error)?) { return Err("file_changed".into()); }
        },
        Body::Directory(source, children) => {
            let mut builder = cap_std::fs::DirBuilder::new();
            // Restrict access at mkdir, not with a chmod after publication.
            #[cfg(unix)] { use cap_std::fs::DirBuilderExt; builder.mode(0o700); }
            destination_parent.create_dir_with(destination, &builder).map_err(creation_error)?;
            *changed = true;
            let target = directory(destination_parent, OsStr::new(destination))?;
            let created = target.dir_metadata().map_err(io_error)?;
            for child in children {
                let name = child.name.clone();
                copy_node(project, source, child, &target, &name, &format!("{destination_path}/{name}"), budget, changed)?;
            }
            target.set_permissions(".", node.metadata.permissions()).map_err(io_error)?;
            if identity(&created) != identity(&destination_parent.symlink_metadata(destination).map_err(io_error)?) { return Err("file_changed".into()); }
        },
    }
    verify_live(project, &node.path, source_parent)?;
    verify_live(project, destination_path, destination_parent)?;
    verify_tree(source_parent, node, budget)
}
fn delete_node(project: &Project, parent: &Dir, node: &mut Node, budget: &Budget, changed: &mut bool) -> Result<(), String> {
    budget.check()?;
    verify_live(project, &node.path, parent)?;
    if stamp(&parent.symlink_metadata(&node.name).map_err(io_error)?) != stamp(&node.metadata) { return Err("file_changed".into()); }
    match &mut node.body {
        Body::File => {
            let mut file = regular(parent, OsStr::new(&node.name))?;
            if hash_file(&mut file, budget)? != node.digest { return Err("file_changed".into()); }
            verify_live(project, &node.path, parent)?;
            if stamp(&parent.symlink_metadata(&node.name).map_err(io_error)?) != stamp(&node.metadata) { return Err("file_changed".into()); }
            parent.remove_file(&node.name).map_err(io_error)?;
            *changed = true;
        },
        Body::Directory(dir, children) => {
            for child in children { delete_node(project, dir, child, budget, changed)?; }
            budget.check()?;
            verify_live(project, &node.path, parent)?;
            // Children changed directory timestamps. Match identity, then rmdir
            // only: an external addition is never recursively removed.
            if identity(&parent.symlink_metadata(&node.name).map_err(io_error)?) != identity(&node.metadata) { return Err("file_changed".into()); }
            parent.remove_dir(&node.name).map_err(io_error)?;
            *changed = true;
        },
    }
    Ok(())
}
fn same_contents(source: &Node, target: &Node) -> bool {
    match (&source.body, &target.body) {
        (Body::File, Body::File) => source.digest == target.digest,
        (Body::Directory(_, a), Body::Directory(_, b)) => a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.name == b.name && same_contents(a, b)),
        _ => false,
    }
}
fn deletion_preflight(parent: &Dir, node: &Node, budget: &Budget) -> Result<(), String> {
    budget.check()?;
    #[cfg(unix)] {
        use std::os::fd::AsRawFd;
        let dot = b".\0";
        if unsafe { libc::faccessat(parent.as_raw_fd(), dot.as_ptr().cast(), libc::W_OK | libc::X_OK, libc::AT_EACCESS) } != 0 { return Err("file_access_denied".into()); }
    }
    if let Body::Directory(dir, children) = &node.body {
        for child in children { deletion_preflight(dir, child, budget)?; }
    }
    Ok(())
}
fn contains_directory(node: &Node, candidate: &str) -> bool {
    match &node.body {
        Body::Directory(_, children) => identity(&node.metadata) == candidate || children.iter().any(|child| contains_directory(child, candidate)),
        Body::File => false,
    }
}
fn mutation_result(result: Result<CreatedEntry, String>, changed: bool) -> Result<CreatedEntry, String> {
    if changed { result.map_err(|_| "file_mutation_unknown".into()) } else { result }
}
pub(super) fn mutate_at(project: &Project, input: MutateEntryInput, lease: &CancellationToken) -> Result<CreatedEntry, String> {
    validate_input(&input)?;
    let mut budget = Budget::new(lease);
    let (parent, mut node) = snapshot(project, &input.path, &mut budget)?;
    if version(&node) != input.expected_version { return Err("file_changed".into()); }
    let destination = input.destination.as_deref().map(|path| {
        let parent = creation_parent(&mutation_root(project)?, Path::new(path))?;
        let name = Path::new(path).file_name().and_then(OsStr::to_str).ok_or("file_access_denied")?.to_owned();
        // Compare descriptor identities too: case-insensitive aliases must not
        // place the destination inside the source tree.
        if contains_directory(&node, &identity(&parent.dir_metadata().map_err(io_error)?)) { return Err("file_access_denied".into()); }
        check_absent(&parent, &name)?;
        verify_live(project, path, &parent)?;
        Ok::<_, String>((parent, name, path))
    }).transpose()?;
    verify_live(project, &input.path, &parent)?;
    verify_tree(&parent, &node, &budget)?;
    budget.check()?;
    if input.action == MutationAction::Delete { deletion_preflight(&parent, &node, &budget)?; }
    let mut changed = false;
    let result = (|| {
        match input.action {
            MutationAction::Copy => {
                let (to, name, path) = destination.as_ref().ok_or("invalid_request")?;
                copy_node(project, &parent, &mut node, to, name, path, &budget, &mut changed)?;
                // Observe the complete published tree, including earlier copied children.
                budget.entries = 0;
                let observed = capture(to, name, path, &mut budget, Path::new(path).components().count())?;
                if !same_contents(&node, &observed) { return Err("file_changed".into()); }
                verify_tree(to, &observed, &budget)?;
            },
            MutationAction::Rename => {
                let (to, name, path) = destination.as_ref().ok_or("invalid_request")?;
                verify_live(project, path, to)?;
                rename_exclusive(&parent, &node.name, to, name)?;
                changed = true;
                verify_live(project, path, to)?;
                if identity(&to.symlink_metadata(name).map_err(io_error)?) != identity(&node.metadata) { return Err("file_changed".into()); }
                budget.entries = 0;
                let observed = capture(to, name, path, &mut budget, Path::new(path).components().count())?;
                if !same_contents(&node, &observed) { return Err("file_changed".into()); }
                verify_tree(to, &observed, &budget)?;
                check_absent(&parent, &node.name)?;
            },
            MutationAction::Delete => {
                delete_node(project, &parent, &mut node, &budget, &mut changed)?;
                check_absent(&parent, &node.name)?;
            },
        }
        budget.check()?;
        verify_live(project, &input.path, &parent)?;
        Ok(CreatedEntry { path: input.destination.clone().unwrap_or_else(|| input.path.clone()), kind: node.kind() })
    })();
    mutation_result(result, changed)
}

/// Retains admission and descriptor pins until the host has dispatched the OS
/// opener. The renderer never supplies an absolute native path.
pub struct EntryDirectoryAdmission {
    pub path: String,
    project: Project, entry: String, parent: Dir, directory: Dir, directory_path: String,
    _admission: OwnedMutexGuard<()>,
}
impl EntryDirectoryAdmission {
    pub fn validate(&self, lease: &CancellationToken) -> Result<(), String> {
        if lease.is_cancelled() { return Err("session_invalid".into()); }
        verify_live(&self.project, &self.entry, &self.parent)?;
        let root = mutation_root(&self.project)?;
        let current = if self.directory_path.is_empty() { root } else { directory(&creation_parent(&root, Path::new(&self.directory_path))?, Path::new(&self.directory_path).file_name().ok_or("file_access_denied")?)? };
        if identity(&current.dir_metadata().map_err(io_error)?) != identity(&self.directory.dir_metadata().map_err(io_error)?) { return Err("workspace_changed".into()); }
        Ok(())
    }
}
pub(super) fn directory_admission(project: Project, entry: String, admission: OwnedMutexGuard<()>, lease: &CancellationToken) -> Result<EntryDirectoryAdmission, String> {
    checked_path(&entry)?;
    let root = mutation_root(&project)?;
    let parent = creation_parent(&root, Path::new(&entry))?;
    let name = Path::new(&entry).file_name().ok_or("file_access_denied")?;
    let metadata = parent.symlink_metadata(name).map_err(io_error)?;
    if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) { return Err("file_access_denied".into()); }
    let (directory_path, directory) = if metadata.is_dir() { (entry.clone(), directory(&parent, name)?) } else {
        (Path::new(&entry).parent().and_then(Path::to_str).unwrap_or("").to_owned(), parent.try_clone().map_err(io_error)?)
    };
    let path = Path::new(&project.path).join(&directory_path).to_str().ok_or("file_access_denied")?.to_owned();
    let held = EntryDirectoryAdmission { path, project, entry, parent, directory, directory_path, _admission: admission };
    held.validate(lease)?;
    Ok(held)
}

#[cfg(test)]
mod tests;
