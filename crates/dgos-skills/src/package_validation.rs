//! Safe validation for user-supplied Skill zip archives.
//!
//! Validation is deliberately performed over an in-memory representation of
//! the archive. No archive entry is ever extracted to the published Skill
//! directory and no package-provided command or script is executed.

use crate::{Result, SkillError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    io::{Cursor, Read},
    path::Path,
};
use zip::ZipArchive;

pub const MAX_EXTERNAL_ARCHIVE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_EXTERNAL_FILE_BYTES: u64 = 256 * 1024;
pub const MAX_EXTERNAL_UNPACKED_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_EXTERNAL_FILES: usize = 128;
pub const MAX_EXTERNAL_ENTRIES: usize = 256;
pub const MAX_EXTERNAL_PATH_DEPTH: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LicenseStatus {
    Present,
    DeclaredOnly,
    Missing,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalSkillMetadata {
    pub schema_version: Option<String>,
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: Option<String>,
    pub identity: String,
    pub source_kind: String,
    pub entry: String,
    pub author: Option<String>,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub license_spdx: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ValidatedExternalPackage {
    pub skill_id: String,
    pub version_label: String,
    pub content_hash: String,
    pub files: BTreeMap<String, Vec<u8>>,
    pub file_manifest: BTreeMap<String, String>,
    pub metadata: ExternalSkillMetadata,
    pub license_status: LicenseStatus,
}

fn invalid(message: impl Into<String>) -> SkillError {
    SkillError::InvalidManifest(format!(
        "invalid external Skill package: {}",
        message.into()
    ))
}

fn allowed_path_bytes(path: &str) -> bool {
    path.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"/._-".contains(&byte))
}

fn normalize_zip_name(raw: &str, is_dir: bool) -> Result<String> {
    if raw.is_empty() || raw.contains('\0') || raw.contains('\\') || !raw.is_ascii() {
        return Err(invalid("entry path must be ASCII and use forward slashes"));
    }
    if raw.starts_with('/') || raw.starts_with('~') || raw.contains(':') {
        return Err(invalid("absolute entry path is not allowed"));
    }
    let without_trailing = if is_dir {
        raw.trim_end_matches('/')
    } else {
        raw
    };
    if without_trailing.is_empty() || !allowed_path_bytes(without_trailing) {
        return Err(invalid("invalid entry path"));
    }
    let parts: Vec<&str> = without_trailing.split('/').collect();
    if parts.is_empty()
        || parts.len() > MAX_EXTERNAL_PATH_DEPTH
        || parts
            .iter()
            .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return Err(invalid("entry path contains an unsafe component"));
    }
    Ok(parts.join("/"))
}

fn is_allowed_resource_file(path: &str) -> bool {
    let mut parts = path.split('/');
    let Some(top) = parts.next() else {
        return false;
    };
    let extension_allowed = matches!(
        Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("md" | "txt" | "json" | "csv")
    );
    parts.next().is_some()
        && matches!(top, "references" | "templates" | "examples")
        && extension_allowed
}

fn valid_selection_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 256
        && !path.contains('\\')
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        })
}

