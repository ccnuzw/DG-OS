//! Parsing for the deliberately small set of external Skill source forms.
//!
//! This module is intentionally a parser, not an installer.  It never invokes
//! `npx`, a shell, Git, or a network client.  Keeping the parser independent
//! from the adapters also makes it possible to preview a source offline.

use crate::{Result, SkillError};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const MAX_INPUT_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillSource {
    LocalZip { path: PathBuf },
    Github(GithubSource),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubSource {
    pub owner: String,
    pub repo: String,
    /// `None` means that the adapter should resolve the repository default
    /// branch.  A requested ref is never treated as a resolved commit.
    pub requested_ref: Option<String>,
    pub skill_subpath: Option<String>,
    pub normalized_source: String,
}

fn invalid(message: impl Into<String>) -> SkillError {
    SkillError::InvalidManifest(format!("invalid Skill source: {}", message.into()))
}

fn has_shell_syntax(input: &str) -> bool {
    input
        .chars()
        .any(|c| matches!(c, ';' | '|' | '&' | '<' | '>' | '$' | '`' | '\n' | '\r'))
}

fn valid_identifier(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.chars().count() <= max_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && value != "."
        && value != ".."
}

fn valid_ref(value: &str) -> bool {
    valid_identifier(value, 128)
        || (value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn valid_subpath(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains('%')
        && !value.contains('\\')
        && value.split('/').all(|part| valid_identifier(part, 128))
}

fn github_source(
    owner: &str,
    repo: &str,
    requested_ref: Option<&str>,
    skill_subpath: Option<&str>,
) -> Result<GithubSource> {
    if !valid_identifier(owner, 100) {
        return Err(invalid("invalid owner"));
    }
    if !valid_identifier(repo, 100) {
        return Err(invalid("invalid repository"));
    }
    if let Some(reference) = requested_ref {
        if !valid_ref(reference) || reference.contains('/') {
            return Err(invalid(
                "ref must be one path segment or a 40-character commit",
            ));
        }
    }
    if let Some(path) = skill_subpath {
        if !valid_subpath(path) {
            return Err(invalid("invalid skill subpath"));
        }
    }

    let mut normalized = format!("https://github.com/{owner}/{repo}");
    if let Some(reference) = requested_ref {
        normalized.push_str("/tree/");
        normalized.push_str(reference);
        if let Some(path) = skill_subpath {
            normalized.push('/');
            normalized.push_str(path);
        }
    } else if let Some(path) = skill_subpath {
        // A subpath without a ref is represented as a repository URL plus the
        // path in the structured value.  Do not invent a branch name here.
        normalized.push_str("#skill=");
        normalized.push_str(path);
    }

    Ok(GithubSource {
        owner: owner.to_owned(),
        repo: repo.to_owned(),
        requested_ref: requested_ref.map(str::to_owned),
        skill_subpath: skill_subpath.map(str::to_owned),
        normalized_source: normalized,
    })
}

fn parse_github_url(input: &str) -> Result<GithubSource> {
    if !input.starts_with("https://") {
        return Err(invalid("GitHub URLs must use HTTPS"));
    }
    if input.contains('?') || input.contains('#') || input.contains('%') {
        return Err(invalid(
            "query, fragment, and encoded URL paths are not allowed",
        ));
    }
    let authority_and_path = &input["https://".len()..];
    let (authority, path) = authority_and_path
        .split_once('/')
        .ok_or_else(|| invalid("GitHub URL is missing a repository path"))?;
    if authority != "github.com" || authority.contains('@') || authority.contains(':') {
        return Err(invalid("only the github.com host is allowed"));
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() == 2 {
        return github_source(parts[0], parts[1], None, None);
    }
    if parts.len() >= 4 && parts[2] == "tree" {
        let reference = parts[3];
        let subpath = if parts.len() > 4 {
            Some(parts[4..].join("/"))
        } else {
            None
        };
        return github_source(parts[0], parts[1], Some(reference), subpath.as_deref());
    }
    Err(invalid(
        "expected https://github.com/<owner>/<repo>[/tree/<ref>/<path>]",
    ))
}

fn parse_shorthand(input: &str) -> Result<GithubSource> {
    if input.matches('/').count() != 1 || input.starts_with('/') || input.ends_with('/') {
        return Err(invalid("expected <owner>/<repo>"));
    }
    let (owner, repo) = input
        .split_once('/')
        .ok_or_else(|| invalid("expected <owner>/<repo>"))?;
    github_source(owner, repo, None, None)
}

/// Tokenize only whitespace, paired quotes, and backslash escapes.  Shell
/// syntax is rejected before tokenization so a command can never be executed
/// accidentally by a later layer.
fn tokenize(input: &str) -> Result<Vec<String>> {
    if has_shell_syntax(input) {
        return Err(invalid("shell metacharacters are not allowed"));
    }
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut in_token = false;

    for character in input.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            in_token = true;
            continue;
        }
        if character == '\\' {
            escaped = true;
            in_token = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            } else {
                current.push(character);
            }
            in_token = true;
            continue;
        }
        match character {
            '\'' | '"' => {
                quote = Some(character);
                in_token = true;
            }
            c if c.is_whitespace() => {
                if in_token {
                    tokens.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            _ => {
                current.push(character);
                in_token = true;
            }
        }
    }
    if escaped || quote.is_some() {
        return Err(invalid("unterminated quote or escape"));
    }
    if in_token {
        tokens.push(current);
    }
    Ok(tokens)
}

/// Parse a controlled `npx skills add ...` command.  The returned value does
/// not retain the original command line, options, or any credentials.
pub fn parse_command(input: &str) -> Result<GithubSource> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(invalid("input exceeds 4 KiB"));
    }
    let tokens = tokenize(input.trim())?;
    if tokens.len() < 5 || tokens[0] != "npx" || tokens[1] != "skills" || tokens[2] != "add" {
        return Err(invalid("expected npx skills add <source> -a codex"));
    }

    let source_token = &tokens[3];
    if source_token.starts_with('-') {
        return Err(invalid("source is missing"));
    }
    let mut app_count = 0usize;
    let mut subpath: Option<String> = None;
    let mut index = 4;
    while index < tokens.len() {
        match tokens[index].as_str() {
            "-a" => {
                app_count += 1;
                if app_count > 1 || tokens.get(index + 1).map(String::as_str) != Some("codex") {
                    return Err(invalid("-a must occur once with codex"));
                }
                index += 2;
            }
            "-s" | "--skill" => {
                if subpath.is_some() {
                    return Err(invalid("skill subpath may occur only once"));
                }
                let value = tokens
                    .get(index + 1)
                    .ok_or_else(|| invalid("skill subpath is missing"))?;
                if value.starts_with('-') {
                    return Err(invalid("skill subpath is missing"));
                }
                subpath = Some(value.clone());
                index += 2;
            }
            option if option.starts_with('-') => {
                return Err(invalid("unknown option"));
            }
            _ => return Err(invalid("extra positional argument")),
        }
    }
    if app_count != 1 {
        return Err(invalid("-a codex is required exactly once"));
    }

    let mut source = if source_token.starts_with("https://") {
        parse_github_url(source_token)?
    } else if source_token.contains("://") {
        return Err(invalid("only GitHub HTTPS sources are allowed"));
    } else {
        parse_shorthand(source_token)?
    };
    if let Some(path) = subpath {
        if source.skill_subpath.is_some() {
            return Err(invalid("skill subpath supplied twice"));
        }
        if !valid_subpath(&path) {
            return Err(invalid("invalid skill subpath"));
        }
        source.skill_subpath = Some(path.clone());
        source.normalized_source.push_str("#skill=");
        source.normalized_source.push_str(&path);
    }
    Ok(source)
}

/// Parse a source supplied by a file picker, a GitHub URL, shorthand, or the
/// controlled command form.  Local paths are accepted only when they end in
/// `.zip`; callers should still obtain them from a trusted file picker.
pub fn parse_source(input: &str) -> Result<SkillSource> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(invalid("input exceeds 4 KiB"));
    }
    let trimmed = input.trim();
    if trimmed == "npx"
        || trimmed
            .strip_prefix("npx")
            .is_some_and(|rest| rest.chars().next().is_some_and(char::is_whitespace))
    {
        return parse_command(trimmed).map(SkillSource::Github);
    }
    if trimmed.starts_with("https://") {
        return parse_github_url(trimmed).map(SkillSource::Github);
    }
    if trimmed.to_ascii_lowercase().ends_with(".zip") {
        if has_shell_syntax(trimmed) || trimmed.is_empty() {
            return Err(invalid("invalid local zip path"));
        }
        return Ok(SkillSource::LocalZip {
            path: PathBuf::from(trimmed),
        });
    }
    if trimmed.contains(' ') || trimmed.contains('\n') || trimmed.contains('\r') {
        return Err(invalid("unsupported source syntax"));
    }
    if trimmed.contains("://") {
        return Err(invalid("only GitHub HTTPS sources are allowed"));
    }
    parse_shorthand(trimmed).map(SkillSource::Github)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_url_and_shorthand() {
        let url = parse_source("https://github.com/acme/tools/tree/v1.2.3/skills/demo").unwrap();
        let SkillSource::Github(url) = url else {
            panic!()
        };
        assert_eq!(url.owner, "acme");
        assert_eq!(url.requested_ref.as_deref(), Some("v1.2.3"));
        assert_eq!(url.skill_subpath.as_deref(), Some("skills/demo"));

        let short = parse_source("acme/tools").unwrap();
        let SkillSource::Github(short) = short else {
            panic!()
        };
        assert_eq!(short.requested_ref, None);
    }

    #[test]
    fn parses_controlled_command_without_retaining_it() {
        let source =
            parse_command("npx skills add 'acme/tools' -a codex --skill \"skills/demo\"").unwrap();
        assert_eq!(
            source.normalized_source,
            "https://github.com/acme/tools#skill=skills/demo"
        );
    }

    #[test]
    fn rejects_shell_and_unsupported_options() {
        assert!(parse_command("npx skills add acme/tools -a codex --yes").is_err());
        assert!(parse_command("npx skills add acme/tools -a codex;echo pwned").is_err());
        assert!(parse_source("http://github.com/acme/tools").is_err());
        assert!(parse_source("https://evil.example/acme/tools").is_err());
    }

    #[test]
    fn accepts_local_zip_paths_with_spaces_without_executing_them() {
        let SkillSource::LocalZip { path } = parse_source("/tmp/my skills/demo.zip").unwrap()
        else {
            panic!()
        };
        assert_eq!(path.to_string_lossy(), "/tmp/my skills/demo.zip");
    }
}
