use super::*;
use crate::{AgentManager, Engine, SessionOptions};
use std::{future::Future, path::PathBuf, pin::Pin, task::Poll};

#[test]
fn envelope_classification_rejects_ambiguous_and_malformed_messages() {
    for value in [
        json!(null), json!([]), json!({}),
        json!({"id":1,"method":"request","result":null}),
        json!({"id":1,"result":null,"error":{"code":-1,"message":"bad"}}),
        json!({"id":1}), json!({"result":null}),
        json!({"id":true,"result":null}), json!({"id":1.5,"result":null}),
        json!({"id":null,"method":"request"}), json!({"method":7}),
        json!({"method":""}), json!({"jsonrpc":"1.0","method":"notice"}),
        json!({"method":"notice","params":null}), json!({"method":"notice","params":true}),
        json!({"id":1,"error":null}), json!({"id":1,"error":{"code":-1}}),
    ] {
        assert!(classify(&value).is_err(), "accepted malformed envelope: {value}");
    }
    for id in [json!(1), json!(-1), json!("native-id"), Value::Null] {
        assert!(matches!(classify(&json!({"id":id,"result":null})), Ok(Envelope::Response)));
    }
    assert!(matches!(classify(&json!({"id":"native-id","method":"approval"})), Ok(Envelope::Request)));
    assert!(matches!(classify(&json!({"method":"notice"})), Ok(Envelope::Notification)));
}

#[cfg(unix)]
struct Fixture {
    root: PathBuf,
    manager: AgentManager,
    wire: Transport,
    events: mpsc::Receiver<Value>,
}

#[cfg(unix)]
impl Fixture {
    async fn new(body: &str) -> Self {
        let root = std::env::temp_dir().join(format!("dgos-transport-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let manager = crate::test_support::manager(&root).await;
        let task_id = crate::test_support::task(&manager, &root).await;
        let options = SessionOptions { workspace: root.to_string_lossy().into(), executable: None, resume_id: None, model: None, trust_workspace: true };
        let create = crate::test_support::create(&manager, "alice", &task_id, options.clone()).await;
        let project_id = manager.development_workspace("alice").await.unwrap().projects[0].id.clone();
        let project = manager.inner.repository.project("alice", &project_id).await.unwrap();
        let lease = CancellationToken::new();
        let launch = manager.inner.providers.prepare("alice", &uuid::Uuid::new_v4().to_string(), &task_id, &project, Engine::Codex, &options, create.project_revision, create.provider, &lease).await.unwrap();
        let executable = root.join("wire");
        crate::test_support::prepare_shell_fixture(&executable, body).await;
        let mut command = Command::new(executable);
        command.current_dir(&root);
        let (wire, events) = Transport::spawn(command, launch.lease.cancellation_token(), ProcessDiagnostic::new("agent_start", "codex"), launch.lease.clone()).unwrap();
        Self { root, manager, wire, events }
    }

    async fn event(&mut self) -> Value {
        tokio::time::timeout(Duration::from_secs(3), self.events.recv()).await.unwrap().unwrap()
    }

    async fn held<F: Future>(&mut self, request: Pin<&mut F>) {
        tokio::select! {
            _ = request => panic!("held request unexpectedly completed"),
            event = self.event() => assert_eq!(event["method"], "held"),
        }
    }

    async fn finish(self) {
        assert!(self.wire.close().await);
        assert!(self.wire.wait_stopped().await);
        assert!(self.manager.shutdown().await);
        std::fs::remove_dir_all(self.root).unwrap();
    }
}

async fn poll_pending<F: Future>(mut future: Pin<&mut F>) {
    std::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    }).await;
}

