#![cfg(unix)]
use super::*;
use crate::{AgentManager, Engine, SessionOptions};
use std::{path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn image_input(color: u8) -> ImageInput {
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([color, 11, 17])))
        .write_to(&mut bytes, ImageFormat::Png).unwrap();
    ImageInput { mime: "image/png".into(), base64: STANDARD.encode(bytes.into_inner()) }
}
struct Fixture { root: PathBuf, manager: AgentManager, task: String, id: String, generation: String }
impl Fixture {
    async fn open(input: &ImageInput) -> Self {
        let root = std::env::temp_dir().join(format!("dgos-native-images-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let executable = root.join("codex");
        let script = r##"
while IFS= read -r line; do
request=${line#*\"id\":}; request=${request%%,*}
case "$line" in
CONFIG_REPLY
*'"method":"initialize"'*) printf '{"id":%s,"result":{}}\n' "$request";;
*'"method":"model/list"'*) printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$request";;
*'"method":"thread/start"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"'"$fixture_provider"'","thread":{"id":"image-thread","turns":[]}}}\n' "$request" "$PWD";;
*'"method":"thread/resume"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"'"$fixture_provider"'","thread":{"id":"image-thread","turns":[{"id":"image-turn","items":[{"id":"restored-image","type":"userMessage","content":[{"type":"image","url":"IMAGE_URL"},{"type":"localImage","path":"/private/image-secret.png"}]}]}]}}}\n' "$request" "$PWD";;
*'"method":"turn/start"'*)
printf '%s\n' "$line" >> native-input.jsonl
printf '{"id":%s,"result":{"turn":{"id":"image-turn"}}}\n' "$request"
printf '%s\n' '{"method":"turn/started","params":{"threadId":"image-thread","turn":{"id":"image-turn"}}}'
printf '%s\n' '{"method":"item/completed","params":{"threadId":"image-thread","turnId":"image-turn","item":{"id":"image-echo","type":"userMessage","content":[{"type":"image","url":"IMAGE_URL"}]}}}'
printf '%s\n' '{"method":"turn/completed","params":{"threadId":"image-thread","turn":{"id":"image-turn","status":"completed","items":[{"id":"terminal-image","type":"userMessage","content":[{"type":"image","url":"IMAGE_URL"},{"type":"localImage","path":"/private/image-secret.png"}]}]}}}'
;;
esac
done
"##.replace("CONFIG_REPLY", crate::test_support::CODEX_CONFIG_REPLY).replace("IMAGE_URL", &format!("data:{};base64,{}", input.mime, input.base64));
        crate::test_support::codex_fixture(&executable, &script).await;
        let manager = crate::test_support::manager(&root).await;
        let task = crate::test_support::task(&manager, &root).await;
        let id = Uuid::new_v4().to_string();
        let options = SessionOptions { workspace: root.to_string_lossy().into_owned(), executable: Some(executable.to_string_lossy().into_owned()), resume_id: None, model: None, trust_workspace: true };
        let create = crate::test_support::create(&manager, "alice", &task, options).await;
        let session = manager.open_session("alice", Engine::Codex, id.clone(), create, Some(task.clone()), CancellationToken::new()).await.unwrap();
        Self { root, manager, task, id, generation: session.generation }
    }
    fn sent(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root.join("native-input.jsonl")).unwrap_or_default().lines().map(|line| serde_json::from_str(line).unwrap()).collect()
    }
    async fn completed(&self, request: &str) -> crate::RunRecord {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(run) = self.manager.list_development_runs("alice", &self.task).await.unwrap().into_iter().find(|run| run.id == request && run.status == "completed") { return run; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap()
    }
    async fn finish(self) {
        assert!(self.manager.shutdown().await);
        std::fs::remove_dir_all(self.root).unwrap();
    }
}

#[tokio::test]
async fn image_only_native_send_is_content_fingerprinted_idempotent_and_privacy_safe_in_durable_history() {
    let image = image_input(33);
    let fixture = Fixture::open(&image).await;
    let request = Uuid::new_v4().to_string();
    fixture.manager.send("alice", &fixture.id, &fixture.generation, request.clone(), String::new(), None, vec![], vec![], vec![image.clone()]).await.unwrap();
    let run = fixture.completed(&request).await;
    fixture.manager.send("alice", &fixture.id, &fixture.generation, request.clone(), String::new(), None, vec![], vec![], vec![image.clone()]).await.unwrap();
    assert_eq!(fixture.sent().len(), 1, "a repeated receipt must never replay native input");
    let native = fixture.sent();
    assert_eq!(native[0]["params"]["input"], json!([{"type":"image","url":format!("data:{};base64,{}", image.mime, image.base64)}]));
    assert_eq!(fixture.manager.send("alice", &fixture.id, &fixture.generation, request.clone(), String::new(), None, vec![], vec![], vec![image_input(99)]).await.unwrap_err(), "request_conflict");
    let metadata = validate_images(std::slice::from_ref(&image)).unwrap().remove(0).metadata();
    assert!(serde_json::to_string(&run.events).unwrap().contains(&metadata.sha256), "host image metadata survives without source bytes");
    let persisted = fixture.manager.inner.repository.run("alice", &request).await.unwrap().unwrap();
    for raw in [serde_json::to_string(&run.events).unwrap(), persisted.events, serde_json::to_string(&fixture.manager.snapshot("alice", &fixture.id).await.unwrap().view.events).unwrap()] {
        for private in [image.base64.as_str(), "data:image/", "/private/image-secret.png"] { assert!(!raw.contains(private), "image source leaked into history"); }
    }
    assert_eq!(fixture.sent().len(), 1);
    assert!(fixture.manager.close_session("alice", &fixture.id).await.unwrap());
    let resumed = fixture.manager.resume_session("alice", &fixture.id, true, CancellationToken::new()).await.unwrap();
    let restored = serde_json::to_string(&resumed.view.events).unwrap();
    assert!(restored.contains("redacted"));
    assert!(!restored.contains(&image.base64));
    assert!(!restored.contains("/private/image-secret.png"));
    fixture.finish().await;
}

