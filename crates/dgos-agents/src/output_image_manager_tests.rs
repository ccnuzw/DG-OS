use super::*;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::ImageFormat;
use serde_json::json;
use std::{io::Cursor, time::Duration};

struct Fixture {
    root: PathBuf,
    manager: AgentManager,
    task: String,
    id: String,
    generation: String,
    account: CancellationToken,
    image_base64: String,
    image_len: usize,
}

impl Fixture {
    async fn open() -> Self {
        let root = std::env::temp_dir().join(format!("dgos-output-image-preview-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let workspace = root.join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let mut image = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([17, 23, 41])))
            .write_to(&mut image, ImageFormat::Png).unwrap();
        let image = image.into_inner();
        let image_len = image.len();
        let image_base64 = STANDARD.encode(image);
        let script = r##"
while IFS= read -r line; do
request=${line#*\"id\":}; request=${request%%,*}
printf '%s\n' "$line" >> protocol
case "$line" in
CONFIG_REPLY
*'"method":"initialize"'*) printf '{"id":%s,"result":{}}\n' "$request";;
*'"method":"model/list"'*) printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$request";;
*'"method":"thread/start"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"'"$fixture_provider"'","thread":{"id":"output-image-thread","turns":[]}}}\n' "$request" "$PWD";;
*'"method":"thread/resume"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"'"$fixture_provider"'","thread":{"id":"output-image-thread","turns":[{"id":"output-image-turn","items":[{"id":"tool-output","type":"mcpToolCall","result":{"content":[{"type":"image","mimeType":"image/png","data":"IMAGE_BASE64"}]}}]}]}}}\n' "$request" "$PWD";;
*'"method":"turn/start"'*)
printf '{"id":%s,"result":{"turn":{"id":"output-image-turn"}}}\n' "$request"
printf '%s\n' '{"method":"turn/started","params":{"threadId":"output-image-thread","turn":{"id":"output-image-turn"}}}'
printf '%s\n' '{"method":"item/completed","params":{"threadId":"output-image-thread","turnId":"output-image-turn","item":{"id":"tool-output","type":"mcpToolCall","result":{"content":[{"type":"image","mimeType":"image/png","data":"IMAGE_BASE64"},{"type":"image","mimeType":"image/png","url":"https://private.invalid/secret.png"},{"type":"image","mimeType":"image/png","path":"/private/output-secret.png"}]}}}}'
if [ -f evict-previews ]; then
  count=0
  while [ "$count" -lt 8 ]; do
    printf '{"method":"item/completed","params":{"threadId":"output-image-thread","turnId":"output-image-turn","item":{"id":"tool-output-%s","type":"mcpToolCall","result":{"content":[{"type":"image","mimeType":"image/png","data":"IMAGE_BASE64"}]}}}}\n' "$count"
    count=$((count + 1))
  done
fi
printf '%s\n' '{"method":"turn/completed","params":{"threadId":"output-image-thread","turn":{"id":"output-image-turn","status":"completed","items":[]}}}'
;;
esac
done
"##.replace("CONFIG_REPLY", crate::test_support::CODEX_CONFIG_REPLY).replace("IMAGE_BASE64", &image_base64);
        let executable = root.join("codex");
        crate::test_support::codex_fixture(&executable, &script).await;
        let manager = crate::test_support::manager(&root).await;
        let task = crate::test_support::task(&manager, &workspace).await;
        let id = Uuid::new_v4().to_string();
        let account = CancellationToken::new();
        let options = SessionOptions { workspace: workspace.to_string_lossy().into_owned(), executable: Some(executable.to_string_lossy().into_owned()), resume_id: None, model: None, trust_workspace: true };
        let create = crate::test_support::create(&manager, "alice", &task, options).await;
        let opened = manager.open_session("alice", Engine::Codex, id.clone(), create, Some(task.clone()), account.clone()).await.unwrap();
        Self { root, manager, task, id, generation: opened.generation, account, image_base64, image_len }
    }