#[cfg(unix)]
#[tokio::test]
async fn orphan_and_late_responses_never_become_requests_or_outbound_errors() {
    let mut fixture = Fixture::new(r#"
while IFS= read -r line; do
printf '%s\n' "$line" >> protocol
case "$line" in
*'"method":"probe"'*)
 printf '%s\n' '{"id":1,"result":{"accepted":true}}' '{"id":1,"result":null}' '{"id":1,"error":{"code":-1,"message":"late"}}' '{"id":999,"result":null}' '{"id":"native-orphan","error":{"code":-1,"message":"foreign"}}' '{"id":null,"result":null}' '{"method":"barrier"}' ;;
*'"method":"finish"'*) printf '%s\n' '{"method":"finished"}';;
esac
done
"#).await;
    assert_eq!(fixture.wire.request("probe", json!({})).await.unwrap(), json!({"accepted":true}));
    // This mirrors consumers that reject every incoming server request. If an
    // orphan were routed as an event, it would produce an outbound error here.
    loop {
        let event = fixture.event().await;
        if event.get("id").is_some() {
            fixture.wire.write(json!({"id":event["id"],"error":{"code":-32601,"message":"unsupported"}})).await.unwrap();
        }
        if event["method"] == "barrier" { break; }
    }
    fixture.wire.write(json!({"method":"finish"})).await.unwrap();
    assert_eq!(fixture.event().await["method"], "finished");
    let sent = std::fs::read_to_string(fixture.root.join("protocol")).unwrap();
    let sent: Vec<Value> = sent.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0]["id"], 1);
    assert!(sent.iter().all(|value| value.get("error").is_none()));
    assert!(fixture.wire.native_available());
    fixture.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn server_string_ids_are_preserved_without_consuming_host_numeric_ids() {
    let mut fixture = Fixture::new(r#"
while IFS= read -r line; do
case "$line" in
*'"method":"probe"'*) printf '%s\n' '{"id":"1","method":"approval","params":{"question":"allow?"}}' '{"id":1,"result":"host-reply"}';;
*'"id":"1"'*) printf '%s\n' '{"method":"server-answered"}';;
esac
done
"#).await;
    assert_eq!(fixture.wire.request("probe", json!({})).await.unwrap(), "host-reply");
    let request = fixture.event().await;
    assert_eq!(request["id"], "1");
    assert_eq!(request["method"], "approval");
    fixture.wire.write(json!({"id":request["id"],"result":{"approved":true}})).await.unwrap();
    assert_eq!(fixture.event().await["method"], "server-answered");
    fixture.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn exhausted_request_ids_fail_closed_without_wrapping() {
    let fixture = Fixture::new(r#"
while IFS= read -r line; do
printf '%s\n' '{"id":18446744073709551614,"result":"last-reply"}'
done
"#).await;
    fixture.wire.next.store(u64::MAX - 1, Ordering::Relaxed);
    assert_eq!(fixture.wire.request("last", json!({})).await.unwrap(), "last-reply");
    assert_eq!(fixture.wire.request("exhausted", json!({})).await.unwrap_err(), "closed");
    assert_eq!(fixture.wire.next.load(Ordering::Relaxed), u64::MAX);
    assert!(fixture.wire.pending.lock().unwrap().replies.is_empty());
    fixture.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn dropped_requests_release_pending_and_late_replies_are_discarded() {
    let mut fixture = Fixture::new(r#"
while IFS= read -r line; do
case "$line" in
*'"method":"hold"'*) printf '%s\n' '{"method":"held"}';;
*'"method":"release"'*) printf '%s\n' '{"id":1,"result":"too-late"}' '{"id":1,"error":{"code":-1,"message":"too-late"}}' '{"method":"released"}';;
esac
done
"#).await;
    let wire = fixture.wire.clone();
    let mut request = Box::pin(wire.request("hold", json!({})));
    fixture.held(request.as_mut()).await;
    assert_eq!(wire.pending.lock().unwrap().replies.len(), 1);
    drop(request);
    assert!(wire.pending.lock().unwrap().replies.is_empty());
    wire.write(json!({"method":"release"})).await.unwrap();
    assert_eq!(fixture.event().await["method"], "released");
    assert!(wire.native_available());
    fixture.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn pending_registration_is_bounded_and_dropped_queued_writes_release_it() {
    let fixture = Fixture::new("while IFS= read -r line; do :; done\n").await;
    let wire = fixture.wire.clone();
    // Fill synchronously so every following request is suspended on enqueue,
    // independent of Tokio's cooperative polling budget.
    while wire.input.try_send(json!({"method":"prefill"})).is_ok() {}
    // Keep every request future alive, including requests suspended on the
    // bounded writer queue. Registration itself must never become unbounded.
    let mut requests = Vec::new();
    for _ in 0..MAX_PENDING {
        let mut request = Box::pin(wire.request("hold", json!({})));
        poll_pending(request.as_mut()).await;
        requests.push(request);
    }
    assert_eq!(wire.pending.lock().unwrap().replies.len(), MAX_PENDING);
    assert_eq!(wire.input.capacity(), 0);
    let queued = requests.pop().unwrap();
    drop(queued);
    assert_eq!(wire.pending.lock().unwrap().replies.len(), MAX_PENDING - 1);
    let mut replacement = Box::pin(wire.request("replacement", json!({})));
    poll_pending(replacement.as_mut()).await;
    assert_eq!(wire.pending.lock().unwrap().replies.len(), MAX_PENDING);
    assert_eq!(wire.request("over-limit", json!({})).await.unwrap_err(), "connection_lost");
    assert!(wire.connection_lost.is_cancelled());
    assert!(wire.pending.lock().unwrap().replies.is_empty());
    drop(replacement);
    drop(requests);
    fixture.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn saturated_event_queue_fails_closed_and_releases_pending_rpcs() {
    let fixture = Fixture::new(r#"
while IFS= read -r line; do
n=0
while [ "$n" -le EVENT_LIMIT ]; do
 printf '%s\n' '{"method":"notice","params":{}}'
 n=$((n + 1))
done
done
"#.replace("EVENT_LIMIT", &EVENT_CAPACITY.to_string()).as_str()).await;
    let wire = fixture.wire.clone();
    let request = wire.request("overflow", json!({}));
    assert_eq!(tokio::time::timeout(Duration::from_secs(3), request).await.unwrap().unwrap_err(), "connection_lost");
    assert!(wire.connection_lost.is_cancelled());
    assert!(!wire.native_available());
    assert!(wire.pending.lock().unwrap().replies.is_empty());
    assert_eq!(fixture.events.len(), EVENT_CAPACITY);
    fixture.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn connection_loss_and_pending_failure_precede_slow_process_reap() {
    let mut fixture = Fixture::new(r#"
trap '' TERM
while IFS= read -r line; do
case "$line" in
*'"method":"hold"'*) printf '%s\n' '{"method":"held"}';;
*'"method":"lose"'*) exec 1>/dev/null; while :; do sleep 10; done;;
esac
done
"#).await;
    let wire = fixture.wire.clone();
    let mut request = Box::pin(wire.request("hold", json!({})));
    fixture.held(request.as_mut()).await;
    wire.write(json!({"method":"lose"})).await.unwrap();
    tokio::time::timeout(Duration::from_millis(250), wire.connection_lost.cancelled()).await.unwrap();
    assert!(!wire.native_available());
    assert!(!wire.stopped());
    assert!(!wire.finished.is_cancelled());
    assert_eq!(tokio::time::timeout(Duration::from_millis(250), request).await.unwrap().unwrap_err(), "connection_lost");
    assert!(wire.pending.lock().unwrap().replies.is_empty());
    assert!(tokio::time::timeout(Duration::from_secs(3), wire.wait_stopped()).await.unwrap());
    assert!(wire.stopped());
    fixture.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn malformed_input_and_writer_failure_signal_loss_without_waiting_for_reap() {
    for body in [
        "trap '' TERM\nIFS= read -r line\nprintf '%s\\n' '{\"id\":1,\"method\":\"ambiguous\",\"result\":null}'\nwhile :; do sleep 10; done\n",
        "trap '' TERM\nexec 0</dev/null\nprintf '%s\\n' '{\"method\":\"stdin-closed\"}'\nwhile :; do sleep 10; done\n",
    ] {
        let mut fixture = Fixture::new(body).await;
        if body.contains("exec 0") {
            assert_eq!(fixture.event().await["method"], "stdin-closed");
        }
        let error = tokio::time::timeout(Duration::from_secs(3), fixture.wire.request("probe", json!({}))).await.unwrap().unwrap_err();
        assert!(matches!(error.as_str(), "connection_lost" | "closed"));
        assert!(fixture.wire.connection_lost.is_cancelled());
        assert!(!fixture.wire.native_available());
        assert!(!fixture.wire.stopped());
        assert!(fixture.wire.pending.lock().unwrap().replies.is_empty());
        fixture.finish().await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn stopping_immediately_invalidates_native_availability_and_fails_pending() {
    let mut fixture = Fixture::new("trap '' TERM\nwhile IFS= read -r line; do printf '%s\\n' '{\"method\":\"held\"}'; done\n").await;
    let wire = fixture.wire.clone();
    let mut request = Box::pin(wire.request("hold", json!({})));
    fixture.held(request.as_mut()).await;
    wire.stop.cancel();
    assert!(wire.connection_lost.is_cancelled());
    assert!(!wire.native_available());
    assert_eq!(tokio::time::timeout(Duration::from_millis(250), request).await.unwrap().unwrap_err(), "connection_lost");
    fixture.finish().await;
}
