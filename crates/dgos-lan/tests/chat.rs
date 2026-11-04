//! FR-010 AC09 and B10-47～B10-54: the remote chat routes over the real FR-006
//! `ChatManager`, `ProviderManager` and `SkillsManager`, with a loopback
//! OpenAI-compatible upstream (no external Provider is called).

mod common;

use common::*;
use dgos_db::sea_orm::{ConnectionTrait, DbBackend, Statement};
use dgos_workbench::{ChatTargetInput, CreateChatSessionInput};
use serde_json::{json, Value};

fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

async fn send(env: &Env, cookie: &str, chat: &str, content: &str, id: &str) -> Reply {
    env.post(
        &format!("/api/v1/lan/chat/sessions/{chat}/messages"),
        json!({ "content": content, "client_request_id": id }),
    )
    .cookie(cookie)
    .send()
    .await
}

#[tokio::test]
async fn smoke_targets_create_send_list_and_replay() {
    let env = env().await;
    let cookie = env.paired(PEER).await;

    let targets = env.get("/api/v1/lan/targets").cookie(&cookie).send().await;
    assert_eq!(targets.status, 200);
    assert_eq!(
        targets.body,
        json!({
            "providers": [{"id": env.provider_id, "name": "Loopback", "models": [{"id": "chat-model", "name": "chat-model"}]}],
            "skills": []
        })
    );
    for secret in [
        "base_url",
        "credential_ref",
        "test-only-secret",
        "127.0.0.1",
        "config",
    ] {
        assert!(!targets.text.contains(secret), "{secret} leaked");
    }

    let chat = env.chat(&cookie).await;
    let list = env
        .get("/api/v1/lan/chat/sessions")
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(list.body["items"].as_array().unwrap().len(), 1);
    assert_eq!(list.body["items"][0]["id"], chat.as_str());
    assert_eq!(
        list.body["items"][0]["provider_id"],
        env.provider_id.as_str()
    );

    env.up.script(reply("你好，远程"));
    let id = uuid();
    let ack = send(&env, &cookie, &chat, "hello", &id).await;
    assert_eq!(ack.status, 202, "{}", ack.text);
    assert_eq!(ack.body["task_id"], id.as_str());
    assert_eq!(ack.body["replayed"], false);
    let items = env.settled(&cookie, &chat).await;
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["role"], "user");
    assert_eq!(items[0]["content"], "hello");
    assert_eq!(items[1]["status"], "completed");
    assert_eq!(items[1]["content"], "你好，远程");
    assert_eq!(items[1]["total_tokens"], 5);
    assert_eq!(env.up.hits(), 1);

    // Idempotency (FR-006): same id + content replays without a new upstream call.
    let replay = send(&env, &cookie, &chat, "hello", &id).await;
    assert_eq!(replay.status, 202);
    assert_eq!(replay.body["replayed"], true);
    assert_eq!(
        replay.body["assistant_message_id"],
        ack.body["assistant_message_id"]
    );
    assert_eq!(env.up.hits(), 1);
    assert_eq!(env.messages(&cookie, &chat).await.len(), 2);
    let conflict = send(&env, &cookie, &chat, "different", &id).await;
    assert_eq!((conflict.status, conflict.code()), (409, Some(1006011)));
    assert!(conflict.body.get("detail").is_none());

    // Created through the remote, visible on the desktop as an ordinary chat.
    let desktop = env
        .host
        .chat
        .list_sessions(ALICE, Default::default())
        .await
        .unwrap();
    assert_eq!(desktop.items.len(), 1);
}

#[tokio::test]
async fn the_remote_sees_only_chats_of_this_pairing() {
    let env = env().await;
    // A desktop chat of the same user.
    let desktop = env
        .host
        .chat
        .create_session(
            &env.host.providers,
            &*env.host.skills.read().await,
            ALICE,
            CreateChatSessionInput {
                target: Some(ChatTargetInput { provider_id: Some(env.provider_id.clone()),
                model_id: Some("chat-model".into()), skill_ids: vec![], generation: Default::default(), no_skill: true, }),
            },
        )
        .await
        .unwrap();
    let a = env.paired(PEER).await;
    let mine = env.chat(&a).await;
    let b = env.paired("192.168.1.11").await;
    let theirs = {
        let reply = env
            .post("/api/v1/lan/chat/sessions", env.target())
            .peer("192.168.1.11")
            .cookie(&b)
            .send()
            .await;
        reply.body["id"].as_str().unwrap().to_owned()
    };
    let list = env.get("/api/v1/lan/chat/sessions").cookie(&a).send().await;
    let ids: Vec<_> = list.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids, std::slice::from_ref(&mine));
    for other in [&desktop.id, &theirs] {
        let read = env
            .get(&format!("/api/v1/lan/chat/sessions/{other}/messages"))
            .cookie(&a)
            .send()
            .await;
        assert_eq!((read.status, read.code()), (404, Some(1006001)));
        let write = send(&env, &a, other, "x", &uuid()).await;
        assert_eq!((write.status, write.code()), (404, Some(1006001)));
    }
    // A re-pairing starts with an empty set (D099 item 5).
    let code = env.code();
    let again = env
        .post("/api/v1/lan/pair", json!({ "code": code }))
        .cookie(&a)
        .send()
        .await;
    let a2 = again.cookie().unwrap();
    let list = env
        .get("/api/v1/lan/chat/sessions")
        .cookie(&a2)
        .send()
        .await;
    assert_eq!(list.body["items"], json!([]));
}