#[tokio::test]
async fn invalid_image_never_creates_a_run_and_image_identity_is_owner_generation_scoped() {
    let image = image_input(42);
    let fixture = Fixture::open(&image).await;
    for (mime, base64) in [("image/png", "aHR0cHM6Ly9leGFtcGxlLmNvbQ==".to_owned()), ("image/jpeg", image.base64.clone()), ("image/svg+xml", STANDARD.encode(b"<svg/>"))] {
        let request = Uuid::new_v4().to_string();
        assert!(fixture.manager.send("alice", &fixture.id, &fixture.generation, request.clone(), String::new(), None, vec![], vec![], vec![ImageInput { mime: mime.into(), base64 }]).await.is_err());
        assert!(fixture.manager.inner.repository.run("alice", &request).await.unwrap().is_none());
    }
    for (owner, generation, expected) in [("bob", fixture.generation.as_str(), "session_not_found"), ("alice", "wrong-generation", "stale_generation")] {
        let request = Uuid::new_v4().to_string();
        assert_eq!(fixture.manager.send(owner, &fixture.id, generation, request.clone(), String::new(), None, vec![], vec![], vec![image.clone()]).await.unwrap_err(), expected);
        assert!(fixture.manager.inner.repository.run("alice", &request).await.unwrap().is_none());
    }
    assert!(fixture.sent().is_empty());
    fixture.finish().await;
}

#[tokio::test]
async fn native_image_input_order_is_part_of_the_request_identity() {
    let first = image_input(1);
    let second = image_input(2);
    let fixture = Fixture::open(&first).await;
    let request = Uuid::new_v4().to_string();
    fixture.manager.send("alice", &fixture.id, &fixture.generation, request.clone(), "compare".into(), None, vec![], vec![], vec![first.clone(), second.clone()]).await.unwrap();
    fixture.completed(&request).await;
    assert_eq!(fixture.manager.send("alice", &fixture.id, &fixture.generation, request.clone(), "compare".into(), None, vec![], vec![], vec![second, first]).await.unwrap_err(), "request_conflict");
    let native = fixture.sent();
    assert_eq!(native.len(), 1);
    assert_eq!(native[0]["params"]["input"][0], json!({"type":"text","text":"compare"}));
    assert_eq!(native[0]["params"]["input"][1]["type"], "image");
    assert_eq!(native[0]["params"]["input"][2]["type"], "image");
    assert_ne!(native[0]["params"]["input"][1]["url"], native[0]["params"]["input"][2]["url"]);
    fixture.finish().await;
}

#[tokio::test]
async fn identical_text_with_distinct_images_preserves_both_turns_without_duplicate_native_echoes() {
    let first = image_input(21);
    let second = image_input(22);
    let fixture = Fixture::open(&first).await;
    for input in [first.clone(), second.clone()] {
        let request = Uuid::new_v4().to_string();
        fixture.manager.send("alice", &fixture.id, &fixture.generation, request.clone(), "same caption".into(), None, vec![], vec![], vec![input]).await.unwrap();
        fixture.completed(&request).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while !fixture.manager.inner.run_projections.lock().await.is_empty() { tokio::time::sleep(Duration::from_millis(10)).await; }
        }).await.unwrap();
    }
    let view = fixture.manager.snapshot("alice", &fixture.id).await.unwrap().view;
    let messages: Vec<_> = view.events.iter().filter(|event| event.kind == "userMessage").collect();
    assert_eq!(messages.len(), 2, "exactly one host image message per admitted turn, never text-based deduplication");
    assert!(messages.iter().all(|event| event.text.as_deref() == Some("same caption")));
    assert_ne!(messages[0].data["images"][0]["sha256"], messages[1].data["images"][0]["sha256"]);
    assert_ne!(messages[0].run, messages[1].run);
    assert_eq!(fixture.sent().len(), 2);
    fixture.finish().await;
}
