//! Restricted GitHub HTTPS adapter for external Skill packages.
//!
//! The adapter resolves a requested ref to an immutable commit before fetching
//! the archive. It never invokes Git, accepts credentials, follows arbitrary
//! redirects, or writes a downloaded archive to the final Skill directory.

use crate::{GithubSource, Result, SkillError, ValidatedExternalPackage};
use reqwest::{redirect::Policy, Client, StatusCode, Url};
use serde::Deserialize;
use std::time::Duration;
use tokio::time::{sleep, timeout};

const API_HOST: &str = "api.github.com";
const CODELOAD_HOST: &str = "codeload.github.com";
const MAX_REDIRECTS: usize = 1;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const PREVIEW_TIMEOUT: Duration = Duration::from_secs(30);
const RETRY_DELAY: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub struct ResolvedGithubSource {
    pub source: GithubSource,
    pub resolved_commit: String,
    pub archive: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct GithubAdapter {
    client: Client,
    api_base: Url,
    codeload_base: Url,
}

#[derive(Debug, Deserialize)]
struct RepositoryResponse {
    default_branch: String,
}

#[derive(Debug, Deserialize)]
struct CommitResponse {
    sha: String,
}

fn invalid(message: impl Into<String>) -> SkillError {
    SkillError::InvalidManifest(format!("invalid GitHub source: {}", message.into()))
}

fn remote_error(status: StatusCode, context: &str) -> SkillError {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            SkillError::RegistryUnreachable(format!("GitHub authentication required for {context}"))
        }
        StatusCode::NOT_FOUND => {
            SkillError::SkillNotFound(format!("GitHub source not found: {context}"))
        }
        _ => {
            SkillError::RegistryUnreachable(format!("GitHub returned HTTP {status} for {context}"))
        }
    }
}

fn redirect_policy() -> Policy {
    Policy::custom(move |attempt| {
        let target = attempt.url();
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        if target.scheme() == "https"
            && target
                .host_str()
                .is_some_and(|host| host == API_HOST || host == CODELOAD_HOST)
        {
            attempt.follow()
        } else {
            attempt.stop()
        }
    })
}

fn client_for() -> Result<Client> {
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent("DG-OS-Skills/1")
        .redirect(redirect_policy())
        .build()
        .map_err(SkillError::HttpError)
}

impl GithubAdapter {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: client_for()?,
            api_base: Url::parse("https://api.github.com/")
                .map_err(|error| invalid(error.to_string()))?,
            codeload_base: Url::parse("https://codeload.github.com/")
                .map_err(|error| invalid(error.to_string()))?,
        })
    }

    #[cfg(test)]
    fn with_test_endpoints(api_base: Url, codeload_base: Url) -> Result<Self> {
        let client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent("DG-OS-Skills-test/1")
            .redirect(Policy::none())
            .build()
            .map_err(SkillError::HttpError)?;
        Ok(Self {
            client,
            api_base,
            codeload_base,
        })
    }

    /// Resolve a source ref to the canonical 40-character commit SHA.
    pub async fn resolve_ref(&self, source: &GithubSource) -> Result<String> {
        timeout(REQUEST_TIMEOUT, self.resolve_ref_inner(source))
            .await
            .map_err(|_| {
                SkillError::RegistryUnreachable("GitHub ref resolution timed out".into())
            })?
    }

    async fn resolve_ref_inner(&self, source: &GithubSource) -> Result<String> {
        let repo_path = format!("repos/{}/{}/", source.owner, source.repo);
        let endpoint = if let Some(reference) = source.requested_ref.as_deref() {
            format!("{repo_path}commits/{reference}")
        } else {
            format!("{repo_path}")
        };
        let url = self
            .api_base
            .join(&endpoint)
            .map_err(|error| invalid(error.to_string()))?;
        let response = self.get_with_retry(url).await?;
        if !response.status().is_success() {
            return Err(remote_error(response.status(), &source.normalized_source));
        }
        if source.requested_ref.is_none() {
            let repository: RepositoryResponse = response
                .json()
                .await
                .map_err(|error| SkillError::RegistryUnreachable(error.to_string()))?;
            let default_branch = repository.default_branch;
            if default_branch.is_empty() || default_branch.contains('/') {
                return Err(invalid("GitHub default branch is unsupported in V1"));
            }
            let url = self
                .api_base
                .join(&format!("{repo_path}commits/{default_branch}"))
                .map_err(|error| invalid(error.to_string()))?;
            let response = self.get_with_retry(url).await?;
            if !response.status().is_success() {
                return Err(remote_error(response.status(), &source.normalized_source));
            }
            return parse_commit(response, &source.normalized_source).await;
        }
        parse_commit(response, &source.normalized_source).await
    }

    pub async fn download_archive(&self, source: &GithubSource, commit: &str) -> Result<Vec<u8>> {
        timeout(REQUEST_TIMEOUT, self.download_archive_inner(source, commit))
            .await
            .map_err(|_| {
                SkillError::RegistryUnreachable("GitHub archive download timed out".into())
            })?
    }

    async fn download_archive_inner(&self, source: &GithubSource, commit: &str) -> Result<Vec<u8>> {
        if commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(invalid("resolved commit is not a 40-character SHA"));
        }
        let endpoint = format!("{}/{}/zip/{}", source.owner, source.repo, commit);
        let url = self
            .codeload_base
            .join(&endpoint)
            .map_err(|error| invalid(error.to_string()))?;
        let response = self.get_with_retry(url).await?;
        if !response.status().is_success() {
            return Err(remote_error(response.status(), &source.normalized_source));
        }
        if response
            .content_length()
            .is_some_and(|length| length > crate::MAX_EXTERNAL_ARCHIVE_BYTES as u64)
        {
            return Err(invalid("GitHub archive exceeds 8 MiB"));
        }
        let mut output = Vec::new();
        let mut response = response;
        while let Some(chunk) = response.chunk().await.map_err(SkillError::HttpError)? {
            if output.len().saturating_add(chunk.len()) > crate::MAX_EXTERNAL_ARCHIVE_BYTES {
                return Err(invalid("GitHub archive exceeds 8 MiB"));
            }
            output.extend_from_slice(&chunk);
        }
        Ok(output)
    }

    pub async fn fetch(&self, source: GithubSource) -> Result<ResolvedGithubSource> {
        timeout(PREVIEW_TIMEOUT, self.fetch_inner(source))
            .await
            .map_err(|_| SkillError::RegistryUnreachable("GitHub preview timed out".into()))?
    }

    async fn fetch_inner(&self, source: GithubSource) -> Result<ResolvedGithubSource> {
        let resolved_commit = self.resolve_ref_inner(&source).await?;
        let archive = self
            .download_archive_inner(&source, &resolved_commit)
            .await?;
        Ok(ResolvedGithubSource {
            source,
            resolved_commit,
            archive,
        })
    }

    async fn get_with_retry(&self, url: Url) -> Result<reqwest::Response> {
        let mut retried = false;
        loop {
            let response = match self.client.get(url.clone()).send().await {
                Ok(response) => response,
                Err(_error) if !retried => {
                    retried = true;
                    sleep(RETRY_DELAY).await;
                    continue;
                }
                Err(error) => return Err(SkillError::HttpError(error)),
            };
            let retry_delay = if response.status().is_server_error() && !retried {
                Some(RETRY_DELAY)
            } else if response.status() == StatusCode::TOO_MANY_REQUESTS && !retried {
                let delay = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok())
                    .map(Duration::from_secs)
                    .unwrap_or(RETRY_DELAY);
                Some(delay.min(REQUEST_TIMEOUT))
            } else {
                None
            };
            if let Some(delay) = retry_delay {
                retried = true;
                drop(response);
                sleep(delay).await;
                continue;
            }
            return Ok(response);
        }
    }

    pub async fn fetch_and_validate(
        &self,
        source: GithubSource,
    ) -> Result<(ResolvedGithubSource, ValidatedExternalPackage)> {
        let resolved = self.fetch(source).await?;
        let package = crate::validate_external_zip_at_subpath(
            &resolved.archive,
            resolved.source.skill_subpath.as_deref(),
        )?;
        Ok((resolved, package))
    }
}