    async fn output(&self) -> (String, String) {
        let request = Uuid::new_v4().to_string();
        self.manager.send("alice", &self.id, &self.generation, request.clone(), "make an image".into(), None, vec![], vec![], vec![]).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let completed = self.manager.list_development_runs("alice", &self.task).await.unwrap().iter().any(|run| run.id == request && run.status == "completed");
                if completed && self.manager.inner.run_projections.lock().await.is_empty() { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        let snapshot = self.manager.snapshot("alice", &self.id).await.unwrap();
        let metadata = snapshot.view.events.iter().find_map(|event| event.data.pointer("/result/content/0")).unwrap();
        let preview_id = metadata["preview_resource_id"].as_str().unwrap().to_owned();
        Uuid::parse_str(&preview_id).unwrap();
        assert_eq!(metadata["mimeType"], "image/png");
        assert_eq!(metadata["bytes_len"], json!(self.image_len));
        assert_eq!(metadata["redacted"], true);
        assert!(metadata.get("data").is_none());
        assert!(matches!(snapshot.view.preview_image(&preview_id), Err(error) if error == "image_preview_expired"), "a snapshot must never copy the image cache");
        (request, preview_id)
    }

    fn protocol(&self) -> String { std::fs::read_to_string(self.root.join("workspace/protocol")).unwrap() }

    async fn finish(self) {
        assert!(self.manager.shutdown().await);
        std::fs::remove_dir_all(self.root).unwrap();
    }
}

#[tokio::test]
async fn output_preview_reads_live_bytes_without_native_dispatch_or_durable_bytes() {
    let fixture = Fixture::open().await;
    let (request, preview_id) = fixture.output().await;
    let protocol = fixture.protocol();
    let preview = fixture.manager.preview_output_image("alice", &fixture.id, &fixture.generation, &preview_id).await.unwrap();
    assert_eq!(preview.mime, "image/png");
    assert_eq!(preview.base64, fixture.image_base64);
    assert_eq!(preview.bytes_len, fixture.image_len);
    let snapshot = serde_json::to_string(&fixture.manager.snapshot("alice", &fixture.id).await.unwrap()).unwrap();
    let stored = fixture.manager.inner.repository.run("alice", &request).await.unwrap().unwrap();
    for raw in [&snapshot, &stored.events] {
        assert!(!raw.contains(&fixture.image_base64), "preview bytes must never enter snapshots or stored events");
    }
    assert_eq!(fixture.protocol(), protocol, "preview must never dispatch native IO");
    assert_eq!(fixture.manager.list_development_runs("alice", &fixture.task).await.unwrap().len(), 1);
    fixture.finish().await;
}

#[tokio::test]
async fn output_preview_rejects_other_owner_stale_generation_and_unknown_resources() {
    let fixture = Fixture::open().await;
    let (_, preview_id) = fixture.output().await;
    let protocol = fixture.protocol();
    let unknown = Uuid::new_v4().to_string();
    for (owner, generation, resource) in [
        ("bob", fixture.generation.as_str(), preview_id.as_str()),
        ("alice", "stale-generation", preview_id.as_str()),
        ("alice", fixture.generation.as_str(), unknown.as_str()),
        ("alice", fixture.generation.as_str(), "https://private.invalid/secret.png"),
        ("alice", fixture.generation.as_str(), "/private/output-secret.png"),
    ] {
        assert!(matches!(fixture.manager.preview_output_image(owner, &fixture.id, generation, resource).await, Err(error) if error == "image_preview_expired"));
    }
    assert!(fixture.manager.preview_output_image("alice", &fixture.id, &fixture.generation, &preview_id).await.is_ok(), "refused reads must not consume a valid preview");
    assert_eq!(fixture.protocol(), protocol);
    fixture.finish().await;
}

#[tokio::test]
async fn output_preview_reports_evicted_resource_expired_without_dispatch() {
    let fixture = Fixture::open().await;
    std::fs::write(fixture.root.join("workspace/evict-previews"), "").unwrap();
    let (_, expired) = fixture.output().await;
    let protocol = fixture.protocol();
    assert!(matches!(fixture.manager.preview_output_image("alice", &fixture.id, &fixture.generation, &expired).await, Err(error) if error == "image_preview_expired"));
    let snapshot = fixture.manager.snapshot("alice", &fixture.id).await.unwrap();
    let latest = snapshot.view.events.iter().rev().find_map(|event| event.data.pointer("/result/content/0/preview_resource_id").and_then(Value::as_str)).unwrap();
    assert_ne!(latest, expired);
    assert!(fixture.manager.preview_output_image("alice", &fixture.id, &fixture.generation, latest).await.is_ok());
    assert_eq!(fixture.protocol(), protocol);
    fixture.finish().await;
}

#[tokio::test]
async fn output_preview_expires_on_close_and_cannot_be_recreated_from_resumed_history() {
    let fixture = Fixture::open().await;
    let (_, preview_id) = fixture.output().await;
    assert!(fixture.manager.close_session("alice", &fixture.id).await.unwrap());
    assert!(matches!(fixture.manager.preview_output_image("alice", &fixture.id, &fixture.generation, &preview_id).await, Err(error) if error == "image_preview_expired"));
    let resumed = fixture.manager.resume_session("alice", &fixture.id, true, CancellationToken::new()).await.unwrap();
    assert_ne!(resumed.generation, fixture.generation);
    let protocol = fixture.protocol();
    for generation in [&fixture.generation, &resumed.generation] {
        assert!(matches!(fixture.manager.preview_output_image("alice", &fixture.id, generation, &preview_id).await, Err(error) if error == "image_preview_expired"));
    }
    let history = serde_json::to_string(&resumed.view.events).unwrap();
    assert!(!history.contains(&fixture.image_base64));
    assert!(!history.contains("preview_resource_id"), "native history may not repopulate live image bytes");
    assert_eq!(fixture.protocol(), protocol);
    fixture.finish().await;
}

#[tokio::test]
async fn output_preview_rejects_cancelled_account_session_and_provider_leases() {
    for cancellation in ["account", "session", "provider"] {
        let fixture = Fixture::open().await;
        let (_, preview_id) = fixture.output().await;
        match cancellation {
            "account" => fixture.account.cancel(),
            "session" => fixture.manager.session_cancellation(&fixture.id).await.unwrap().0.cancel(),
            "provider" => fixture.manager.session_cancellation(&fixture.id).await.unwrap().1.cancel(),
            _ => unreachable!(),
        }
        assert!(matches!(fixture.manager.preview_output_image("alice", &fixture.id, &fixture.generation, &preview_id).await, Err(error) if error == "image_preview_expired"));
        fixture.finish().await;
    }
}

#[tokio::test]
async fn output_preview_rejects_provider_revision_revocation_without_native_dispatch() {
    let fixture = Fixture::open().await;
    let (_, preview_id) = fixture.output().await;
    let protocol = fixture.protocol();
    fixture.manager.inner.providers.providers().toggle_provider("fixture-alice", false, "alice").await.unwrap();
    fixture.manager.inner.providers.providers().toggle_provider("fixture-alice", true, "alice").await.unwrap();
    assert!(matches!(fixture.manager.preview_output_image("alice", &fixture.id, &fixture.generation, &preview_id).await, Err(error) if matches!(error.as_str(), "image_preview_expired" | "image_preview_unavailable")));
    assert_eq!(fixture.protocol(), protocol);
    fixture.finish().await;
}

#[tokio::test]
async fn output_preview_preserves_workspace_identity_guard() {
    let fixture = Fixture::open().await;
    let (_, preview_id) = fixture.output().await;
    let workspace = fixture.root.join("workspace");
    let moved = fixture.root.join("previous-workspace");
    std::fs::rename(&workspace, &moved).unwrap();
    std::fs::create_dir(&workspace).unwrap();
    let result = fixture.manager.preview_output_image("alice", &fixture.id, &fixture.generation, &preview_id).await;
    std::fs::remove_dir(&workspace).unwrap();
    std::fs::rename(&moved, &workspace).unwrap();
    assert!(matches!(result, Err(error) if error == "image_preview_unavailable"));
    fixture.finish().await;
}