#[tokio::test]
async fn targets_are_fixed_and_no_mutation_or_tool_fields_are_accepted() {
    let env = env().await;
    let cookie = env.paired(PEER).await;
    let chat = env.chat(&cookie).await;
    for body in [
        json!({"content": "x", "client_request_id": uuid(), "tools": [{"server_id": "s", "tool_name": "t"}]}),
        json!({"content": "x", "client_request_id": uuid(), "accept_destination": "abc"}),
        json!({"content": "x", "client_request_id": uuid(), "session_id": chat}),
    ] {
        let reply = env
            .post(&format!("/api/v1/lan/chat/sessions/{chat}/messages"), body)
            .cookie(&cookie)
            .send()
            .await;
        assert_eq!((reply.status, reply.text.as_str()), (400, ""));
    }
    // Neither a skill nor "no skill": the target must be chosen at creation.
    let undecided = env
        .post(
            "/api/v1/lan/chat/sessions",
            json!({"target": {"provider_id": env.provider_id, "model_id": "chat-model", "skill_id": null, "no_skill": false}}),
        )
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!((undecided.status, undecided.code()), (409, Some(1006013)));
    let unknown_model = env
        .post(
            "/api/v1/lan/chat/sessions",
            json!({"target": {"provider_id": env.provider_id, "model_id": "nope", "skill_id": null, "no_skill": true}}),
        )
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(
        (unknown_model.status, unknown_model.code()),
        (400, Some(1006015))
    );
    let missing = env
        .post("/api/v1/lan/chat/sessions", json!({}))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(missing.status, 400);
    // No rename, delete or target routes exist.
    for (method, path) in [
        ("DELETE", format!("/api/v1/lan/chat/sessions/{chat}")),
        ("POST", format!("/api/v1/lan/chat/sessions/{chat}/rename")),
        ("POST", format!("/api/v1/lan/chat/sessions/{chat}/target")),
    ] {
        let reply = env
            .call(method, &path)
            .origin(Some(&env.origin()))
            .cookie(&cookie)
            .send()
            .await;
        assert_eq!(reply.status, 404, "{method} {path}");
    }
}

#[tokio::test]
async fn a_changed_destination_is_1006012_without_detail() {
    let env = env().await;
    let cookie = env.paired(PEER).await;
    let chat = env.chat(&cookie).await;
    send(&env, &cookie, &chat, "first", &uuid()).await;
    env.settled(&cookie, &chat).await;
    // Same type, another port: a different destination fingerprint.
    let moved = format!("http://127.0.0.1:{}/v1", env.up.addr.port() + 1);
    env.db
        .execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "UPDATE providers SET base_url = ? WHERE id = ?",
            [moved.into(), env.provider_id.clone().into()],
        ))
        .await
        .unwrap();
    let reply = send(&env, &cookie, &chat, "second", &uuid()).await;
    assert_eq!((reply.status, reply.code()), (409, Some(1006012)));
    assert_eq!(reply.body["message"], "destination_changed");
    assert!(reply.body.get("detail").is_none(), "{}", reply.text);
    assert!(!reply.text.contains("127.0.0.1"));
    assert_eq!(env.up.hits(), 1);
}

