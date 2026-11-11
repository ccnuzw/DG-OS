//! Transient, byte-validated native image inputs and output previews. Only
//! privacy-safe metadata is retained in session snapshots and stored runs.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::{ImageDecoder, ImageFormat, ImageReader, Limits};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::VecDeque, io::Cursor};

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
        declared_total += decoded_image_len(&input.base64)?;
        if declared_total > IMAGE_TOTAL_BYTES_LIMIT { return Err("agent_image_total_limit".into()); }
    }
    let mut images = Vec::with_capacity(inputs.len());
    for input in inputs {
        let (bytes, width, height) = decode_image(&input.mime, &input.base64)?;
        let metadata = ImageMetadata { mime: input.mime.clone(), bytes_len: bytes.len(), width, height, sha256: format!("{:x}", Sha256::digest(&bytes)) };
        images.push(ValidatedImage { data_url: format!("data:{};base64,{}", input.mime, input.base64), metadata });
    }
    Ok(images)
}

fn decoded_image_len(encoded: &str) -> Result<usize, String> {
    if encoded.is_empty() || encoded.len() > IMAGE_BYTES_LIMIT.div_ceil(3) * 4 { return Err("agent_image_bytes_limit".into()); }
    let padding = if encoded.ends_with("==") { 2 } else { usize::from(encoded.ends_with('=')) };
    let len = encoded.len().div_ceil(4) * 3 - padding;
    if len > IMAGE_BYTES_LIMIT { return Err("agent_image_bytes_limit".into()); }
    Ok(len)
}

fn raster_format(mime: &str) -> Option<ImageFormat> {
    match mime {
        "image/png" => Some(ImageFormat::Png),
        "image/jpeg" => Some(ImageFormat::Jpeg),
        "image/webp" => Some(ImageFormat::WebP),
        _ => None,
    }
}

fn decode_image(mime: &str, encoded: &str) -> Result<(Vec<u8>, u32, u32), String> {
    decoded_image_len(encoded)?;
    let expected = raster_format(mime).ok_or("agent_image_type")?;
    // STANDARD rejects whitespace, URL-safe variants, invalid trailing bits
    // and noncanonical padding. Bound the allocation before decoding.
    let bytes = STANDARD.decode(encoded).map_err(|_| "agent_image_invalid")?;
    if bytes.is_empty() || bytes.len() > IMAGE_BYTES_LIMIT { return Err("agent_image_bytes_limit".into()); }
    if image::guess_format(&bytes).map_err(|_| "agent_image_invalid")? != expected { return Err("agent_image_type".into()); }
    let mut limits = Limits::default();
    limits.max_image_width = Some(IMAGE_DIMENSION_LIMIT);
    limits.max_image_height = Some(IMAGE_DIMENSION_LIMIT);
    limits.max_alloc = Some(IMAGE_DECODE_BYTES_LIMIT);
    let mut reader = ImageReader::with_format(Cursor::new(&bytes), expected);
    reader.limits(limits);
    let decoder = reader.into_decoder().map_err(|_| "agent_image_invalid")?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > IMAGE_PIXEL_LIMIT { return Err("agent_image_dimensions".into()); }
    // Full decoding rejects truncated/corrupt payloads as well as forged magic.
    image::DynamicImage::from_decoder(decoder).map_err(|_| "agent_image_invalid")?;
    Ok((bytes, width, height))
}

pub const OUTPUT_IMAGE_COUNT_LIMIT: usize = 8;
pub const OUTPUT_IMAGE_TOTAL_BYTES_LIMIT: usize = 8 * 1024 * 1024;
const OUTPUT_IMAGE_DEPTH_LIMIT: usize = 32;
const OUTPUT_IMAGE_NODE_LIMIT: usize = 4096;

/// Returned only by an explicit request against the original live session.
#[derive(Serialize)]
pub struct OutputImagePreview {
    pub mime: String,
    pub base64: String,
    pub bytes_len: usize,
}

struct CachedOutputImage {
    id: String,
    mime: String,
    bytes: Vec<u8>,
}

#[derive(Default)]
pub(crate) struct OutputImageCache {
    entries: VecDeque<CachedOutputImage>,
    bytes_len: usize,
}