async fn parse_commit(response: reqwest::Response, context: &str) -> Result<String> {
    let payload: CommitResponse = response
        .json()
        .await
        .map_err(|error| SkillError::RegistryUnreachable(error.to_string()))?;
    if payload.sha.len() != 40 || !payload.sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid(format!(
            "GitHub returned an invalid commit for {context}"
        )));
    }
    Ok(payload.sha.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse_source, SkillSource};
    use std::io::{Cursor, Write};
    use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

    #[test]
    fn builds_immutable_commit_archive_paths() {
        let SkillSource::Github(source) = parse_source(
            "https://github.com/ccnuzw/daoge-skills/tree/spec-docs-pro-v2.1.0/skills/spec-docs-pro",
        )
        .unwrap() else {
            panic!()
        };
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let url = Url::parse("https://codeload.github.com/")
            .unwrap()
            .join(&format!("{}/{}/zip/{commit}", source.owner, source.repo))
            .unwrap();
        assert_eq!(url.as_str(), "https://codeload.github.com/ccnuzw/daoge-skills/zip/0123456789abcdef0123456789abcdef01234567");
    }

    #[test]
    fn rejects_non_sha_archive_downloads() {
        assert!(
            "not-a-sha".len() != 40 || !"not-a-sha".bytes().all(|byte| byte.is_ascii_hexdigit())
        );
    }

    #[tokio::test]
    async fn resolves_pinned_ref_and_fetches_fixture_archive() {
        let mut server = mockito::Server::new_async().await;
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let _repo_mock = server
            .mock("GET", "/repos/acme/repo/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"default_branch":"main"}"#)
            .create_async()
            .await;
        let _ref_mock = server
            .mock("GET", "/repos/acme/repo/commits/main")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(r#"{{"sha":"{commit}"}}"#))
            .create_async()
            .await;
        let _archive_mock = server
            .mock("GET", format!("/acme/repo/zip/{commit}").as_str())
            .with_status(200)
            .with_header("content-type", "application/zip")
            .with_body(fixture_archive())
            .create_async()
            .await;
        let base = Url::parse(&format!("{}/", server.url())).unwrap();
        let adapter = GithubAdapter::with_test_endpoints(base.clone(), base).unwrap();
        let SkillSource::Github(source) = parse_source("acme/repo").unwrap() else {
            panic!()
        };
        let (resolved, package) = adapter.fetch_and_validate(source).await.unwrap();
        assert_eq!(resolved.resolved_commit, commit);
        assert!(!resolved.archive.is_empty());
        assert_eq!(package.metadata.name, "Demo");
    }

    fn fixture_archive() -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(&mut output);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer.start_file("repo-main/SKILL.md", options).unwrap();
        writer
            .write_all(b"---\nname: Demo\ndescription: Fixture\n---\n")
            .unwrap();
        writer.finish().unwrap();
        output.into_inner()
    }
}
