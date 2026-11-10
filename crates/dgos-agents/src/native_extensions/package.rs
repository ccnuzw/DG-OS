//! Native packages retain all ordinary resources and executable bits, but execute nothing.
use super::Error;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{collections::{BTreeMap, BTreeSet}, io::{Cursor, Read}};
pub const ARCHIVE_LIMIT: usize = 8 * 1024 * 1024;
pub const TREE_LIMIT: usize = 16 * 1024 * 1024;
pub const FILE_LIMIT: usize = 4 * 1024 * 1024;
pub const ENTRY_LIMIT: usize = 1024;
pub const DEPTH_LIMIT: usize = 8;
#[derive(Clone)]
pub struct PackageFile { pub path: String, pub bytes: Vec<u8>, pub executable: bool }
#[derive(Clone, Debug, Serialize)]
pub struct ResourceFile { pub path: String, pub hash: String, pub bytes: usize, pub executable: bool }
#[derive(Clone, Debug, Serialize)]
pub struct Metadata {
    pub name: String, pub description: String, pub license: Option<String>,
    pub dependencies: super::dependencies::Declarations, pub dynamic_commands: bool,
}
#[derive(Clone)]
pub struct NativeSkillPackage {
    pub(super) files: BTreeMap<String, PackageFile>,
    pub(super) hash: String,
    pub(super) metadata: Metadata,
    pub(super) execution_bits: BTreeMap<String, u32>,
}
pub struct PackageIndex {
    pub packages: BTreeMap<String, NativeSkillPackage>,
    pub rejected: Vec<(String, String)>,
}
/// App-private cache storage. A host-derived path is required; directory-fd
/// traversal and atomic replacement use the same protections as native journals.
#[cfg(unix)]
pub struct HostSkillCache(super::paths::Directory);
#[cfg(unix)]
impl HostSkillCache {
    pub fn open(path: &std::path::Path) -> Result<Self, Error> {
        let directory = super::paths::Directory::ensure(path)?;
        directory.private()?;
        Ok(Self(directory))
    }
    pub fn read(&self) -> Result<Option<Vec<u8>>, Error> {
        self.0.read_bytes("sources.json", ARCHIVE_LIMIT).map(|value| value.map(|(bytes, _)| bytes))
    }
    pub fn write(&self, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() > ARCHIVE_LIMIT { return Err(Error::Limit); }
        self.0.replace_journal("sources.json", bytes)
    }
}
#[cfg(unix)]
impl NativeSkillPackage {
    /// Only a trusted host file-picker/source authorization may supply this path.
    /// Symlinks, hardlinks, special files and concurrent tree changes are rejected.
    pub fn from_directory(path: &std::path::Path) -> Result<Self, Error> {
        let root = super::paths::Directory::open(path)?;
        let (snapshot, files) = super::resource_tree::scan(&root)?;
        for path in snapshot.directories.keys() { loose_path(path)?; }
        let mut package = Self::from_files(files)?;
        package.execution_bits = snapshot.files.iter().map(|(path, entry)| (path.clone(), entry.mode & 0o111)).collect();
        package.rehash();
        if super::resource_tree::scan(&root)?.0 != snapshot || super::paths::Directory::open(path)?.identity()? != snapshot.identity { return Err(Error::Conflict); }
        Ok(package)
    }
}
pub(super) fn safe_path(path: &str) -> Result<(), Error> {
    if path.is_empty() || path.len() > 1024 || !path.is_ascii() || path.split('/').count() > DEPTH_LIMIT
        || path.split('/').any(|p| p.is_empty() || p == "." || p == ".." || p.len() > 255)
        || !path.bytes().all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b)) { return Err(Error::UnsafePath); }
    Ok(())
}
// Use ASCII-insensitive comparisons even on case-sensitive fixture volumes.
pub(super) fn loose_path(path: &str) -> Result<(), Error> {
    let reserved = [".claude-plugin", ".dgos-native-source.json", ".managed", ".system", "managed", "synced", "plugin.json", "managed-policy.json"];
    if path.split('/').any(|part| reserved.iter().any(|marker| part.eq_ignore_ascii_case(marker)))
        || path.split('/').zip(path.split('/').skip(1)).any(|(parent, name)| parent.eq_ignore_ascii_case("hooks") && name.eq_ignore_ascii_case("hooks.json"))
    { return Err(Error::Unsupported); }
    Ok(())
}
pub(super) fn slug(s: &str) -> Result<(), Error> {
    safe_path(s)?;
    if s.contains('/') || s.starts_with('.') || s.len() > 128 { return Err(Error::UnsafePath); }
    Ok(())
}
fn references(files: &BTreeMap<String, PackageFile>) -> Result<(), Error> {
    use pulldown_cmark::{Event, Tag};
    for file in files.values().filter(|f| f.path.ends_with(".md")) {
        let Ok(text) = std::str::from_utf8(&file.bytes) else { continue; };
        for event in pulldown_cmark::Parser::new(text) {
            let destination = match event { Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => dest_url, _ => continue };
            let raw = destination.split(['#', '?']).next().unwrap_or("");
            if raw.is_empty() || raw.starts_with("https://") || raw.starts_with("http://") || raw.starts_with("mailto:") { continue; }
            if raw.starts_with('/') || raw.contains(['\\', ':', '%', '\0']) { return Err(Error::UnsafePath); }
            let mut parts: Vec<&str> = file.path.split('/').collect(); parts.pop();
            for part in raw.split('/') { match part { "." | "" => {}, ".." => { if parts.pop().is_none() { return Err(Error::UnsafePath); } }, other => parts.push(other) } }
        }
    }
    Ok(())
}
pub(super) fn metadata(bytes: &[u8]) -> Result<Metadata, Error> {
    if bytes.len() > 256 * 1024 { return Err(Error::Limit); }
    let text = std::str::from_utf8(bytes).map_err(|_| Error::InvalidFormat)?;
    let mut lines = text.lines();
    if lines.next().map(|l| l.trim_start_matches('\u{feff}')) != Some("---") { return Err(Error::InvalidFormat); }
    let mut yaml = String::new(); let mut closed = false;
    for line in lines { if line == "---" { closed = true; break; } yaml.push_str(line); yaml.push('\n'); }
    if !closed { return Err(Error::InvalidFormat); }
    let value: serde_yaml::Value = serde_yaml::from_str(&yaml).map_err(|_| Error::InvalidFormat)?;
    let get = |key: &str| value.get(key).and_then(serde_yaml::Value::as_str).map(str::to_owned);
    let name = get("name").ok_or(Error::InvalidFormat)?;
    slug(&name)?;
    let description = get("description").unwrap_or_default();
    if description.len() > 4096 { return Err(Error::Limit); }
    let dependencies = super::dependencies::Declarations::parse(&value)?;
    Ok(Metadata { name, description, license: get("license"), dependencies, dynamic_commands: text.contains("!`") || text.contains("$ARGUMENTS") })
}
impl NativeSkillPackage {
    pub fn from_files(files: Vec<PackageFile>) -> Result<Self, Error> {
        if files.len() > ENTRY_LIMIT { return Err(Error::Limit); }
        let mut output = BTreeMap::new(); let mut seen = BTreeSet::new(); let mut total = 0usize;
        for file in files {
            safe_path(&file.path)?;
            loose_path(&file.path)?;
            if !seen.insert(file.path.to_ascii_lowercase()) { return Err(Error::Conflict); }
            if file.bytes.len() > FILE_LIMIT { return Err(Error::Limit); }
            total = total.checked_add(file.bytes.len()).ok_or(Error::Limit)?;
            if total > TREE_LIMIT { return Err(Error::Limit); }
            output.insert(file.path.clone(), file);
        }
        // A file cannot also be a parent directory, including case aliases.
        let mut directories = BTreeMap::new();
        for path in output.keys() {
            let parts: Vec<_> = path.split('/').collect();
            for end in 1..parts.len() {
                let dir = parts[..end].join("/");
                if directories.insert(dir.to_ascii_lowercase(), dir.clone()).is_some_and(|old| old != dir) { return Err(Error::Conflict); }
            }
        }
        for path in output.keys() { let mut prefix = String::new(); for part in path.split('/').take(path.split('/').count()-1) { if !prefix.is_empty() { prefix.push('/'); } prefix.push_str(part); if seen.contains(&prefix.to_ascii_lowercase()) { return Err(Error::Conflict); } } }
        let metadata = metadata(&output.get("SKILL.md").ok_or(Error::InvalidFormat)?.bytes)?;
        references(&output)?;
        let execution_bits = output.values().map(|f| (f.path.clone(), if f.executable { 0o100 } else { 0 })).collect();
        let mut package = Self { files: output, hash: String::new(), metadata, execution_bits };
        package.rehash();
        Ok(package)
    }
    fn rehash(&mut self) {
        let mut hash = Sha256::new();
        for file in self.files.values() {
            hash.update(file.path.as_bytes()); hash.update([0]);
            hash.update(self.execution_bits.get(&file.path).copied().unwrap_or(0).to_le_bytes());
            hash.update((file.bytes.len() as u64).to_le_bytes()); hash.update(&file.bytes);
        }
        self.hash = format!("{:x}", hash.finalize());
    }
    fn archive_files(bytes: &[u8], reject_ownership: bool) -> Result<(Vec<PackageFile>, BTreeMap<String, u32>, Vec<String>), Error> {
        if bytes.len() > ARCHIVE_LIMIT { return Err(Error::Limit); }
        // No archive entry is materialized as a link. Legacy UNIX extra fields
        // may encode link targets even when the external mode looks regular.
        // The entire archive is validated before any candidate is selected.
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| Error::InvalidFormat)?;
        if zip.len() > ENTRY_LIMIT * 2 { return Err(Error::Limit); }
        let mut all = Vec::new(); let mut seen = BTreeSet::new(); let mut total = 0usize; let mut count = 0;
        let mut modes = BTreeMap::new();
        let mut archive_paths = BTreeMap::new();
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|_| Error::InvalidFormat)?;
            if entry.encrypted() || entry.is_symlink() { return Err(Error::UnsafePath); }
            if let Some(extra) = entry.extra_data() {
                let mut at = 0;
                while at < extra.len() {
                    if extra.len() - at < 4 { return Err(Error::InvalidFormat); }
                    let id = u16::from_le_bytes([extra[at], extra[at+1]]);
                    let size = u16::from_le_bytes([extra[at+2], extra[at+3]]) as usize;
                    at += 4;
                    if size > extra.len() - at { return Err(Error::InvalidFormat); }
                    if matches!(id, 0x000c | 0x000d | 0x756e) { return Err(Error::UnsafePath); }
                    at += size;
                }
            }
            let raw = entry.name().map_err(|_| Error::InvalidFormat)?.into_owned();
            let directory = entry.is_dir();
            if !directory && raw.ends_with('/') { return Err(Error::InvalidFormat); }
            let path = if directory { raw.strip_suffix('/').unwrap_or(&raw).to_owned() } else { raw };
            safe_path(&path)?;
            if reject_ownership { loose_path(&path)?; }
            if !seen.insert(path.to_ascii_lowercase()) { return Err(Error::Conflict); }
            archive_paths.insert(path.clone(), directory);
            let mode = entry.unix_mode().unwrap_or(0); let kind = mode & 0o170000;
            if kind != 0 && kind != if directory { 0o040000 } else { 0o100000 } { return Err(Error::UnsafePath); }
            if entry.size() > FILE_LIMIT as u64 || (directory && entry.size() != 0) { return Err(Error::Limit); }
            total = total.checked_add(entry.size() as usize).ok_or(Error::Limit)?;
            if total > TREE_LIMIT { return Err(Error::Limit); }
            if directory { continue; }
            count += 1; if count > ENTRY_LIMIT { return Err(Error::Limit); }
            let mut data = Vec::new(); (&mut entry).take(FILE_LIMIT as u64 + 1).read_to_end(&mut data).map_err(|_| Error::InvalidFormat)?;
            if data.len() > FILE_LIMIT || data.len() as u64 != entry.size() { return Err(Error::Limit); }
            modes.insert(path.clone(), mode & 0o111);
            all.push(PackageFile { path, bytes: data, executable: mode & 0o111 != 0 });
        }
        let mut directories = BTreeMap::new();
        for (path, is_directory) in &archive_paths {
            if *is_directory { directories.insert(path.to_ascii_lowercase(), path.as_str()); }
        }
        for path in archive_paths.keys() {
            for (at, _) in path.match_indices('/') {
                let ancestor = &path[..at];
                if archive_paths.get(ancestor) == Some(&false) { return Err(Error::Conflict); }
                if directories.insert(ancestor.to_ascii_lowercase(), ancestor).is_some_and(|old| old != ancestor) { return Err(Error::Conflict); }
            }
        }
        if archive_paths.iter().any(|(path, is_directory)| !is_directory && directories.contains_key(&path.to_ascii_lowercase())) { return Err(Error::Conflict); }
        Ok((all, modes, directories.into_values().map(str::to_owned).collect()))
    }
    pub fn from_zip(bytes: &[u8], selected_subpath: Option<&str>) -> Result<Self, Error> {
        if let Some(path) = selected_subpath { safe_path(path)?; }
        let (all, modes, _) = Self::archive_files(bytes, true)?;
        let wrapper = if !all.iter().any(|f| f.path == "SKILL.md") && all.iter().all(|f| f.path.contains('/')) { let first = all.first().and_then(|f| f.path.split('/').next()); first.filter(|p| all.iter().all(|f| f.path.starts_with(&format!("{p}/")))).map(str::to_owned) } else { None };
        let prefix = [wrapper.as_deref(), selected_subpath].into_iter().flatten().collect::<Vec<_>>().join("/");
        let mut selected_modes = BTreeMap::new();
        let files = all.into_iter().filter_map(|mut f| {
            let bits = modes.get(&f.path).copied().unwrap_or(0);
            if !prefix.is_empty() { f.path = f.path.strip_prefix(&format!("{prefix}/"))?.to_owned(); }
            selected_modes.insert(f.path.clone(), bits); Some(f)
        }).collect();
        let mut package = Self::from_files(files)?;
        package.execution_bits = selected_modes; package.rehash(); Ok(package)
    }
    /// Reuses the existing restricted source downloader; this adapter never fetches.
    pub fn from_github(source: &dgos_skills::ResolvedGithubSource) -> Result<Self, Error> {
        Self::from_zip(&source.archive, source.source.skill_subpath.as_deref())
    }
    pub fn content_hash(&self) -> &str { &self.hash }
    pub fn summary(&self) -> Vec<ResourceFile> { self.files.values().map(|f| ResourceFile { path: f.path.clone(), hash: format!("{:x}", Sha256::digest(&f.bytes)), bytes: f.bytes.len(), executable: f.executable }).collect() }
    pub fn metadata(&self) -> &Metadata { &self.metadata }
    pub fn resource_counts(&self) -> (usize, usize, usize) {
        (self.files.len(), self.files.values().filter(|file| file.path.starts_with("scripts/")).count(), self.files.values().filter(|file| file.executable).count())
    }
    /// Enumerate every SKILL.md package in a validated archive, not just the
    /// repository root. Paths are relative to the single archive wrapper.
    pub fn index_zip(bytes: &[u8]) -> Result<PackageIndex, Error> {
        let (mut files, modes, directories) = Self::archive_files(bytes, false)?;
        let wrapper = if !files.iter().any(|f| f.path == "SKILL.md") && files.iter().all(|f| f.path.contains('/')) {
            files.first().and_then(|f| f.path.split('/').next()).filter(|first| files.iter().all(|f| f.path.starts_with(&format!("{first}/")))).map(str::to_owned)
        } else { None };
        let mut unwrapped_modes = BTreeMap::new();
        for file in &mut files {
            let bits = modes.get(&file.path).copied().unwrap_or(0);
            if let Some(wrapper) = &wrapper { file.path = file.path.strip_prefix(&format!("{wrapper}/")).ok_or(Error::InvalidFormat)?.into(); }
            unwrapped_modes.insert(file.path.clone(), bits);
        }
        let directories = directories.into_iter().filter_map(|path| match &wrapper { Some(wrapper) => path.strip_prefix(&format!("{wrapper}/")).map(str::to_owned), None => Some(path) }).collect();
        Self::index_files(files, unwrapped_modes, directories)
    }
    #[cfg(unix)]
    pub fn index_archive(path: &std::path::Path) -> Result<PackageIndex, Error> {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let parent = super::paths::Directory::open(path.parent().ok_or(Error::UnsafePath)?)?;
        let mut file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC).open(path).map_err(|_| Error::UnsafePath)?;
        let before = file.metadata().map_err(|_| Error::Unavailable)?;
        if !before.is_file() || before.nlink() != 1 || before.uid() != unsafe { libc::geteuid() } { return Err(Error::UnsafePath); }
        super::paths::no_extended_acl(&file)?;
        if before.len() > ARCHIVE_LIMIT as u64 { return Err(Error::Limit); }
        let mut bytes = Vec::new();
        (&mut file).take(ARCHIVE_LIMIT as u64 + 1).read_to_end(&mut bytes).map_err(|_| Error::Unavailable)?;
        let after = file.metadata().map_err(|_| Error::Unavailable)?;
        let named = std::fs::symlink_metadata(path).map_err(|_| Error::Conflict)?;
        if bytes.len() > ARCHIVE_LIMIT || bytes.len() as u64 != before.len() || before.dev() != after.dev() || before.ino() != after.ino() || before.mtime() != after.mtime() || before.mtime_nsec() != after.mtime_nsec() || named.dev() != after.dev() || named.ino() != after.ino() || super::paths::Directory::open(path.parent().ok_or(Error::UnsafePath)?)?.identity()? != parent.identity()? { return Err(Error::Conflict); }
        Self::index_zip(&bytes)
    }
    #[cfg(unix)]
    pub fn index_directory(path: &std::path::Path) -> Result<PackageIndex, Error> {
        let root = super::paths::Directory::open(path)?;
        let (snapshot, files) = super::resource_tree::scan(&root)?;
        let modes = snapshot.files.iter().map(|(path, entry)| (path.clone(), entry.mode & 0o111)).collect();
        let index = Self::index_files(files, modes, snapshot.directories.keys().cloned().collect())?;
        if super::resource_tree::scan(&root)?.0 != snapshot || super::paths::Directory::open(path)?.identity()? != snapshot.identity { return Err(Error::Conflict); }
        Ok(index)
    }
    fn index_files(files: Vec<PackageFile>, modes: BTreeMap<String, u32>, directories: Vec<String>) -> Result<PackageIndex, Error> {
        let paths: Vec<_> = files.iter().filter_map(|file| {
            if file.path == "SKILL.md" { Some(String::new()) } else { file.path.strip_suffix("/SKILL.md").map(str::to_owned) }
        }).collect();
        if paths.len() > 128 { return Err(Error::Limit); }
        let mut groups: BTreeMap<String, Vec<PackageFile>> = paths.iter().map(|path| (path.clone(), Vec::new())).collect();
        for mut file in files {
            let mut destination: Option<&String> = None;
            for path in paths.iter().filter(|path| path.is_empty() || file.path.strip_prefix(path.as_str()).is_some_and(|suffix| suffix.starts_with('/'))) {
                if let Some(previous) = destination.replace(path) {
                    let mut selected = file.clone();
                    if !previous.is_empty() { selected.path = selected.path[previous.len() + 1..].into(); }
                    groups.get_mut(previous).ok_or(Error::InvalidFormat)?.push(selected);
                }
            }
            if let Some(path) = destination {
                if !path.is_empty() { file.path = file.path[path.len() + 1..].into(); }
                groups.get_mut(path).ok_or(Error::InvalidFormat)?.push(file);
            }
        }
        let mut index = PackageIndex { packages: BTreeMap::new(), rejected: Vec::new() };
        for (path, selected) in groups {
            let prefix = if path.is_empty() { String::new() } else { format!("{path}/") };
            if directories.iter().filter_map(|directory| directory.strip_prefix(&prefix)).any(|directory| loose_path(directory).is_err()) {
                index.rejected.push((path, Error::Unsupported.to_string())); continue;
            }
            match Self::from_files(selected) {
                Ok(mut package) => {
                    package.execution_bits = package.files.keys().map(|relative| (relative.clone(), modes.get(&format!("{prefix}{relative}")).copied().unwrap_or(0))).collect();
                    package.rehash(); index.packages.insert(path, package);
                },
                Err(error) => index.rejected.push((path, error.to_string())),
            }
        }
        Ok(index)
    }
}