// Session snapshots and run persistence clone the view. Never copy live bytes
// into those values, even when a caller does not immediately serialize them.
impl Clone for OutputImageCache {
    fn clone(&self) -> Self { Self::default() }
}
impl std::fmt::Debug for OutputImageCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("OutputImageCache").field("entries", &self.entries.len()).field("bytes_len", &self.bytes_len).finish()
    }
}
impl OutputImageCache {
    pub(crate) fn clear(&mut self) { self.entries.clear(); self.bytes_len = 0; }

    pub(crate) fn preview(&self, id: &str) -> Result<OutputImagePreview, String> {
        let image = self.entries.iter().find(|image| image.id == id).ok_or("image_preview_expired")?;
        Ok(OutputImagePreview { mime: image.mime.clone(), base64: STANDARD.encode(&image.bytes), bytes_len: image.bytes.len() })
    }

    fn insert(&mut self, mime: &str, encoded: &str) -> Result<(String, usize), String> {
        let (bytes, _, _) = decode_image(mime, encoded)?;
        while self.entries.len() >= OUTPUT_IMAGE_COUNT_LIMIT || self.bytes_len + bytes.len() > OUTPUT_IMAGE_TOTAL_BYTES_LIMIT {
            let Some(expired) = self.entries.pop_front() else { break; };
            self.bytes_len -= expired.bytes.len();
        }
        let id = uuid::Uuid::new_v4().to_string();
        let len = bytes.len();
        self.bytes_len += len;
        self.entries.push_back(CachedOutputImage { id: id.clone(), mime: mime.into(), bytes });
        Ok((id, len))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImagePosition { Input, Envelope, UserOutput, Tool, Output }

struct ImageScanBudget { nodes: usize, images: usize, bytes: usize }
impl ImageScanBudget {
    fn new() -> Self { Self { nodes: OUTPUT_IMAGE_NODE_LIMIT, images: OUTPUT_IMAGE_COUNT_LIMIT, bytes: OUTPUT_IMAGE_TOTAL_BYTES_LIMIT } }
    fn admit(&mut self, encoded: &str) -> bool {
        let Ok(len) = decoded_image_len(encoded) else { return false; };
        if self.images == 0 || len > self.bytes { return false; }
        self.images -= 1;
        self.bytes -= len;
        true
    }
}

/// Sanitize a native transcript event before exposing it. A live cache is
/// supplied only for run-bound output, never for hydrated history or prompts.
pub(crate) fn redact_native_images(kind: &str, value: &mut Value, cache: Option<&mut OutputImageCache>) {
    let position = match kind {
        "message" | "item/started" | "item/completed" | "turn/completed" => ImagePosition::Envelope,
        "content_block_start" | "agentMessage" | "functionCallOutput" | "mcpToolCall" | "dynamicToolCall" | "imageGeneration" | "result" => ImagePosition::Output,
        _ => ImagePosition::Input,
    };
    let mut cache = cache;
    let mut remaining = ImageScanBudget::new();
    redact_native_value(value, position, 0, &mut remaining, &mut cache, true);
}

fn redact_native_value(value: &mut Value, mut position: ImagePosition, depth: usize, remaining: &mut ImageScanBudget, cache: &mut Option<&mut OutputImageCache>, mut capture: bool) {
    if depth > OUTPUT_IMAGE_DEPTH_LIMIT {
        *value = json!({"redacted":true,"unavailable_reason":"image_preview_unavailable"});
        return;
    }
    capture &= remaining.nodes > 0;
    remaining.nodes = remaining.nodes.saturating_sub(1);
    match value {
        Value::Object(object) => {
            capture &= object.get("history") != Some(&Value::Bool(true));
            match object.get("type").and_then(Value::as_str) {
                Some("userMessage" | "hookPrompt" | "localImage" | "tool_use" | "server_tool_use") => position = ImagePosition::Input,
                Some("user") if position != ImagePosition::Input => position = ImagePosition::UserOutput,
                Some("assistant" | "agentMessage") if position != ImagePosition::Input => position = ImagePosition::Output,
                Some("tool_result") if position != ImagePosition::Input => position = ImagePosition::Tool,
                Some("mcpToolCall" | "dynamicToolCall" | "functionCallOutput" | "imageGeneration") if position != ImagePosition::Input => position = ImagePosition::Tool,
                _ => {},
            }
            let image = object.get("type").and_then(Value::as_str).is_some_and(|kind| matches!(kind, "image" | "localImage" | "input_image" | "inputImage" | "image_url"));
            if image && position != ImagePosition::Output {
                *value = json!({"type":"image","redacted":true});
                return;
            }
            if image { object.insert("type".into(), Value::String("image".into())); }
            let mime = image_mime(object);
            let document = object.get("type").and_then(Value::as_str) == Some("document");
            let source_base64 = object.get("source").and_then(|source| source.get("type")).and_then(Value::as_str) == Some("base64");
            let source_text = object.get("source").and_then(|source| source.get("type")).and_then(Value::as_str) == Some("text");
            let binary = image || object.contains_key("blob") && object.contains_key("mimeType")
                || document && !source_text && (source_base64 || object.contains_key("base64") || !mime.starts_with("text/") && (object.contains_key("data") || mime.starts_with("image/")))
                || !source_text && position == ImagePosition::Output && mime.starts_with("image/") && (object.contains_key("data") || object.contains_key("base64") || source_base64);
            if binary {
                redact_binary_node(object, position == ImagePosition::Output && capture, cache, remaining);
            }
            if object.get("type").and_then(Value::as_str) == Some("resource") {
                if let Some(resource) = object.get_mut("resource").and_then(Value::as_object_mut).filter(|resource| resource.contains_key("blob")) {
                    redact_binary_node(resource, position == ImagePosition::Output && capture, cache, remaining);
                }
            }
            // Codex imageGeneration may return inline base64 without a content
            // block. Infer only its raster magic; URLs/file IDs are never read.
            if object.get("type").and_then(Value::as_str) == Some("imageGeneration") {
                if let Some(Value::String(encoded)) = object.get_mut("result") {
                    let inline = inline_data_url(encoded);
                    let mime = inline.map(|(mime, _)| mime).or_else(|| inline_raster_mime(encoded)).unwrap_or("");
                    let encoded = inline.map_or(encoded.as_str(), |(_, encoded)| encoded);
                    let mut image = serde_json::Map::new();
                    image.insert("type".into(), Value::String("image".into()));
                    image.insert("mimeType".into(), Value::String(mime.into()));
                    set_preview_metadata(&mut image, mime, Some(encoded), position == ImagePosition::Tool && capture, cache, remaining);
                    object.insert("result".into(), Value::Object(image));
                }
            }
            for (key, child) in object.iter_mut() {
                let child_position = if matches!(key.as_str(), "arguments" | "input" | "parameters") { ImagePosition::Input }
                    else if position == ImagePosition::Tool {
                        if matches!(key.as_str(), "result" | "output" | "content" | "contentItems" | "structuredContent") { ImagePosition::Output } else { ImagePosition::Input }
                    } else { position };
                redact_native_value(child, child_position, depth + 1, remaining, cache, capture);
            }
        },
        Value::Array(items) => {
            for item in items { redact_native_value(item, position, depth + 1, remaining, cache, capture); }
        },
        Value::String(text) if text.get(..11).is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:image/")) => { *text = "[image omitted]".into(); },
        _ => {},
    }
}

fn inline_raster_mime(encoded: &str) -> Option<&'static str> {
    let prefix = STANDARD.decode(encoded.get(..16)?).ok()?;
    match image::guess_format(&prefix).ok()? {
        ImageFormat::Png => Some("image/png"),
        ImageFormat::Jpeg => Some("image/jpeg"),
        ImageFormat::WebP => Some("image/webp"),
        _ => None,
    }
}

fn inline_data_url(value: &str) -> Option<(&str, &str)> {
    let header_end = value.get(..value.len().min(160))?.find(',')?;
    let mime = value[..header_end].strip_prefix("data:")?.strip_suffix(";base64")?;
    (mime.len() <= 128).then_some((mime, &value[header_end + 1..]))
}

fn image_mime(object: &serde_json::Map<String, Value>) -> &str {
    object.get("mimeType").or_else(|| object.get("mime_type")).or_else(|| object.get("mime"))
        .or_else(|| object.get("source").and_then(|source| source.get("media_type")))
        .and_then(Value::as_str).filter(|mime| mime.len() <= 128).unwrap_or("")
}

fn redact_binary_node(object: &mut serde_json::Map<String, Value>, capture: bool, cache: &mut Option<&mut OutputImageCache>, remaining: &mut ImageScanBudget) {
    let declared_mime = image_mime(object).to_owned();
    let image = object.get("type").and_then(Value::as_str) == Some("image");
    let url = if image {
        let url = object.remove("imageUrl").or_else(|| object.remove("image_url")).or_else(|| object.remove("url"));
        object.remove("url"); object.remove("imageUrl"); object.remove("image_url");
        object.remove("fileId"); object.remove("file_id");
        if let Some(source) = object.get_mut("source").and_then(Value::as_object_mut) { source.remove("url"); source.remove("file_id"); }
        url
    } else { None };
    // Move payload strings out of the event; never clone unbounded native data.
    let data = object.remove("data");
    let blob = object.remove("blob");
    let base64 = object.remove("base64");
    let source_data = object.get_mut("source").and_then(Value::as_object_mut).and_then(|source| source.remove("data"));
    let source_base64 = object.get("source").and_then(|source| source.get("type")).and_then(Value::as_str) == Some("base64");
    let inline = url.as_ref().and_then(Value::as_str).and_then(inline_data_url);
    let mime = if declared_mime.is_empty() { inline.map_or("", |(mime, _)| mime) } else { &declared_mime };
    let encoded = data.as_ref().or(blob.as_ref()).or(base64.as_ref()).and_then(Value::as_str)
        .or_else(|| source_base64.then(|| source_data.as_ref().and_then(Value::as_str)).flatten())
        .or_else(|| inline.map(|(_, encoded)| encoded));
    let capture = capture && inline.is_none_or(|(inline_mime, _)| inline_mime == mime);
    set_preview_metadata(object, mime, encoded, capture, cache, remaining);
}

fn set_preview_metadata(object: &mut serde_json::Map<String, Value>, mime: &str, encoded: Option<&str>, capture: bool, cache: &mut Option<&mut OutputImageCache>, remaining: &mut ImageScanBudget) {
    object.insert("redacted".into(), Value::Bool(true));
    if !mime.is_empty() && mime.len() <= 128 { object.insert("mimeType".into(), Value::String(mime.into())); }
    let Some(encoded) = encoded else {
        // A snapshot may already contain an opaque reference. Keep it so the UI
        // can report expiration after generation replacement or session closure.
        if !object.contains_key("preview_resource_id") { object.insert("unavailable_reason".into(), Value::String("image_preview_unavailable".into())); }
        return;
    };
    object.remove("preview_resource_id");
    object.remove("bytes_len");
    if let Ok(len) = decoded_image_len(encoded) { object.insert("bytes_len".into(), Value::from(len)); }
    let result = if capture {
        if remaining.admit(encoded) {
            cache.as_deref_mut().ok_or_else(|| "image_preview_expired".to_string()).and_then(|cache| cache.insert(mime, encoded))
        } else { Err("image_preview_unavailable".into()) }
    } else { Err("image_preview_expired".into()) };
    match result {
        Ok((id, len)) => {
            object.insert("preview_resource_id".into(), Value::String(id));
            object.insert("bytes_len".into(), Value::from(len));
            object.remove("unavailable_reason");
        },
        Err(error) => {
            let reason = if raster_format(mime).is_none() { "image_preview_unavailable" } else if error == "image_preview_expired" { "image_preview_expired" } else { "image_preview_unavailable" };
            object.insert("unavailable_reason".into(), Value::String(reason.into()));
        },
    }
}

/// Apply to input echoes and hydrated history before they are exposed or stored.
/// This never fetches, reads or forwards a native path or URL.
pub fn redact_image_inputs(value: &mut Value) {
    let mut remaining = ImageScanBudget::new();
    redact_native_value(value, ImagePosition::Envelope, 0, &mut remaining, &mut None, false);
}

#[cfg(test)]
#[path = "images_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "images_manager_tests.rs"]
mod manager_tests;