#[tokio::test]
async fn regenerate_replaces_the_last_reply_and_replays() {
    let env = env().await;
    let cookie = env.paired(PEER).await;
    let chat = env.chat(&cookie).await;
    env.up.script(reply("first"));
    send(&env, &cookie, &chat, "q", &uuid()).await;
    let items = env.settled(&cookie, &chat).await;
    let assistant = items[1]["id"].as_str().unwrap().to_owned();
    env.up.script(reply("second"));
    let id = uuid();
    let path = format!("/api/v1/lan/chat/messages/{assistant}/regenerate");
    let ack = env
        .post(&path, json!({ "client_request_id": id }))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(ack.status, 202, "{}", ack.text);
    // FR-006 keeps the old reply until the new one completes, then replaces it.
    let mut items = Vec::new();
    for _ in 0..300 {
        items = env.settled(&cookie, &chat).await;
        if items.last().unwrap()["content"] == "second" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(items.len(), 2);
    assert_eq!(items.last().unwrap()["content"], "second");
    // Replayed after the replaced reply is gone: the original ack, no new call.
    let replay = env
        .post(&path, json!({ "client_request_id": id }))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(replay.status, 202, "{}", replay.text);
    assert_eq!(replay.body["replayed"], true);
    assert_eq!(env.up.hits(), 2);
    // Another device cannot regenerate it.
    let other = env.paired("192.168.1.11").await;
    let foreign = env
        .post(
            &format!(
                "/api/v1/lan/chat/messages/{}/regenerate",
                items.last().unwrap()["id"].as_str().unwrap()
            ),
            json!({ "client_request_id": uuid() }),
        )
        .peer("192.168.1.11")
        .cookie(&other)
        .send()
        .await;
    assert_eq!((foreign.status, foreign.code()), (409, Some(1006014)));
    let user_message = env
        .post(
            &format!(
                "/api/v1/lan/chat/messages/{}/regenerate",
                items[0]["id"].as_str().unwrap()
            ),
            json!({ "client_request_id": uuid() }),
        )
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(user_message.code(), Some(1006014));
}

#[tokio::test]
async fn cancel_stops_only_this_devices_running_task() {
    let env = env().await;
    let cookie = env.paired(PEER).await;
    let chat = env.chat(&cookie).await;
    env.up.script(vec![delta("partial "), Step::Hold]);
    let ack = send(&env, &cookie, &chat, "long", &uuid()).await;
    let task = ack.body["task_id"].as_str().unwrap().to_owned();
    until(|| env.up.open() == 1).await;
    // Busy: the user has one active generation (shared with the desktop).
    let busy = send(&env, &cookie, &chat, "again", &uuid()).await;
    assert_eq!((busy.status, busy.code()), (409, Some(1006010)));
    // Another device cannot cancel it.
    let other = env.paired("192.168.1.11").await;
    let path = format!("/api/v1/lan/chat/tasks/{task}/cancel");
    let foreign = env
        .post(&path, json!({}))
        .peer("192.168.1.11")
        .cookie(&other)
        .send()
        .await;
    assert_eq!(foreign.body, json!({"cancelled": false}));
    let cancelled = env.post(&path, json!({})).cookie(&cookie).send().await;
    assert_eq!(cancelled.body, json!({"cancelled": true}));
    let items = env.settled(&cookie, &chat).await;
    assert_eq!(items[1]["status"], "interrupted");
    assert_eq!(items[1]["content"], "partial ");
    // Finished, unknown and malformed ids: false, no side effects.
    for id in [task.clone(), uuid(), "not-a-uuid".into()] {
        let reply = env
            .post(&format!("/api/v1/lan/chat/tasks/{id}/cancel"), json!({}))
            .cookie(&cookie)
            .send()
            .await;
        assert_eq!(reply.body, json!({"cancelled": false}), "{id}");
    }
}

#[tokio::test]
async fn revoking_a_device_interrupts_its_generation() {
    let env = env().await;
    let cookie = env.paired(PEER).await;
    let chat = env.chat(&cookie).await;
    env.up.script(vec![delta("half"), Step::Hold]);
    send(&env, &cookie, &chat, "long", &uuid()).await;
    until(|| env.up.open() == 1).await;
    let id = env.core.list_devices()[0].session_id.clone();
    assert!(env.core.disconnect(&id).await);
    until(|| env.up.open() == 0).await;
    // The desktop sees the saved part.
    for _ in 0..300 {
        let page = env
            .host
            .chat
            .list_messages(
                ALICE,
                dgos_workbench::ListChatMessagesInput {
                    session_id: chat.clone(),
                    before_seq: None,
                    limit: None,
                },
            )
            .await
            .unwrap();
        if page.items[1].status != "generating" {
            assert_eq!(page.items[1].status, "interrupted");
            assert_eq!(page.items[1].content, "half");
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("generation did not end");
}

#[tokio::test]
async fn a_pairing_creates_at_most_100_chats() {
    let env = env().await;
    let cookie = env.paired(PEER).await;
    for _ in 0..100 {
        env.chat(&cookie).await;
    }
    let limited = env
        .post("/api/v1/lan/chat/sessions", env.target())
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!((limited.status, limited.code()), (429, Some(1010011)));
    assert_eq!(limited.body["detail"], json!({"scope": "chat_sessions"}));
    let list = env
        .get("/api/v1/lan/chat/sessions")
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(list.body["items"].as_array().map(Vec::len), Some(100));
}

#[tokio::test]
async fn message_paging_and_query_errors() {
    let env = env().await;
    let cookie = env.paired(PEER).await;
    let chat = env.chat(&cookie).await;
    for n in 0..2 {
        send(&env, &cookie, &chat, &format!("q{n}"), &uuid()).await;
        env.settled(&cookie, &chat).await;
    }
    let page = env
        .get(&format!(
            "/api/v1/lan/chat/sessions/{chat}/messages?limit=2"
        ))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(page.body["items"].as_array().unwrap().len(), 2);
    let before: Value = page.body["next_before_seq"].clone();
    assert!(before.is_i64());
    let older = env
        .get(&format!(
            "/api/v1/lan/chat/sessions/{chat}/messages?limit=2&before_seq={before}"
        ))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(older.body["items"][0]["content"], "q0");
    let bad = env
        .get(&format!(
            "/api/v1/lan/chat/sessions/{chat}/messages?limit=abc"
        ))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!((bad.status, bad.text.as_str()), (400, ""));
}
