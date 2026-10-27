use crate::{BundledCatalog, BundledSkill, ContentSkillMetadata, Result, SkillError};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

pub const MAX_FILE_BYTES: u64 = 256 * 1024;
pub const MAX_PACKAGE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PACKAGE_FILES: usize = 128;

fn invalid(message: impl Into<String>) -> SkillError {
    SkillError::InvalidManifest(message.into())
}

pub fn safe_child(root: &Path, relative: &str) -> Result<PathBuf> {
    if relative.is_empty() || relative.contains('\\') || relative.contains(':') {
        return Err(invalid("invalid relative package path"));
    }
    let mut path = root.to_path_buf();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => path.push(part),
            _ => return Err(invalid("package path escapes root")),
        }
        if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(invalid("symlinks are not allowed"));
        }
    }
    Ok(path)
}

fn collect(
    root: &Path,
    dir: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
    total: &mut usize,
    entries: &mut usize,
) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .map_err(|_| invalid("outside package"))?
            .to_str()
            .ok_or_else(|| invalid("non UTF-8 path"))?
            .to_string();
        *entries += 1;
        if *entries > 256
            || Path::new(&rel).components().count() > 8
            || rel
                .bytes()
                .any(|byte| !byte.is_ascii_alphanumeric() && !b"/._-".contains(&byte))
        {
            return Err(invalid("resource path or directory limit exceeded"));
        }
        if ty.is_dir() {
            if !["references", "templates", "examples"]
                .contains(&rel.split('/').next().unwrap_or(""))
            {
                return Err(invalid("unsupported resource directory"));
            }
            collect(root, &path, files, total, entries)?;
        } else if ty.is_file() {
            if files.len() >= MAX_PACKAGE_FILES || entry.metadata()?.len() > MAX_FILE_BYTES {
                return Err(invalid("package file limit exceeded"));
            }
            let top = ["skill.json", "SKILL.md", "LICENSE"].contains(&rel.as_str());
            let resource = rel.contains('/')
                && ["references", "templates", "examples"]
                    .contains(&rel.split('/').next().unwrap_or(""))
                && matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("md" | "txt" | "json" | "csv")
                );
            if !top && !resource {
                return Err(invalid(format!("unsupported file: {rel}")));
            }
            // Bounded reads also protect against a file growing after metadata.
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(&path)?
                .take(MAX_FILE_BYTES + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > MAX_FILE_BYTES as usize {
                return Err(invalid("resource grew beyond limit"));
            }
            std::str::from_utf8(&bytes).map_err(|_| invalid("resources must be UTF-8 text"))?;
            *total += bytes.len();
            if *total > MAX_PACKAGE_BYTES {
                return Err(invalid("package byte limit exceeded"));
            }
            files.insert(rel, bytes);
        } else {
            return Err(invalid("symlinks and special files are not allowed"));
        }
    }
    Ok(())
}

pub fn validate_package(path: &Path, expected_hash: &str) -> Result<BundledSkill> {
    if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(invalid("symlink package root"));
    }
    let mut files = BTreeMap::new();
    let mut total = 0;
    let mut entries = 0;
    collect(path, path, &mut files, &mut total, &mut entries)?;
    for required in ["skill.json", "SKILL.md", "LICENSE"] {
        if files.get(required).map_or(true, |bytes| bytes.is_empty()) {
            return Err(invalid(format!("missing {required}")));
        }
    }
    let meta: ContentSkillMetadata = serde_json::from_slice(&files["skill.json"])?;
    let categories = [
        "writing",
        "visual",
        "video-audio",
        "documents-office",
        "data-research",
        "development-automation",
    ];
    if meta.schema_version != "1.0"
        || meta.entry != "SKILL.md"
        || meta.identity != "dgos_official"
        || meta.source.kind != "dgos_official"
        || meta.id != format!("dgos.official.{}.{}", meta.category, meta.slug)
        || meta.slug.is_empty()
        || meta
            .slug
            .bytes()
            .any(|b| !b.is_ascii_lowercase() && !b.is_ascii_digit() && b != b'-')
        || meta.source.path != Some(format!("official/{}/{}", meta.category, meta.slug))
        || !categories.contains(&meta.category.as_str())
        || meta.name.trim().is_empty()
        || !meta.license.redistributable
        || meta.license.spdx.as_deref().map_or(true, str::is_empty)
        || meta
            .id
            .bytes()
            .any(|b| !b.is_ascii_lowercase() && !b.is_ascii_digit() && b != b'.' && b != b'-')
        || semver::Version::parse(&meta.version).is_err()
    {
        return Err(invalid("invalid official skill metadata"));
    }
    let mut hash = Sha256::new();
    let mut manifest = BTreeMap::new();
    let body =
        String::from_utf8(files["SKILL.md"].clone()).map_err(|_| invalid("non UTF-8 body"))?;
    let license_text =
        String::from_utf8(files["LICENSE"].clone()).map_err(|_| invalid("non UTF-8 license"))?;
    for (name, bytes) in files {
        hash.update(name.as_bytes());
        hash.update([0]);
        hash.update(&bytes);
        hash.update([0]);
        manifest.insert(name, format!("{:x}", Sha256::digest(&bytes)));
    }
    let actual = format!("{:x}", hash.finalize());
    if actual != expected_hash {
        return Err(invalid("package hash mismatch"));
    }
    Ok(BundledSkill {
        metadata: meta,
        install_path: path.to_string_lossy().into_owned(),
        content_hash: actual,
        files: manifest,
        body,
        license_text,
    })
}

pub fn load_bundled_catalog(root: &Path) -> Result<Vec<BundledSkill>> {
    let catalog = std::fs::read(safe_child(root, "catalog.v1.json")?)?;
    // Trust derives from the application release, never from package metadata.
    if catalog != include_bytes!("../../../skills/catalog.v1.json") {
        return Err(invalid("bundled catalog differs from application release"));
    }
    let catalog: BundledCatalog = serde_json::from_slice(&catalog)?;
    if catalog.schema_version != "1.0"
        || catalog.catalog_version.is_empty()
        || catalog.skills.len() > 128
    {
        return Err(invalid("unsupported catalog"));
    }
    let mut seen = std::collections::HashSet::new();
    catalog
        .skills
        .into_iter()
        .map(|entry| {
            if !seen.insert(entry.id.clone()) {
                return Err(invalid("duplicate catalog ID"));
            }
            let package = validate_package(&safe_child(root, &entry.path)?, &entry.sha256)?;
            if package.metadata.id != entry.id
                || package.metadata.version != entry.version
                || package.metadata.source.path.as_deref() != Some(entry.path.as_str())
            {
                return Err(invalid("catalog metadata mismatch"));
            }
            Ok(package)
        })
        .collect()
}