fn frontmatter(body: &str) -> Result<(String, String)> {
    let mut lines = body.lines();
    let first = lines
        .next()
        .map(|line| line.trim_end_matches('\r'))
        .ok_or_else(|| invalid("SKILL.md is empty"))?;
    if first.trim_start_matches('\u{feff}') != "---" {
        return Err(invalid("SKILL.md must start with YAML frontmatter"));
    }
    let mut name = None;
    let mut description = None;
    let mut closed = false;
    for line in lines {
        let line = line.trim_end_matches('\r');
        if line == "---" {
            closed = true;
            break;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches(['\'', '"']);
        match key.trim() {
            "name" => {
                if name.replace(value.to_owned()).is_some() {
                    return Err(invalid("duplicate frontmatter name"));
                }
            }
            "description" if description.replace(value.to_owned()).is_some() => {
                return Err(invalid("duplicate frontmatter description"));
            }
            "description" => { description = Some(value.to_owned()); }
            _ => {}
        }
    }
    if !closed {
        return Err(invalid("SKILL.md frontmatter is not closed"));
    }
    let name = name.filter(|value| !value.trim().is_empty());
    let description = description.filter(|value| !value.trim().is_empty());
    let Some(name) = name else {
        return Err(invalid("frontmatter name is required"));
    };
    let Some(description) = description else {
        return Err(invalid("frontmatter description is required"));
    };
    if name.chars().count() > 128 || description.chars().count() > 4_096 {
        return Err(invalid("frontmatter field is too long"));
    }
    Ok((name, description))
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 160
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        && !id.starts_with("dgos.official.")
}

fn parse_manifest(
    bytes: Option<&[u8]>,
    front_name: &str,
    front_description: &str,
) -> Result<(ExternalSkillMetadata, bool)> {
    let Some(bytes) = bytes else {
        return Ok((
            ExternalSkillMetadata {
                schema_version: None,
                id: String::new(),
                name: front_name.to_owned(),
                description: front_description.to_owned(),
                version: None,
                identity: "imported".into(),
                source_kind: "external".into(),
                entry: "SKILL.md".into(),
                author: None,
                category: None,
                tags: Vec::new(),
                license_spdx: None,
            },
            false,
        ));
    };
    let value: Value = serde_json::from_slice(bytes).map_err(|error| invalid(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| invalid("skill.json must be an object"))?;
    const ALLOWED: &[&str] = &[
        "schema_version",
        "id",
        "slug",
        "name",
        "description",
        "version",
        "identity",
        "entry",
        "author",
        "category",
        "tags",
        "locales",
        "compatible_apps",
        "required_tools",
        "license",
        "source",
        "verification",
    ];
    if object.keys().any(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(invalid("skill.json contains an unknown field"));
    }

    let schema_version = object
        .get("schema_version")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("skill.json schema_version must be 1.0"))?;
    if schema_version != "1.0" {
        return Err(invalid("unsupported skill.json schema"));
    }
    let entry = object
        .get("entry")
        .and_then(Value::as_str)
        .or_else(|| {
            if object.contains_key("entry") {
                None
            } else {
                Some("SKILL.md")
            }
        })
        .ok_or_else(|| invalid("skill.json entry must be a string"))?;
    if entry != "SKILL.md" {
        return Err(invalid("skill.json entry must be SKILL.md"));
    }
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| {
            if object.contains_key("name") {
                None
            } else {
                Some(front_name)
            }
        })
        .ok_or_else(|| invalid("skill.json name must be a string"))?;
    let description = object
        .get("description")
        .and_then(Value::as_str)
        .or_else(|| {
            if object.contains_key("description") {
                None
            } else {
                Some(front_description)
            }
        })
        .ok_or_else(|| invalid("skill.json description must be a string"))?;
    if name != front_name || description != front_description {
        return Err(invalid("skill.json metadata disagrees with SKILL.md"));
    }
    let identity = object
        .get("identity")
        .and_then(Value::as_str)
        .or_else(|| {
            if object.contains_key("identity") {
                None
            } else {
                Some("imported")
            }
        })
        .ok_or_else(|| invalid("skill.json identity must be a string"))?;
    if identity.is_empty() || identity.starts_with("dgos_official") {
        return Err(invalid("external content cannot declare official identity"));
    }
    if let Some(source) = object.get("source") {
        let source = source
            .as_object()
            .ok_or_else(|| invalid("skill.json source must be an object"))?;
        if let Some(kind) = source.get("kind") {
            let kind = kind
                .as_str()
                .ok_or_else(|| invalid("skill.json source.kind must be a string"))?;
            if kind == "dgos_official" {
                return Err(invalid("external content cannot declare official source"));
            }
        }
    }
    let id = object
        .get("id")
        .or_else(|| object.get("slug"))
        .and_then(Value::as_str)
        .or_else(|| {
            if object.contains_key("id") || object.contains_key("slug") {
                None
            } else {
                Some("")
            }
        })
        .ok_or_else(|| invalid("skill.json id must be a string"))?;
    if !id.is_empty() && !safe_id(id) {
        return Err(invalid("invalid external skill id"));
    }
    let version = object
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            if object.contains_key("version") {
                None
            } else {
                Some(String::new())
            }
        });
    if object.contains_key("version") && version.is_none() {
        return Err(invalid("skill.json version must be a string"));
    }
    let version = version.filter(|value| !value.is_empty());
    if version
        .as_deref()
        .is_some_and(|value| value.is_empty() || value.len() > 128)
    {
        return Err(invalid("invalid external skill version"));
    }
    let tags = if let Some(value) = object.get("tags") {
        let values = value
            .as_array()
            .ok_or_else(|| invalid("skill.json tags must be an array"))?;
        values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("skill.json tags must be strings"))
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    let license = object.get("license");
    if let Some(license) = license {
        if !license.is_string() && !license.is_object() {
            return Err(invalid("skill.json license must be a string or object"));
        }
        if let Some(spdx) = license.get("spdx") {
            if !spdx.is_string() {
                return Err(invalid("skill.json license.spdx must be a string"));
            }
        }
    }
    for field in ["author", "category"] {
        if let Some(value) = object.get(field) {
            if !value.is_string() {
                return Err(invalid(format!("skill.json {field} must be a string")));
            }
        }
    }
    let license_spdx = object
        .get("license")
        .and_then(|license| license.as_str().or_else(|| license.get("spdx")?.as_str()))
        .map(str::to_owned);
    Ok((
        ExternalSkillMetadata {
            schema_version: Some(schema_version.to_owned()),
            id: id.to_owned(),
            name: name.to_owned(),
            description: description.to_owned(),
            version,
            identity: identity.to_owned(),
            source_kind: "external".into(),
            entry: entry.into(),
            author: object
                .get("author")
                .and_then(Value::as_str)
                .map(str::to_owned),
            category: object
                .get("category")
                .and_then(Value::as_str)
                .map(str::to_owned),
            tags,
            license_spdx,
        },
        true,
    ))
}

