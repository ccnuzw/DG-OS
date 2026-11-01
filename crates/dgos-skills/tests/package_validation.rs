use dgos_skills::{validate_external_zip, validate_external_zip_at_subpath, LicenseStatus};
use std::io::{Cursor, Write};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, bytes) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
    output.into_inner()
}

const SKILL: &[u8] = b"---\nname: Demo\ndescription: A safe demo skill\n---\n\n# Demo\n";

#[test]
fn validates_minimal_external_package_and_generates_import_identity() {
    let package = validate_external_zip(&archive(&[("SKILL.md", SKILL)])).unwrap();
    assert!(package.skill_id.starts_with("imported."));
    assert_eq!(package.metadata.identity, package.skill_id);
    assert_eq!(package.metadata.source_kind, "external");
    assert_eq!(package.license_status, LicenseStatus::Missing);
    assert!(package.file_manifest.contains_key("SKILL.md"));
}

#[test]
fn accepts_wrapped_root_with_manifest_and_license() {
    let manifest = br#"{
      "schema_version":"1.0",
      "id":"community.demo",
      "name":"Demo",
      "description":"A safe demo skill",
      "version":"2.0.0",
      "identity":"community",
      "entry":"SKILL.md",
      "license":{"spdx":"MIT"}
    }"#;
    let package = validate_external_zip(&archive(&[
        ("repo-main/SKILL.md", SKILL),
        ("repo-main/skill.json", manifest),
        ("repo-main/LICENSE", b"MIT"),
        ("repo-main/references/notes.md", b"notes"),
    ]))
    .unwrap();
    assert_eq!(package.skill_id, "community.demo");
    assert_eq!(package.version_label, "2.0.0");
    assert_eq!(package.license_status, LicenseStatus::Present);
    assert!(package.files.contains_key("references/notes.md"));
}

#[test]
fn selects_one_skill_from_a_github_style_repository_archive() {
    let package = validate_external_zip_at_subpath(
        &archive(&[
            ("repo-main/skills/demo/SKILL.md", SKILL),
            ("repo-main/skills/demo/LICENSE", b"MIT"),
            ("repo-main/README.md", b"repository documentation"),
        ]),
        Some("skills/demo"),
    )
    .unwrap();
    assert!(package.files.contains_key("SKILL.md"));
    assert!(package.files.contains_key("LICENSE"));
}

#[test]
fn rejects_missing_entry_unsafe_paths_and_executable_files() {
    assert!(validate_external_zip(&archive(&[("README.md", b"no skill")])).is_err());
    assert!(validate_external_zip(&archive(&[("../SKILL.md", SKILL)])).is_err());
    assert!(validate_external_zip(&archive(&[("scripts/run.sh", b"echo no")])).is_err());

    let mut output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output);
    let options = SimpleFileOptions::default().unix_permissions(0o100755);
    writer.start_file("SKILL.md", options).unwrap();
    writer.write_all(SKILL).unwrap();
    writer.finish().unwrap();
    assert!(validate_external_zip(&output.into_inner()).is_err());
}

#[test]
fn rejects_binary_content_and_multiple_roots() {
    assert!(validate_external_zip(&archive(&[("SKILL.md", &[0xff, 0xfe])])).is_err());
    assert!(validate_external_zip(&archive(&[
        ("one/SKILL.md", SKILL),
        ("two/SKILL.md", SKILL),
    ]))
    .is_err());
}

#[test]
fn external_manifest_cannot_claim_official_identity_or_source() {
    for field in [
        br#""identity":"dgos_official""# as &[u8],
        br#""source":{"kind":"dgos_official"}""# as &[u8],
    ] {
        let manifest = format!(
            "{{\"schema_version\":\"1.0\",\"id\":\"community.demo\",\"name\":\"Demo\",\"description\":\"A safe demo skill\",{}}}",
            std::str::from_utf8(field).unwrap()
        );
        assert!(validate_external_zip(&archive(&[
            ("SKILL.md", SKILL),
            ("skill.json", manifest.as_bytes()),
        ]))
        .is_err());
    }
}
