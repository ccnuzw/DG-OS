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

fn live_view() -> crate::session::SessionView {
    let mut view = crate::session::SessionView::new("/tmp/project".into());
    view.status = "running".into();
    view.active_run = Some("live-run".into());
    view
}

fn output_image(input: &ImageInput) -> Value {
    json!({"type":"image","mimeType":input.mime,"data":input.base64})
}

#[test]
fn native_output_variants_preview_only_on_explicit_live_lookup() {
    for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
        let input = encoded(format);
        let variants = [
            output_image(&input),
            json!({"type":"image","source":{"type":"base64","media_type":input.mime,"data":input.base64}}),
            json!({"type":"resource","resource":{"uri":"native://image","mimeType":input.mime,"blob":input.base64}}),
            json!({"type":"document","source":{"type":"base64","media_type":input.mime,"data":input.base64}}),
        ];
        for image in variants {
            let mut view = live_view();
            view.push("mcpToolCall", None, json!({"type":"mcpToolCall","result":{"content":[image]}}));
            let node = &view.events[0].data["result"]["content"][0];
            let node = node.get("resource").unwrap_or(node);
            let id = node["preview_resource_id"].as_str().unwrap();
            assert!(uuid::Uuid::parse_str(id).is_ok());
            assert_eq!(node["redacted"], true);
            assert_eq!(node["mimeType"], input.mime);
            let preview = view.preview_image(id).unwrap();
            assert_eq!(preview.mime, input.mime);
            assert_eq!(preview.base64, input.base64);
            assert_eq!(preview.bytes_len, STANDARD.decode(&input.base64).unwrap().len());
            assert_eq!(node["bytes_len"], preview.bytes_len);
            assert!(!view.events[0].data.to_string().contains(&input.base64));
        }
    }
}

#[test]
fn claude_user_tool_result_is_output_but_prompts_tool_arguments_and_history_are_not() {
    let input = encoded(ImageFormat::Png);
    let mut view = live_view();
    let mut data = json!({"type":"user","content":[output_image(&input),{"type":"tool_result","content":[output_image(&input)]}]});
    view.push_run("message", None, data.clone(), Some("earlier-background-run".into()));
    assert_eq!(view.events[0].data["content"][0], json!({"type":"image","redacted":true}));
    let id = view.events[0].data["content"][1]["content"][0]["preview_resource_id"].as_str().unwrap();
    assert_eq!(view.preview_image(id).unwrap().base64, input.base64);
    data["history"] = Value::Bool(true);
    view.push("message", None, data);
    view.push("userMessage", None, json!({"type":"userMessage","content":[output_image(&input)]}));
    view.push("message", None, json!({"type":"assistant","content":[{"type":"tool_use","input":{"image":output_image(&input)}}]}));
    view.push("mcpToolCall", None, json!({"type":"mcpToolCall","arguments":{"image":output_image(&input)},"input":{"image":output_image(&input)}}));
    view.push_run("mcpToolCall", None, json!({"type":"mcpToolCall","result":{"content":[output_image(&input)]}}), None);
    for event in &view.events[1..] {
        assert!(!event.data.to_string().contains("preview_resource_id"));
        assert!(!event.data.to_string().contains(&input.base64));
    }
}

#[test]
fn hydrated_history_and_replacement_do_not_create_previews() {
    let input = encoded(ImageFormat::Png);
    let data = json!({"type":"mcpToolCall","result":{"content":[output_image(&input)]}});
    let mut view = crate::session::SessionView::new("/tmp/project".into());
    view.push_run("mcpToolCall", None, data.clone(), Some("old-run".into()));
    assert!(!view.events[0].data.to_string().contains("preview_resource_id"));
    view.status = "running".into();
    view.active_run = Some("current-run".into());
    assert!(view.replace_history_event(1, "mcpToolCall", None, data));
    assert!(!view.events[0].data.to_string().contains("preview_resource_id"));
    assert!(!view.events[0].data.to_string().contains(&input.base64));
}

