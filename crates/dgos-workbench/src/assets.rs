//! Controlled chat attachments (V2 D133 §2).
//!
//! The renderer addresses an attachment only by its asset id. Bytes cross the IPC boundary as
//! base64 on import and preview, and nowhere else. The host checks the declared MIME type against
//! the content, and never stores or returns a filesystem path.

use std::collections::{HashMap, HashSet};

use base64::Engine as _;
use dgos_db::sea_orm::{DatabaseConnection, DbErr};
use dgos_db::{AssetRef, AssetRepository, DRAFT_LEASE_ID};
use dgos_types::ImageInput;
use serde::{Deserialize, Serialize};

use crate::chat::ChatError;

pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_ITEMS: usize = 4;
pub const MAX_TOTAL_BYTES: usize = 24 * 1024 * 1024;

const PNG_SIGNATURE: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const JPEG_SIGNATURE: &[u8] = &[0xFF, 0xD8, 0xFF];
const TEXT_HEAD: &str = "\n\n[Attachment: ";
const TEXT_SEP: &str = "]\n";
/// Draft protection: an unlinked import stays alive this long, and each preview renews it.
const DRAFT_LEASE_SECS: i64 = 24 * 60 * 60;

/// Renderer input for one import. Unknown fields are rejected.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatAssetImportInput {
    pub name: String,
    pub mime: String,
    pub base64: String,
}

#[derive(Serialize)]
pub struct ChatAssetPreview {
    pub asset: AssetRef,
    pub base64: String,
}

#[derive(Serialize)]
pub struct ChatAssetRemoved {
    pub removed: bool,
}

/// One attachment of a message, loaded for the account that owns it. Bytes stay in memory only
/// for the duration of one send or one history replay.
pub struct Attachment {
    pub name: String,
    pub mime: String,
    data: Vec<u8>,
    text: Option<String>,
}

impl Attachment {
    fn load(name: String, mime: String, data: Vec<u8>) -> Result<Self, ChatError> {
        let text = if mime.starts_with("text/") {
            Some(
                std::str::from_utf8(&data)
                    .map_err(|_| ChatError::AssetInvalid)?
                    .to_owned(),
            )
        } else {
            None
        };
        Ok(Self {
            name,
            mime,
            data,
            text,
        })
    }

    fn is_image(&self) -> bool {
        self.mime.starts_with("image/")
    }
}

fn internal(_: DbErr) -> ChatError {
    ChatError::Internal
}

fn normalize_mime(raw: &str) -> String {
    raw.split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// Returns the stored MIME type after checking the bytes against it.
fn validate(mime: &str, data: &[u8]) -> Result<&'static str, ChatError> {
    let (stored, is_text): (&'static str, bool) = match mime {
        "text/plain" => ("text/plain", true),
        "text/markdown" => ("text/markdown", true),
        "image/png" => ("image/png", false),
        "image/jpeg" => ("image/jpeg", false),
        "image/webp" => ("image/webp", false),
        _ => return Err(ChatError::AssetInvalid),
    };
    if is_text {
        if data.len() > MAX_TEXT_BYTES {
            return Err(ChatError::AssetLimitExceeded);
        }
        if data.contains(&0) || std::str::from_utf8(data).is_err() {
            return Err(ChatError::AssetInvalid);
        }
    } else {
        if data.len() > MAX_IMAGE_BYTES {
            return Err(ChatError::AssetLimitExceeded);
        }
        let signed = match stored {
            "image/png" => data.starts_with(PNG_SIGNATURE),
            "image/jpeg" => data.starts_with(JPEG_SIGNATURE),
            _ => data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP",
        };
        if !signed {
            return Err(ChatError::AssetInvalid);
        }
    }
    Ok(stored)
}

/// Keeps the last path component only. A name with control characters or no text is refused.
fn sanitize_name(raw: &str) -> Result<String, ChatError> {
    let base = raw
        .rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or("")
        .trim();
    if base.is_empty() || base.chars().count() > 255 || base.chars().any(char::is_control) {
        return Err(ChatError::AssetInvalid);
    }
    Ok(base.to_owned())
}

/// Largest base64 text that can decode to `MAX_IMAGE_BYTES`, the largest accepted file.
fn encoded_limit() -> usize {
    (MAX_IMAGE_BYTES + 2) / 3 * 4
}

/// Message text with the text attachments appended in order, as the model receives it.
pub fn expanded(content: &str, attachments: &[Attachment]) -> String {
    let mut out = String::with_capacity(expanded_len(content.len(), attachments));
    out.push_str(content);
    for item in attachments {
        if let Some(text) = &item.text {
            out.push_str(TEXT_HEAD);
            out.push_str(&item.name);
            out.push_str(TEXT_SEP);
            out.push_str(text);
        }
    }
    out
}

/// Byte length of [`expanded`], so the context budget is counted without building the string.
pub fn expanded_len(content_len: usize, attachments: &[Attachment]) -> usize {
    content_len
        + attachments
            .iter()
            .filter_map(|item| {
                item.text.as_ref().map(|text| {
                    TEXT_HEAD.len() + item.name.len() + TEXT_SEP.len() + text.len()
                })
            })
            .sum::<usize>()
}