pub(crate) fn content_hash(
    files: &BTreeMap<String, Vec<u8>>,
) -> (String, BTreeMap<String, String>) {
    let mut hash = Sha256::new();
    let mut manifest = BTreeMap::new();
    for (path, bytes) in files {
        hash.update(path.as_bytes());
        hash.update([0]);
        hash.update(bytes);
        hash.update([0]);
        manifest.insert(path.clone(), format!("{:x}", Sha256::digest(bytes)));
    }
    (format!("{:x}", hash.finalize()), manifest)
}

/// Validate a zip archive supplied by a local file picker or GitHub adapter.
pub fn validate_external_zip(bytes: &[u8]) -> Result<ValidatedExternalPackage> {
    validate_external_zip_at_subpath(bytes, None)
}

/// Validate one Skill nested inside a repository archive. The complete ZIP is
/// still inspected for limits and unsafe entries before files outside the
/// selected subpath are discarded.
pub fn validate_external_zip_at_subpath(
    bytes: &[u8],
    selected_subpath: Option<&str>,
) -> Result<ValidatedExternalPackage> {
    if let Some(path) = selected_subpath {
        if !valid_selection_path(path) {
            return Err(invalid("invalid selected Skill subpath"));
        }
    }
    if bytes.len() > MAX_EXTERNAL_ARCHIVE_BYTES {
        return Err(invalid("compressed archive exceeds 8 MiB"));
    }
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).map_err(|error| invalid(error.to_string()))?;
    if archive.len() > MAX_EXTERNAL_ENTRIES {
        return Err(invalid("archive contains too many entries"));
    }

    struct Entry {
        path: String,
        is_dir: bool,
        bytes: Vec<u8>,
    }
    let mut entries = Vec::with_capacity(archive.len());
    let mut total_unpacked = 0u64;
    let mut total_compressed = 0u64;
    let mut file_count = 0usize;
    let mut seen = HashSet::new();
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|error| invalid(error.to_string()))?;
        if file.encrypted() {
            return Err(invalid("encrypted archives are not supported"));
        }
        if file.is_symlink() {
            return Err(invalid("symlinks are not allowed"));
        }
        let raw_name = file
            .name()
            .map_err(|error| invalid(error.to_string()))?
            .into_owned();
        let is_dir = file.is_dir() || raw_name.ends_with('/');
        let path = normalize_zip_name(&raw_name, is_dir)?;
        if !seen.insert(path.clone()) {
            return Err(invalid("duplicate normalized entry path"));
        }
        if path.split('/').count() > MAX_EXTERNAL_PATH_DEPTH {
            return Err(invalid("entry path is too deep"));
        }
        if let Some(mode) = file.unix_mode() {
            let file_type = mode & 0o170000;
            if file_type != 0 && file_type != 0o100000 && file_type != 0o040000 {
                return Err(invalid("symlinks and special files are not allowed"));
            }
            if !is_dir && mode & 0o111 != 0 {
                return Err(invalid("executable files are not allowed"));
            }
        }
        total_compressed = total_compressed
            .checked_add(file.compressed_size())
            .ok_or_else(|| invalid("compressed size overflow"))?;
        if total_compressed > MAX_EXTERNAL_ARCHIVE_BYTES as u64
            || file.size() > MAX_EXTERNAL_FILE_BYTES
        {
            return Err(invalid("archive entry exceeds compressed or file limit"));
        }
        total_unpacked = total_unpacked
            .checked_add(file.size())
            .ok_or_else(|| invalid("unpacked size overflow"))?;
        if total_unpacked > MAX_EXTERNAL_UNPACKED_BYTES {
            return Err(invalid("archive expands beyond 2 MiB"));
        }
        let bytes = if is_dir {
            Vec::new()
        } else {
            file_count += 1;
            if file_count > MAX_EXTERNAL_FILES {
                return Err(invalid("archive contains too many files"));
            }
            let mut data = Vec::with_capacity(file.size() as usize);
            file.read_to_end(&mut data)
                .map_err(|error| invalid(error.to_string()))?;
            if data.len() as u64 > MAX_EXTERNAL_FILE_BYTES {
                return Err(invalid("file grew beyond the declared limit"));
            }
            data
        };
        entries.push(Entry {
            path,
            is_dir,
            bytes,
        });
    }

    // A common archive layout is `repo-name/SKILL.md`. Strip exactly one
    // wrapper only when there are no root-level files; mixed roots are never
    // guessed.
    let has_root_file = entries
        .iter()
        .any(|entry| !entry.is_dir && !entry.path.contains('/'));
    let top_components: HashSet<&str> = entries
        .iter()
        .filter_map(|entry| entry.path.split('/').next())
        .collect();
    let wrapper = if !has_root_file && top_components.len() == 1 {
        let candidate = *top_components.iter().next().expect("one component");
        if !matches!(candidate, "references" | "templates" | "examples") {
            Some(candidate.to_owned())
        } else {
            None
        }
    } else if !has_root_file && top_components.len() > 1 {
        return Err(invalid("multiple top-level Skill roots are not allowed"));
    } else {
        None
    };

    let selection_prefix = selected_subpath.map(str::to_owned);
    let mut files = BTreeMap::new();
    for entry in entries {
        if entry.is_dir && wrapper.as_deref() == Some(entry.path.as_str()) {
            continue;
        }
        let mut path = if let Some(wrapper) = wrapper.as_deref() {
            let prefix = format!("{wrapper}/");
            entry
                .path
                .strip_prefix(&prefix)
                .ok_or_else(|| invalid("entry is outside the package root"))?
                .to_owned()
        } else {
            entry.path.clone()
        };
        if let Some(selection) = selection_prefix.as_deref() {
            let prefix = format!("{selection}/");
            if path == selection {
                path.clear();
            } else if let Some(stripped) = path.strip_prefix(&prefix) {
                path = stripped.to_owned();
            } else {
                continue;
            }
        }
        if entry.is_dir {
            if path.is_empty() {
                continue;
            }
            if !matches!(path.as_str(), "references" | "templates" | "examples") {
                return Err(invalid(format!("unsupported package directory: {path}")));
            }
            continue;
        }
        if path.is_empty() || !allowed_path_bytes(&path) {
            return Err(invalid("invalid normalized package path"));
        }
        let is_top = !path.contains('/');
        let allowed_top = matches!(path.as_str(), "SKILL.md" | "skill.json" | "LICENSE");
        if !(is_top && allowed_top) && !is_allowed_resource_file(&path) {
            return Err(invalid(format!("unsupported package file: {path}")));
        }
        if files.insert(path, entry.bytes).is_some() {
            return Err(invalid("duplicate package file"));
        }
    }

    validate_external_files(files)
}