#[test]
fn clone_delta_and_persisted_views_keep_only_expired_opaque_references() {
    let input = encoded(ImageFormat::Png);
    let mut view = live_view();
    view.push("agentMessage", None, json!({"type":"agentMessage","content":[output_image(&input)]}));
    let id = view.events[0].data["content"][0]["preview_resource_id"].as_str().unwrap().to_owned();
    let persisted = serde_json::to_string(&view).unwrap();
    assert!(persisted.contains(&id));
    assert!(!persisted.contains(&input.base64));
    assert!(!persisted.contains("output_images"));
    let restored: crate::session::SessionView = serde_json::from_str(&persisted).unwrap();
    let (delta, _) = view.snapshot_since(None);
    for snapshot in [view.clone(), delta, restored] {
        assert_eq!(snapshot.preview_image(&id).err().as_deref(), Some("image_preview_expired"));
    }
    assert_eq!(view.preview_image(&id).unwrap().base64, input.base64);
    let mut metadata = view.events[0].data.clone();
    redact_native_images("agentMessage", &mut metadata, None);
    assert_eq!(metadata["content"][0]["preview_resource_id"], id);
}

#[test]
fn cache_limits_evict_oldest_entries_without_copying_bytes_to_clones() {
    let input = encoded(ImageFormat::Png);
    let mut cache = OutputImageCache::default();
    let mut ids = Vec::new();
    for _ in 0..OUTPUT_IMAGE_COUNT_LIMIT + 1 { ids.push(cache.insert(&input.mime, &input.base64).unwrap().0); }
    assert_eq!(cache.entries.len(), OUTPUT_IMAGE_COUNT_LIMIT);
    assert_eq!(cache.preview(&ids[0]).err().as_deref(), Some("image_preview_expired"));
    assert!(cache.preview(ids.last().unwrap()).is_ok());
    assert!(cache.clone().entries.is_empty());
    assert_eq!(cache.clone().bytes_len, 0);

    let mut bytes = STANDARD.decode(encoded(ImageFormat::Jpeg).base64).unwrap();
    bytes.resize(IMAGE_BYTES_LIMIT, 0);
    let full = STANDARD.encode(bytes);
    cache.clear();
    let mut ids = Vec::new();
    for _ in 0..5 { ids.push(cache.insert("image/jpeg", &full).unwrap().0); }
    assert_eq!(cache.bytes_len, OUTPUT_IMAGE_TOTAL_BYTES_LIMIT);
    assert_eq!(cache.entries.len(), 4);
    assert_eq!(cache.preview(&ids[0]).err().as_deref(), Some("image_preview_expired"));
    assert!(cache.preview(&ids[4]).is_ok());
}

#[test]
fn output_validation_rejects_corruption_mime_spoofs_dimensions_and_oversize_without_references() {
    let valid = encoded(ImageFormat::Png);
    let mut bytes = STANDARD.decode(&valid.base64).unwrap();
    bytes.truncate(bytes.len() / 2);
    let mut wide = Cursor::new(Vec::new());
    DynamicImage::new_rgb8(IMAGE_DIMENSION_LIMIT + 1, 1).write_to(&mut wide, ImageFormat::Png).unwrap();
    let mut pixels = Cursor::new(Vec::new());
    DynamicImage::new_rgb8(4097, 4096).write_to(&mut pixels, ImageFormat::Png).unwrap();
    let bad = [
        ("image/jpeg", valid.base64.clone()),
        ("image/svg+xml", valid.base64.clone()),
        ("image/png", "iVBORw0KGgo=".into()),
        ("image/png", STANDARD.encode(bytes)),
        ("image/png", "A".repeat(IMAGE_BYTES_LIMIT.div_ceil(3) * 4 + 1)),
        ("image/png", STANDARD.encode(vec![0; IMAGE_BYTES_LIMIT + 1])),
        ("image/png", STANDARD.encode(wide.into_inner())),
        ("image/png", STANDARD.encode(pixels.into_inner())),
    ];
    let mut cache = OutputImageCache::default();
    for (mime, data) in bad {
        let mut event = json!({"type":"agentMessage","content":[{"type":"image","mimeType":mime,"data":data}]});
        redact_native_images("agentMessage", &mut event, Some(&mut cache));
        assert_eq!(event["content"][0]["redacted"], true);
        assert_eq!(event["content"][0]["unavailable_reason"], "image_preview_unavailable");
        assert!(event["content"][0].get("preview_resource_id").is_none());
        assert!(event["content"][0].get("data").is_none());
    }
    assert!(cache.entries.is_empty());
    assert_eq!(cache.bytes_len, 0);
}