/// Image parts of a message, in attachment order.
pub fn image_parts(attachments: &[Attachment]) -> Vec<ImageInput> {
    attachments
        .iter()
        .filter(|item| item.is_image())
        .map(|item| ImageInput {
            mime: item.mime.clone(),
            data: item.data.clone(),
        })
        .collect()
}

#[derive(Clone)]
pub struct ChatAssets {
    repo: AssetRepository,
}

impl ChatAssets {
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            repo: AssetRepository::new(db),
        }
    }

    pub async fn import(
        &self,
        user: &str,
        input: ChatAssetImportInput,
    ) -> Result<AssetRef, ChatError> {
        let name = sanitize_name(&input.name)?;
        if input.base64.len() > encoded_limit() {
            return Err(ChatError::AssetLimitExceeded);
        }
        let data = base64::engine::general_purpose::STANDARD
            .decode(input.base64.as_bytes())
            .map_err(|_| ChatError::AssetInvalid)?;
        let mime = validate(&normalize_mime(&input.mime), &data)?;
        let id = uuid::Uuid::new_v4().to_string();
        let expires_at = chrono::Utc::now().timestamp() + DRAFT_LEASE_SECS;
        self.repo
            .insert(user, &id, &name, mime, &data, expires_at)
            .await
            .map_err(internal)?;
        Ok(AssetRef {
            id,
            name,
            mime: mime.to_owned(),
            bytes_len: data.len() as i64,
        })
    }

    pub async fn preview(&self, user: &str, id: &str) -> Result<ChatAssetPreview, ChatError> {
        let asset = self
            .repo
            .refs(user, &[id.to_owned()])
            .await
            .map_err(internal)?
            .into_iter()
            .next()
            .ok_or(ChatError::AssetNotFound)?;
        let data = self
            .repo
            .bytes(user, id)
            .await
            .map_err(internal)?
            .ok_or(ChatError::AssetNotFound)?;
        let renewed = self
            .repo
            .set_lease(
                user,
                id,
                DRAFT_LEASE_ID,
                chrono::Utc::now().timestamp() + DRAFT_LEASE_SECS,
            )
            .await
            .map_err(internal)?;
        if !renewed {
            return Err(ChatError::AssetNotFound);
        }
        Ok(ChatAssetPreview {
            asset,
            base64: base64::engine::general_purpose::STANDARD.encode(data),
        })
    }

    /// Deletes an asset that no message references. The repository checks the reference in the
    /// same statement, so a reference added after the caller looked is never deleted.
    pub async fn remove(&self, user: &str, id: &str) -> Result<ChatAssetRemoved, ChatError> {
        if self.repo.remove_draft(user, id).await.map_err(internal)? == 1 {
            return Ok(ChatAssetRemoved { removed: true });
        }
        if self.repo.refs(user, &[id.to_owned()]).await.map_err(internal)?.is_empty() {
            Err(ChatError::AssetNotFound)
        } else {
            Err(ChatError::AssetReferenced)
        }
    }

    /// Attachments of an outgoing message, checked as a set: count, duplicates, ownership, total.
    pub async fn for_send(&self, user: &str, ids: &[String]) -> Result<Vec<Attachment>, ChatError> {
        if ids.len() > MAX_ITEMS {
            return Err(ChatError::AssetLimitExceeded);
        }
        let mut seen = HashSet::new();
        if !ids.iter().all(|id| seen.insert(id.as_str())) {
            return Err(ChatError::AssetInvalid);
        }
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let refs = self.repo.refs(user, ids).await.map_err(internal)?;
        if refs.len() != ids.len() {
            return Err(ChatError::AssetNotFound);
        }
        let total: i64 = refs.iter().map(|asset| asset.bytes_len).sum();
        if total > MAX_TOTAL_BYTES as i64 {
            return Err(ChatError::AssetLimitExceeded);
        }
        let mut out = Vec::with_capacity(refs.len());
        for asset in refs {
            let data = self
                .repo
                .bytes(user, &asset.id)
                .await
                .map_err(internal)?
                .ok_or(ChatError::AssetNotFound)?;
            out.push(Attachment::load(asset.name, asset.mime, data)?);
        }
        Ok(out)
    }

    /// Attachments of stored messages, keyed by message id, in attachment order.
    pub async fn for_messages(
        &self,
        user: &str,
        message_ids: &[String],
    ) -> Result<HashMap<String, Vec<Attachment>>, ChatError> {
        let mut out: HashMap<String, Vec<Attachment>> = HashMap::new();
        if message_ids.is_empty() {
            return Ok(out);
        }
        for row in self
            .repo
            .for_messages(user, message_ids)
            .await
            .map_err(internal)?
        {
            let attachment = Attachment::load(row.name, row.mime, row.data)?;
            out.entry(row.message_id).or_default().push(attachment);
        }
        Ok(out)
    }
}
