use dgos_skills::{parse_command, parse_source, GithubSource, SkillSource};

#[test]
fn accepts_supported_github_forms() {
    let SkillSource::Github(GithubSource {
        owner,
        repo,
        requested_ref,
        skill_subpath,
        ..
    }) = parse_source(
        "https://github.com/ccnuzw/daoge-skills/tree/spec-docs-pro-v2.1.0/skills/spec-docs-pro",
    )
    .unwrap()
    else {
        panic!("expected GitHub source");
    };
    assert_eq!(owner, "ccnuzw");
    assert_eq!(repo, "daoge-skills");
    assert_eq!(requested_ref.as_deref(), Some("spec-docs-pro-v2.1.0"));
    assert_eq!(skill_subpath.as_deref(), Some("skills/spec-docs-pro"));

    let source =
        parse_command("npx skills add ccnuzw/daoge-skills -a codex -s spec-docs-pro").unwrap();
    assert_eq!(source.owner, "ccnuzw");
    assert_eq!(source.skill_subpath.as_deref(), Some("spec-docs-pro"));
}

#[test]
fn accepts_local_zip_for_file_picker_layer() {
    let SkillSource::LocalZip { path } = parse_source("/tmp/my-skill.zip").unwrap() else {
        panic!("expected local zip");
    };
    assert_eq!(path.to_string_lossy(), "/tmp/my-skill.zip");
}

#[test]
fn rejects_unsafe_or_ambiguous_commands() {
    for command in [
        "npx skills add ccnuzw/daoge-skills -a codex --yes",
        "npx skills add ccnuzw/daoge-skills -a codex --token secret",
        "npx skills add ccnuzw/daoge-skills -a codex -s one -s two",
        "npx skills add ccnuzw/daoge-skills -a codex; echo pwned",
    ] {
        assert!(
            parse_command(command).is_err(),
            "accepted unsafe command: {command}"
        );
    }
    for source in [
        "http://github.com/acme/repo",
        "https://github.com.evil.example/acme/repo",
        "https://github.com/acme/repo?ref=main",
        "https://github.com/acme/repo/tree/feature//slashes",
    ] {
        assert!(
            parse_source(source).is_err(),
            "accepted unsafe source: {source}"
        );
    }
}