/// Reuse the installed archive's metadata/identity rules without re-zipping or
/// copying its verified snapshot bytes.
pub(crate) fn validate_external_files(
    files: BTreeMap<String, Vec<u8>>,
) -> Result<ValidatedExternalPackage> {
    for bytes in files.values() {
        std::str::from_utf8(bytes).map_err(|_| invalid("package files must be UTF-8 text"))?;
    }

    let skill_bytes = files
        .get("SKILL.md")
        .ok_or_else(|| invalid("SKILL.md is required"))?;
    let body = std::str::from_utf8(skill_bytes).map_err(|_| invalid("SKILL.md is not UTF-8"))?;
    let (front_name, front_description) = frontmatter(body)?;
    let (mut metadata, has_manifest) = parse_manifest(
        files.get("skill.json").map(Vec::as_slice),
        &front_name,
        &front_description,
    )?;
    let (content_hash, file_manifest) = content_hash(&files);
    if metadata.id.is_empty() {
        metadata.id = format!("imported.{}", &content_hash[..16]);
        metadata.identity = metadata.id.clone();
    }
    let skill_id = metadata.id.clone();
    let version_label = metadata
        .version
        .clone()
        .unwrap_or_else(|| format!("revision:{}", &content_hash[..16]));
    let license_status = match files.get("LICENSE") {
        Some(license) if !license.is_empty() => LicenseStatus::Present,
        _ if has_manifest && metadata.license_spdx.is_some() => LicenseStatus::DeclaredOnly,
        _ => LicenseStatus::Missing,
    };
    Ok(ValidatedExternalPackage {
        skill_id,
        version_label,
        content_hash,
        files,
        file_manifest,
        metadata,
        license_status,
    })
}

/// Alias used by callers that already have bounded archive bytes.
pub fn validate_external_package(bytes: &[u8]) -> Result<ValidatedExternalPackage> {
    validate_external_zip(bytes)
}