#[test]
fn textual_resources_and_documents_survive_while_binary_attachments_are_metadata_only() {
    let input = encoded(ImageFormat::Png);
    let mut view = live_view();
    let plain_document = json!({"type":"document","source":{"type":"text","data":"Authorized document text"}});
    let plain_resource = json!({"type":"resource","resource":{"uri":"file:///approved/resource","mimeType":"text/plain","text":"Authorized resource text"}});
    let structured = json!({"data":"Authorized structured data","blob":"Ordinary structured field"});
    view.push("mcpToolCall", None, json!({"type":"mcpToolCall","result":{"content":[
        plain_document, plain_resource,
        {"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"UERGIGJ5dGVz"},"path":"/native/report.pdf"},
        {"type":"image","mimeType":input.mime,"data":input.base64,"uri":"native://output","path":"/native/plot.png","url":"https://must-not-fetch.test/image","fileId":"private-id","source":{"type":"url","url":"https://must-not-fetch.test/source"}}
    ],"structuredContent":structured}}));
    let content = &view.events[0].data["result"]["content"];
    assert_eq!(content[0], plain_document);
    assert_eq!(content[1], plain_resource);
    assert_eq!(view.events[0].data["result"]["structuredContent"], structured);
    assert_eq!(content[2]["path"], "/native/report.pdf");
    assert!(content[2]["source"].get("data").is_none());
    assert_eq!(content[2]["unavailable_reason"], "image_preview_unavailable");
    assert_eq!(content[3]["uri"], "native://output");
    assert_eq!(content[3]["path"], "/native/plot.png");
    for secret in ["UERGIGJ5dGVz", "must-not-fetch", "private-id"] { assert!(!view.events[0].data.to_string().contains(secret)); }
}

#[test]
fn generation_and_final_native_output_preview_without_reading_saved_paths() {
    let input = encoded(ImageFormat::Png);
    let mut view = live_view();
    view.push("imageGeneration", None, json!({"type":"imageGeneration","result":input.base64,"savedPath":"/native/generated.png"}));
    let generated = &view.events[0].data;
    let id = generated["result"]["preview_resource_id"].as_str().unwrap();
    assert_eq!(view.preview_image(id).unwrap().base64, input.base64);
    assert_eq!(generated["savedPath"], "/native/generated.png");
    view.status = "idle".into();
    view.push("turn/completed", None, json!({"turn":{"items":[{"type":"agentMessage","content":[output_image(&input)]},{"type":"userMessage","content":[output_image(&input)]}]}}));
    let items = &view.events[1].data["turn"]["items"];
    assert!(view.preview_image(items[0]["content"][0]["preview_resource_id"].as_str().unwrap()).is_ok());
    assert!(items[1]["content"][0].get("preview_resource_id").is_none());
    view.push("imageGeneration", None, json!({"type":"imageGeneration","result":"opaque-file-id","savedPath":"/native/not-read.png"}));
    assert_eq!(view.events[2].data["result"]["unavailable_reason"], "image_preview_unavailable");
    assert!(!view.events[2].data.to_string().contains("opaque-file-id"));
}

#[test]
fn closure_unknown_state_and_explicit_clear_expire_previews() {
    let input = encoded(ImageFormat::Png);
    let mut view = live_view();
    view.push("agentMessage", None, json!({"type":"agentMessage","content":[output_image(&input)]}));
    let id = view.events[0].data["content"][0]["preview_resource_id"].as_str().unwrap().to_owned();
    for unknown in ["missing", "/etc/passwd", "https://example.test/image.png"] {
        assert_eq!(view.preview_image(unknown).err().as_deref(), Some("image_preview_expired"));
    }
    view.status = "unknown".into();
    assert_eq!(view.preview_image(&id).err().as_deref(), Some("image_preview_expired"));
    view.status = "error".into();
    assert!(view.preview_image(&id).is_ok());
    view.clear_output_image_previews();
    assert_eq!(view.preview_image(&id).err().as_deref(), Some("image_preview_expired"));
    view.push("agentMessage", None, json!({"type":"agentMessage","content":[output_image(&input)]}));
    let id = view.events[1].data["content"][0]["preview_resource_id"].as_str().unwrap().to_owned();
    view.status = "closed".into();
    view.push("host/closed", None, Value::Null);
    view.status = "idle".into();
    assert_eq!(view.preview_image(&id).err().as_deref(), Some("image_preview_expired"));
}

#[test]
fn extraction_depth_and_node_limits_do_not_capture_nested_or_excess_images() {
    let input = encoded(ImageFormat::Png);
    let mut nested = output_image(&input);
    for _ in 0..OUTPUT_IMAGE_DEPTH_LIMIT + 1 { nested = json!({"nested":nested}); }
    let mut cache = OutputImageCache::default();
    let mut deep = json!({"type":"agentMessage","content":[nested]});
    redact_native_images("agentMessage", &mut deep, Some(&mut cache));
    assert!(cache.entries.is_empty());
    assert!(!deep.to_string().contains(&input.base64));
    let mut items = vec![Value::Null; OUTPUT_IMAGE_NODE_LIMIT];
    items.push(output_image(&input));
    let mut wide = json!({"type":"agentMessage","content":items});
    redact_native_images("agentMessage", &mut wide, Some(&mut cache));
    assert!(cache.entries.is_empty());
    assert!(!wide.to_string().contains(&input.base64));
}

#[test]
fn codex_typed_output_images_accept_inline_data_urls_only_in_output_positions() {
    let input = encoded(ImageFormat::Png);
    let data_url = format!("data:{};base64,{}", input.mime, input.base64);
    let variants = [
        ("dynamicToolCall", "contentItems", json!({"type":"inputImage","imageUrl":data_url})),
        ("functionCallOutput", "output", json!({"type":"input_image","image_url":data_url})),
    ];
    for (kind, field, image) in variants {
        let mut view = live_view();
        let mut event = json!({"type":kind,"arguments":{"image":image}});
        event[field] = json!([image]);
        view.push(kind, None, event);
        let output = &view.events[0].data[field][0];
        assert_eq!(output["type"], "image");
        assert_eq!(output["mimeType"], input.mime);
        let id = output["preview_resource_id"].as_str().unwrap();
        assert_eq!(view.preview_image(id).unwrap().base64, input.base64);
        assert_eq!(view.events[0].data["arguments"]["image"], json!({"type":"image","redacted":true}));
        assert!(!view.events[0].data.to_string().contains(&data_url));
        for image in [
            json!({"type":"inputImage","imageUrl":"https://native.test/image.png"}),
            json!({"type":"input_image","file_id":"private-native-file"}),
            json!({"type":"input_image","image_url":"data:image/svg+xml;base64,PHN2Zz4="}),
        ] {
            let mut data = json!({"type":kind});
            data[field] = json!([image]);
            view.push(kind, None, data);
            let output = &view.events.last().unwrap().data[field][0];
            assert_eq!(output["unavailable_reason"], "image_preview_unavailable");
            assert!(output.get("preview_resource_id").is_none());
            assert!(!output.to_string().contains("native.test"));
            assert!(!output.to_string().contains("private-native-file"));
        }
    }
}

#[test]
fn per_event_capture_budget_preserves_first_eight_opaque_references() {
    let input = encoded(ImageFormat::Png);
    let mut view = live_view();
    view.push("agentMessage", None, json!({"type":"agentMessage","content":vec![output_image(&input); OUTPUT_IMAGE_COUNT_LIMIT + 1]}));
    let content = view.events[0].data["content"].as_array().unwrap();
    for image in &content[..OUTPUT_IMAGE_COUNT_LIMIT] {
        assert!(view.preview_image(image["preview_resource_id"].as_str().unwrap()).is_ok());
    }
    assert!(content[OUTPUT_IMAGE_COUNT_LIMIT].get("preview_resource_id").is_none());
    assert_eq!(content[OUTPUT_IMAGE_COUNT_LIMIT]["unavailable_reason"], "image_preview_unavailable");
}
