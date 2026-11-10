use super::*;
use image::{DynamicImage, RgbImage};

fn encoded(format: ImageFormat) -> ImageInput {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::from_pixel(2, 3, image::Rgb([12, 34, 56]))).write_to(&mut bytes, format).unwrap();
    let mime = match format { ImageFormat::Png => "image/png", ImageFormat::Jpeg => "image/jpeg", ImageFormat::WebP => "image/webp", _ => unreachable!() };
    ImageInput { mime: mime.into(), base64: STANDARD.encode(bytes.into_inner()) }
}

#[test]
fn validates_all_supported_formats_and_builds_native_inline_inputs() {
    for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
        let input = encoded(format);
        let images = validate_images(std::slice::from_ref(&input)).unwrap();
        let image = &images[0];
        assert_eq!((image.metadata().width, image.metadata().height), (2, 3));
        assert_eq!(image.metadata().mime, input.mime);
        assert_eq!(image.fingerprint().len(), 64);
        assert_eq!(image.native_input(), json!({"type":"image","url":format!("data:{};base64,{}", input.mime, input.base64)}));
        assert!(!serde_json::to_string(&image.metadata()).unwrap().contains(&input.base64));
    }
}

#[test]
fn fingerprints_are_content_based_and_image_order_is_retained() {
    let first = encoded(ImageFormat::Png);
    let second = encoded(ImageFormat::Jpeg);
    let images = validate_images(&[first.clone(), second.clone()]).unwrap();
    let reordered = validate_images(&[second, first.clone()]).unwrap();
    assert_ne!(images[0].fingerprint(), images[1].fingerprint());
    assert_eq!(images[0].fingerprint(), reordered[1].fingerprint());
    assert_eq!(images[0].fingerprint(), validate_images(&[first]).unwrap()[0].fingerprint());
}

#[test]
fn rejects_urls_paths_unknown_fields_and_mime_spoofing() {
    for value in [json!({"url":"https://example.org/image.png"}), json!({"path":"/private/image.png"}), json!({"mime":"image/png","base64":"AA==","path":"/etc/passwd"})] {
        assert!(serde_json::from_value::<ImageInput>(value).is_err());
    }
    let mut input = encoded(ImageFormat::Png);
    input.mime = "image/jpeg".into();
    assert_eq!(validate_images(&[input]).err().unwrap(), "agent_image_type");
    for mime in ["image/svg+xml", "image/gif", "IMAGE/PNG", "text/plain"] {
        assert_eq!(validate_images(&[ImageInput { mime: mime.into(), base64: "AA==".into() }]).err().unwrap(), "agent_image_type");
    }
}

#[test]
fn rejects_malformed_and_truncated_bytes_even_with_valid_signature() {
    for base64 in ["", "%%%%", "data:image/png;base64,AA==", "iVBORw0KGgo=", "AA==\n"] {
        assert!(validate_images(&[ImageInput { mime: "image/png".into(), base64: base64.into() }]).is_err());
    }
    let input = encoded(ImageFormat::Png);
    let mut bytes = STANDARD.decode(input.base64).unwrap();
    bytes.truncate(bytes.len() / 2);
    assert!(validate_images(&[ImageInput { mime: input.mime, base64: STANDARD.encode(bytes) }]).is_err());
}

#[test]
fn enforces_encoded_decoded_count_aggregate_and_dimension_limits() {
    assert!(validate_images(&[]).unwrap().is_empty());
    let input = encoded(ImageFormat::Png);
    assert_eq!(validate_images(&vec![input.clone(); IMAGE_COUNT_LIMIT + 1]).err().unwrap(), "agent_image_count_limit");
    assert_eq!(validate_images(&[ImageInput { mime: input.mime.clone(), base64: "A".repeat(IMAGE_BYTES_LIMIT.div_ceil(3) * 4 + 1) }]).err().unwrap(), "agent_image_bytes_limit");
    assert_eq!(validate_images(&[ImageInput { mime: input.mime.clone(), base64: STANDARD.encode(vec![0; IMAGE_BYTES_LIMIT + 1]) }]).err().unwrap(), "agent_image_bytes_limit");
    let mut padded = STANDARD.decode(input.base64).unwrap();
    padded.resize(IMAGE_BYTES_LIMIT, 0);
    let full = ImageInput { mime: input.mime, base64: STANDARD.encode(padded) };
    assert_eq!(validate_images(&vec![full; 4]).err().unwrap(), "agent_image_total_limit");
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::new_rgb8(IMAGE_DIMENSION_LIMIT + 1, 1).write_to(&mut bytes, ImageFormat::Png).unwrap();
    assert!(validate_images(&[ImageInput { mime: "image/png".into(), base64: STANDARD.encode(bytes.into_inner()) }]).is_err());
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::new_rgb8(4097, 4096).write_to(&mut bytes, ImageFormat::Png).unwrap();
    assert_eq!(validate_images(&[ImageInput { mime: "image/png".into(), base64: STANDARD.encode(bytes.into_inner()) }]).err().unwrap(), "agent_image_dimensions");
}

#[test]
fn recursively_redacts_image_bytes_urls_paths_and_native_file_ids_from_history() {
    let mut event = json!({"type":"userMessage","content":[{"type":"text","text":"describe this"},{"type":"image","url":"data:image/png;base64,secret"},{"type":"localImage","path":"/private/photo.png"},{"type":"image","fileId":"private-id"}],"nested":{"raw":"DATA:IMAGE/PNG;base64,secret"}});
    redact_image_inputs(&mut event);
    assert_eq!(event["content"][0]["text"], "describe this");
    for index in 1..4 { assert_eq!(event["content"][index], json!({"type":"image","redacted":true})); }
    assert_eq!(event["nested"]["raw"], "[image omitted]");
    let stored = event.to_string();
    for secret in ["secret", "/private", "private-id", "data:image"] { assert!(!stored.contains(secret)); }
    let first = event.clone();
    redact_image_inputs(&mut event);
    assert_eq!(event, first);
}
