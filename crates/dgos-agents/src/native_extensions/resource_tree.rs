use super::{package::{self, NativeSkillPackage, PackageFile}, paths::{Directory, Identity, Snapshot}, Error};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Tree { pub identity: Identity, pub directories: BTreeMap<String, Identity>, pub files: BTreeMap<String, Snapshot>, pub hash: String }
pub(super) fn scan(root: &Directory) -> Result<(Tree, Vec<PackageFile>), Error> {
    let mut tree = Tree { identity: root.identity()?, directories: BTreeMap::new(), files: BTreeMap::new(), hash: String::new() };
    let mut files = Vec::new(); let mut total = 0; let mut entries = 0;
    collect(root, "", &mut tree, &mut files, &mut total, &mut entries)?;
    let mut digest = Sha256::new();
    for (path, id) in &tree.directories { digest.update(path.as_bytes()); digest.update(id.dev.to_le_bytes()); digest.update(id.ino.to_le_bytes()); }
    for (path, snapshot) in &tree.files { digest.update(path.as_bytes()); digest.update(snapshot.hash.as_bytes()); digest.update(snapshot.mode.to_le_bytes()); }
    tree.hash = format!("{:x}", digest.finalize());
    Ok((tree, files))
}
fn collect(dir: &Directory, prefix: &str, tree: &mut Tree, files: &mut Vec<PackageFile>, total: &mut usize, entries: &mut usize) -> Result<(), Error> {
    dir.writable_owner()?;
    let (names, incomplete) = dir.names(package::ENTRY_LIMIT * 2)?;
    if incomplete { return Err(Error::Limit); }
    let original_names = names.clone();
    let mut folded = std::collections::BTreeSet::new();
    for name in names {
        if !folded.insert(name.to_ascii_lowercase()) { return Err(Error::Conflict); }
        *entries += 1; if *entries > package::ENTRY_LIMIT * 2 { return Err(Error::Limit); }
        let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        package::safe_path(&path)?;
        match dir.kind(&name)? {
            "directory" => {
                let child = dir.child(name.as_ref(), false)?;
                tree.directories.insert(path.clone(), child.identity()?);
                collect(&child, &path, tree, files, total, entries)?;
                if dir.child(name.as_ref(), false)?.identity()? != child.identity()? { return Err(Error::Conflict); }
            },
            "file" => {
                let (bytes, snapshot) = dir.read_bytes(&name, package::FILE_LIMIT)?.ok_or(Error::Conflict)?;
                if snapshot.mode & 0o022 != 0 { return Err(Error::AccessDenied); }
                *total += bytes.len(); if *total > package::TREE_LIMIT || tree.files.len() >= package::ENTRY_LIMIT { return Err(Error::Limit); }
                files.push(PackageFile { path: path.clone(), bytes, executable: snapshot.mode & 0o111 != 0 });
                tree.files.insert(path, snapshot);
            },
            _ => return Err(Error::UnsafePath),
        }
    }
    let (after_names, incomplete) = dir.names(package::ENTRY_LIMIT * 2)?;
    if incomplete || after_names != original_names { return Err(Error::Conflict); }
    for name in original_names {
        let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        if let Some(expected) = tree.files.get(&path) {
            if dir.read_bytes(&name, package::FILE_LIMIT)?.map(|v| v.1).as_ref() != Some(expected) { return Err(Error::Conflict); }
        } else if let Some(expected) = tree.directories.get(&path) {
            if dir.child(name.as_ref(), false)?.identity()? != *expected { return Err(Error::Conflict); }
        }
    }
    Ok(())
}
pub(super) fn write(root: &Directory, package: &NativeSkillPackage) -> Result<Tree, Error> {
    for file in package.files.values() {
        let mut parts: Vec<&str> = file.path.split('/').collect(); let filename = parts.pop().ok_or(Error::UnsafePath)?;
        // Root itself is borrowed; open children without an absolute path.
        let mut directories = Vec::new();
        for part in parts {
            let dir = match directories.last() { Some(dir) => { let dir: &Directory = dir; dir.child(part.as_ref(), true)? }, None => root.child(part.as_ref(), true)? };
            directories.push(dir);
        }
        let destination = directories.last().unwrap_or(root);
        destination.write_new(filename, &file.bytes, 0o600 | package.execution_bits.get(&file.path).copied().unwrap_or(0))?;
    }
    scan(root).map(|v| v.0)
}
