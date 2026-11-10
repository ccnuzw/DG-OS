//! Transient, byte-validated Codex image inputs. Renderer paths and remote URLs
//! are deliberately not part of this DTO; only privacy-safe metadata is serializable.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::{ImageDecoder, ImageFormat, ImageReader, Limits};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Cursor;

pub const IMAGE_COUNT_LIMIT: usize = 4;
// Inline base64 plus bounded text must fit the existing 4 MiB native echo frame.
pub const IMAGE_BYTES_LIMIT: usize = 2 * 1024 * 1024;
pub const IMAGE_TOTAL_BYTES_LIMIT: usize = 2 * 1024 * 1024;
pub const IMAGE_DIMENSION_LIMIT: u32 = 8192;
pub const IMAGE_PIXEL_LIMIT: u64 = 16 * 1024 * 1024;
const IMAGE_DECODE_BYTES_LIMIT: u64 = 128 * 1024 * 1024;

// No Debug/Serialize: image bytes must never enter logs or stored run inputs.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageInput {
    pub mime: String,
    pub base64: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ImageMetadata {
    pub mime: String,
    pub bytes_len: usize,
    pub width: u32,
    pub height: u32,
    pub sha256: String,
}

pub struct ValidatedImage {
    data_url: String,
    metadata: ImageMetadata,
}
impl ValidatedImage {
    pub fn native_input(&self) -> Value { json!({"type":"image","url":self.data_url}) }
    pub fn fingerprint(&self) -> &str { &self.metadata.sha256 }
    pub fn metadata(&self) -> ImageMetadata { self.metadata.clone() }
}

pub fn validate_images(inputs: &[ImageInput]) -> Result<Vec<ValidatedImage>, String> {
    if inputs.len() > IMAGE_COUNT_LIMIT { return Err("agent_image_count_limit".into()); }
    let mut declared_total = 0usize;
    for input in inputs {
        if input.base64.is_empty() || input.base64.len() > IMAGE_BYTES_LIMIT.div_ceil(3) * 4 { return Err("agent_image_bytes_limit".into()); }
        let padding = if input.base64.ends_with("==") { 2 } else { usize::from(input.base64.ends_with('=')) };
        let decoded_len = input.base64.len().div_ceil(4) * 3 - padding;
        if decoded_len > IMAGE_BYTES_LIMIT { return Err("agent_image_bytes_limit".into()); }
        declared_total += decoded_len;
        if declared_total > IMAGE_TOTAL_BYTES_LIMIT { return Err("agent_image_total_limit".into()); }
    }
    let mut images = Vec::with_capacity(inputs.len());
    for input in inputs {
        let expected = match input.mime.as_str() {
            "image/png" => ImageFormat::Png,
            "image/jpeg" => ImageFormat::Jpeg,
            "image/webp" => ImageFormat::WebP,
            _ => return Err("agent_image_type".into()),
        };
        // STANDARD rejects whitespace, URL-safe variants, invalid trailing bits
        // and noncanonical padding. Aggregate size was bounded before decoding.
        let bytes = STANDARD.decode(&input.base64).map_err(|_| "agent_image_invalid")?;
        if bytes.is_empty() || bytes.len() > IMAGE_BYTES_LIMIT { return Err("agent_image_bytes_limit".into()); }
        if image::guess_format(&bytes).map_err(|_| "agent_image_invalid")? != expected {
            return Err("agent_image_type".into());
        }
        let mut limits = Limits::default();
        limits.max_image_width = Some(IMAGE_DIMENSION_LIMIT);
        limits.max_image_height = Some(IMAGE_DIMENSION_LIMIT);
        limits.max_alloc = Some(IMAGE_DECODE_BYTES_LIMIT);
        let mut reader = ImageReader::with_format(Cursor::new(&bytes), expected);
        reader.limits(limits);
        let decoder = reader.into_decoder().map_err(|_| "agent_image_invalid")?;
        let (width, height) = decoder.dimensions();
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > IMAGE_PIXEL_LIMIT {
            return Err("agent_image_dimensions".into());
        }
        // Full decoding rejects truncated/corrupt payloads, not just forged magic
        // bytes. Decoder limits and the pixel budget bound decompression memory.
        image::DynamicImage::from_decoder(decoder).map_err(|_| "agent_image_invalid")?;
        let metadata = ImageMetadata { mime: input.mime.clone(), bytes_len: bytes.len(), width, height, sha256: format!("{:x}", Sha256::digest(&bytes)) };
        images.push(ValidatedImage { data_url: format!("data:{};base64,{}", input.mime, input.base64), metadata });
    }
    Ok(images)
}

/// Apply before native events/history are exposed or persisted. Image URL/path,
/// file IDs and embedded byte payloads are not useful transcript data and may be
/// secrets. This never fetches, reads or forwards a native path or URL.
pub fn redact_image_inputs(value: &mut Value) {
    match value {
        Value::Object(object) => {
            let image = object.get("type").and_then(Value::as_str).is_some_and(|kind| matches!(kind, "image" | "localImage" | "input_image" | "image_url"));
            if image {
                *value = json!({"type":"image","redacted":true});
            } else {
                for child in object.values_mut() { redact_image_inputs(child); }
            }
        }
        Value::Array(items) => { for item in items { redact_image_inputs(item); } }
        Value::String(text) if text.get(..11).is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:image/")) => {
            *text = "[image omitted]".into();
        }
        _ => {},
    }
}

#[cfg(test)]
#[path = "images_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "images_manager_tests.rs"]
mod manager_tests;
