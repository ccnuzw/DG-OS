use super::CodexSession;
use crate::codex_operations::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

// All RPCs below are selected from the installed v2 schema. No caller can
// supply a method, path, argv, unobserved child, or unobserved terminal ID.
// Observations are bounded independently; an absent method disables only its
// domain, while exhausted pages remain explicitly incomplete.

const MAX_PAGES: usize = 8;
const MAX_ROWS: usize = 1024;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_CHILDREN: usize = 32;

#[derive(Default)]
pub(super) struct OperationsState {
    children: BTreeMap<String, CodexChildAgent>,
    unsupported: HashSet<String>,
    incomplete: bool,
    automatic: Vec<AutomaticRun>,
    seen_turns: HashSet<String>,
}
struct AutomaticRun { run_id: String, turn_id: String, status: Option<String> }
impl OperationsState {
    pub(super) fn has_seen_turn(&self, native: &str) -> bool { self.seen_turns.contains(native) }
    fn known(&self, root: &str, thread: &str) -> bool { root == thread || self.children.contains_key(thread) }
    fn observe_child(&mut self, root: &str, owner: &str, child: &str) {
        if !valid_id(child) || child == root || child == owner { self.incomplete = true; return; }
        if let Some(previous) = self.children.get(child) {
            if previous.parent_thread_id != owner { self.incomplete = true; }
            return;
        }
        if self.children.len() >= MAX_CHILDREN { self.incomplete = true; return; }
        self.children.insert(child.into(), CodexChildAgent { thread_id: child.into(), parent_thread_id: owner.into(), status: "unknown".into(), turn_id: None });
    }
    // Only native creation observations on a known ancestor establish ancestry.
    // A send/wait recipient or activity reference alone grants no child authority.
    pub(super) fn item(&mut self, root: &str, owner: &str, item: &Value) {
        if !self.known(root, owner) { return; }
        if item["type"] == "subAgentActivity" {
            if item["kind"] == "started" {
                if let Some(child) = item["agentThreadId"].as_str() { self.observe_child(root, owner, child); }
                else { self.incomplete = true; }
            }
            // Activity kind is not a thread lifecycle status; confirmed_child
            // reads authoritative current state before any control is enabled.
            return;
        }
        if item["type"] != "collabAgentToolCall" || item["senderThreadId"].as_str() != Some(owner) { return; }
        if item["tool"] == "spawnAgent" {
            for child in item["receiverThreadIds"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                self.observe_child(root, owner, child);
            }
        }
        for (id, state) in item["agentsStates"].as_object().into_iter().flatten() {
            if let Some(child) = self.children.get_mut(id) {
                if let Some(status) = state["status"].as_str() { child.status = status.into(); }
            }
        }
    }
    pub(super) fn event(&mut self, root: &str, event: &Value) {
        if event["method"] == "host/closed" {
            for run in self.automatic.iter_mut().filter(|run| run.status.is_none()) { run.status = Some("unknown".into()); }
        }
        let params = &event["params"];
        let Some(owner) = params["threadId"].as_str() else { return; };
        if !self.known(root, owner) { return; }
        match event["method"].as_str() {
            Some("item/started" | "item/completed") => self.item(root, owner, &params["item"]),
            Some("turn/started" | "turn/completed") => {
                for item in params["turn"]["items"].as_array().into_iter().flatten() { self.item(root, owner, item); }
                if owner == root && event["method"] == "turn/completed" {
                    let native = params["turn"]["id"].as_str();
                    for run in self.automatic.iter_mut().filter(|run| Some(run.turn_id.as_str()) == native) {
                        run.status = Some(match params["turn"]["status"].as_str() { Some("completed") => "completed", Some("interrupted") => "cancelled", Some("failed") => "failed", _ => "unknown" }.into());
                    }
                }
                if let Some(child) = self.children.get_mut(owner) {
                    child.turn_id = if event["method"] == "turn/started" { params["turn"]["id"].as_str().map(str::to_owned) } else { None };
                    child.status = params["turn"]["status"].as_str().unwrap_or("unknown").into();
                }
            }
            Some("thread/closed") => { if let Some(child) = self.children.get_mut(owner) { child.status = "notLoaded".into(); child.turn_id = None; } }
            _ => {}
        }
    }
}
fn valid_id(id: &str) -> bool { !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control) }
fn text_input(text: &str) -> Value { json!([{"type":"text","text":text,"text_elements":[]}]) }
fn text_of(item: &Value) -> Option<String> {
    if item["type"] == "reasoning" { return None; }
    if let Some(text) = item["text"].as_str() { return Some(text.into()); }
    let content = item.get("content").or_else(|| item.get("input"))?.as_array()?;
    let texts: Vec<_> = content.iter().filter(|value| value["type"] == "text").filter_map(|value| value["text"].as_str()).collect();
    (!texts.is_empty()).then(|| texts.join("\n"))
}
fn history_turn(raw: &Value, incomplete: &mut bool) -> Option<CodexHistoryTurn> {
    let id = raw["id"].as_str().filter(|id| valid_id(id))?;
    let status = raw["status"].as_str()?;
    let mut items = Vec::new();
    for item in raw["items"].as_array()? {
        let (Some(id), Some(kind)) = (item["id"].as_str().filter(|id| valid_id(id)), item["type"].as_str()) else { *incomplete = true; continue; };
        items.push(CodexHistoryItem { id: id.into(), kind: kind.into(), text: text_of(item) });
    }
    Some(CodexHistoryTurn { id: id.into(), status: status.into(), items })
}
fn exact_membership(observed: &[Value], requested: &[String]) -> bool {
    let ids: HashSet<_> = observed.iter().filter_map(|value| value["id"].as_str()).collect();
    ids.len() == observed.len() && requested.len() == ids.len() && requested.iter().map(String::as_str).collect::<HashSet<_>>() == ids
}
fn continuation(reply: &Value, cursors: &mut HashSet<String>) -> Result<Option<String>, ()> {
    match reply.get("nextCursor") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(cursor)) if !cursor.is_empty() && cursor.len() <= 4096 && cursors.insert(cursor.clone()) => Ok(Some(cursor.clone())),
        _ => Err(()),
    }
}

impl CodexSession {
    pub(crate) async fn automatic_runs(&self) -> Vec<String> { self.turn.lock().await.operations.automatic.iter().map(|run| run.run_id.clone()).collect() }
    pub(crate) async fn automatic_run_status(&self, run_id: &str) -> Option<String> {
        self.turn.lock().await.operations.automatic.iter().find(|run| run.run_id == run_id).and_then(|run| run.status.clone())
    }
    pub(crate) async fn settle_automatic_run(&self, run_id: &str) {
        let mut turn = self.turn.lock().await;
        turn.operations.automatic.retain(|run| run.run_id != run_id);
    }
    async fn operation_request(&self, method: &str, params: Value, domain: &str) -> Result<Value, String> {
        if self.turn.lock().await.operations.unsupported.contains(domain) { return Err("codex_operation_unsupported".into()); }
        self.wire.request(method, params).await.map_err(|error| if error == "method_unsupported" { "codex_operation_unsupported".into() } else { error })
    }
    async fn operation_rpc(&self, method: &str, params: Value, domain: &str) -> Result<Value, String> {
        let result = self.operation_request(method, params, domain).await;
        if result.as_ref().err().is_some_and(|error| error == "codex_operation_unsupported") { self.turn.lock().await.operations.unsupported.insert(domain.into()); }
        result
    }
    async fn operation_pages(&self, method: &str, mut params: Value, domain: &str) -> Result<(Vec<Value>, bool), String> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut rows = Vec::new();
        let mut cursors = HashSet::new();
        let mut bytes = 0;
        params["limit"] = json!(if method == "thread/items/list" { 1 } else { 128 });
        let pages = if method == "thread/items/list" { 64 } else { MAX_PAGES };
        for _ in 0..pages {
            let mut reply = match tokio::time::timeout_at(deadline, self.operation_rpc(method, params.clone(), domain)).await {
                Ok(Ok(reply)) => reply,
                Ok(Err(error)) if rows.is_empty() => return Err(error),
                Err(_) if rows.is_empty() => return Err("result_unknown".into()),
                _ => return Ok((rows, true)),
            };
            crate::images::redact_image_inputs(&mut reply);
            let data = reply["data"].as_array().ok_or("invalid_output")?;
            for row in data {
                bytes += super::event_size(row);
                if rows.len() >= MAX_ROWS || bytes > MAX_BYTES { return Ok((rows, true)); }
                rows.push(row.clone());
            }
            match continuation(&reply, &mut cursors) {
                Ok(None) => return Ok((rows, false)),
                Ok(Some(cursor)) => params["cursor"] = json!(cursor),
                Err(()) => return Ok((rows, true)),
            }
        }
        Ok((rows, true))
    }
    async fn operation_raw_history(&self, root: &str, thread: &str) -> Result<(Vec<Value>, bool), String> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut hydrations = 0;
        let (rows, mut incomplete) = self.operation_pages("thread/turns/list", json!({"threadId":thread,"itemsView":"notLoaded","sortDirection":"desc"}), "turns").await?;
        let mut turns = Vec::new();
        let mut seen = HashSet::new();
        let mut budget = MAX_BYTES;
        for mut raw in rows {
            if raw.get("itemsView").and_then(Value::as_str).is_some_and(|view| view != "full") {
                if hydrations >= MAX_PAGES || tokio::time::Instant::now() >= deadline { incomplete = true; }
                else if let Some(id) = raw["id"].as_str().map(str::to_owned) {
                    hydrations += 1;
                    let result = tokio::time::timeout_at(deadline, self.operation_pages("thread/items/list", json!({"threadId":thread,"turnId":id,"sortDirection":"asc"}), "turns")).await;
                    match result {
                        Ok(Ok((entries, partial))) => {
                            incomplete |= partial;
                            let mut items = Vec::new();
                            for entry in entries {
                                if entry["turnId"].as_str() != Some(id.as_str()) { incomplete = true; continue; }
                                if let Some(item) = entry.get("item") { items.push(item.clone()); } else { incomplete = true; }
                            }
                            raw["items"] = Value::Array(items);
                            raw["itemsView"] = json!("full");
                        }
                        _ => incomplete = true,
                    }
                } else { incomplete = true; continue; }
            }
            let size = super::event_size(&raw);
            if size > budget { incomplete = true; break; }
            budget -= size;
            {
                let mut turn = self.turn.lock().await;
                for item in raw["items"].as_array().into_iter().flatten() { turn.operations.item(root, thread, item); }
            }
            match history_turn(&raw, &mut incomplete) {
                Some(turn) if seen.insert(turn.id.clone()) => turns.push(raw),
                Some(_) => {},
                None => incomplete = true,
            }
        }
        turns.reverse();
        Ok((turns, incomplete))
    }
    async fn operation_history(&self, root: &str, thread: &str) -> Result<(Vec<CodexHistoryTurn>, bool), String> {
        let (rows, mut incomplete) = self.operation_raw_history(root, thread).await?;
        let turns = rows.iter().filter_map(|raw| history_turn(raw, &mut incomplete)).collect();
        Ok((turns, incomplete))
    }
    async fn confirmed_child(&self, child: &str) -> Result<Value, String> {
        if !self.turn.lock().await.operations.children.contains_key(child) { return Err("codex_child_unobserved".into()); }
        let reply = self.operation_rpc("thread/read", json!({"threadId":child,"includeTurns":false}), "children").await?;
        let thread = reply.get("thread").filter(|thread| thread["id"].as_str() == Some(child)).ok_or("invalid_output")?;
        let parent = self.turn.lock().await.operations.children.get(child).map(|child| child.parent_thread_id.clone()).ok_or("codex_child_unobserved")?;
        if thread.get("parentThreadId").and_then(Value::as_str).is_some_and(|actual| actual != parent) { return Err("codex_child_unobserved".into()); }
        Ok(thread.clone())
    }
    async fn current_child_turn(&self, child: &str) -> Result<Option<String>, String> {
        let reply = self.operation_rpc("thread/turns/list", json!({"threadId":child,"limit":1,"itemsView":"notLoaded","sortDirection":"desc"}), "turns").await?;
        let turns = reply["data"].as_array().ok_or("invalid_output")?;
        Ok(turns.first().filter(|turn| turn["status"] == "inProgress").and_then(|turn| turn["id"].as_str()).filter(|id| valid_id(id)).map(str::to_owned))
    }
    pub(crate) async fn observe_operations(&self) -> Result<CodexOperationsSnapshot, String> {
        let view = self.snapshot().await;
        let root = view.native_id.as_deref().ok_or("session_closed")?;
        if !self.native_available() { return Err("session_closed".into()); }
        let mut out = CodexOperationsSnapshot::default();
        match self.operation_history(root, root).await { Ok((turns, incomplete)) => { out.turns = turns; out.incomplete |= incomplete; }, Err(_) => out.incomplete = true }
        let mut visited = HashSet::new();
        for _ in 0..MAX_CHILDREN {
            let next = self.turn.lock().await.operations.children.keys().find(|child| !visited.contains(*child)).cloned();
            let Some(child) = next else { break; };
            visited.insert(child.clone());
            match self.confirmed_child(&child).await {
                Ok(thread) => {
                    let current = self.current_child_turn(&child).await;
                    let mut state = self.turn.lock().await;
                    if let Some(entry) = state.operations.children.get_mut(&child) {
                        entry.status = thread.pointer("/status/type").and_then(Value::as_str).unwrap_or("unknown").into();
                        entry.turn_id = current.as_ref().ok().cloned().flatten();
                    }
                    out.incomplete |= current.is_err();
                    drop(state);
                    if let Ok((_, incomplete)) = self.operation_history(root, &child).await { out.incomplete |= incomplete; } else { out.incomplete = true; }
                }
                Err(_) => out.incomplete = true,
            }
        }
        match self.operation_pages("thread/backgroundTerminals/list", json!({"threadId":root}), "terminals").await {
            Ok((rows, incomplete)) => {
                out.incomplete |= incomplete;
                for row in rows {
                    if let (Some(id), Some(command)) = (row["processId"].as_str().filter(|id| valid_id(id)), row["command"].as_str()) {
                        // The installed terminal inventory reports presence,
                        // not a lifecycle status; do not invent one.
                        out.terminals.push(CodexBackgroundTerminal { process_id: id.into(), command: command.into(), status: "unknown".into() });
                    } else { out.incomplete = true; }
                }
            }
            Err(_) => out.incomplete = true,
        }
        match self.operation_pages("thread/queue/list", json!({"threadId":root}), "queue").await {
            Ok((rows, incomplete)) => {
                out.incomplete |= incomplete;
                for row in rows {
                    if let Some(id) = row["id"].as_str().filter(|id| valid_id(id)) {
                        out.queue.push(CodexQueueEntry { id: id.into(), text: text_of(&row).unwrap_or_default() });
                    } else { out.incomplete = true; }
                }
            }
            Err(_) => out.incomplete = true,
        }
        let state = self.turn.lock().await;
        out.children = state.operations.children.values().cloned().collect();
        out.incomplete |= state.operations.incomplete;
        out.unsupported = state.operations.unsupported.iter().cloned().collect();
        out.unsupported.push("close_child".into());
        if state.operations.unsupported.contains("turns") { out.unsupported.push("children".into()); }
        out.unsupported.sort();
        Ok(out)
    }
    pub(crate) async fn read_child_history(&self, child: &str) -> Result<CodexChildHistory, String> {
        self.confirmed_child(child).await?;
        let root = self.snapshot().await.native_id.ok_or("session_closed")?;
        let (turns, incomplete) = self.operation_history(&root, child).await?;
        Ok(CodexChildHistory { thread_id: child.into(), turns, incomplete })
    }
    async fn operation_turn(&self, method: &str, params: Value, run_id: String, domain: &str) -> Result<(), String> {
        let expected_thread = params["threadId"].as_str().map(str::to_owned);
        {
            let mut view = self.view.lock().await;
            if view.status != "idle" || !self.native_available() { return Err("busy".into()); }
            let mut turn = self.turn.lock().await;
            turn.id = None;
            turn.starting = true;
            turn.operation_binding = method == "thread/compact/start";
            turn.deferred.clear();
            turn.deferred_bytes = 0;
            turn.user_echo_pending = false;
            turn.steering_echoes = 0;
            view.status = "running".into();
            view.active_run = Some(run_id.clone());
            view.error = None;
        }
        let result = self.operation_rpc(method, params, domain).await.and_then(|reply| {
            if method == "review/start" && reply["reviewThreadId"].as_str() != expected_thread.as_deref() { Err("result_unknown".into()) } else { Ok(reply) }
        });
        let mut view = self.view.lock().await;
        let mut turn = self.turn.lock().await;
        turn.starting = false;
        turn.deferred_bytes = 0;
        let result = if matches!(view.status.as_str(), "unknown" | "closed") { Err("result_unknown".into()) } else { result };
        let identity = result.as_ref().ok().and_then(|reply| reply.pointer("/turn/id")).and_then(Value::as_str).filter(|id| valid_id(id)).map(str::to_owned);
        let success = result.is_ok() && (identity.is_some() || method == "thread/compact/start");
        let mut replies = Vec::new();
        if success {
            turn.id = identity;
            for event in std::mem::take(&mut turn.deferred) { if let Some(reply) = Self::apply_event(&mut view, &mut turn, event) { replies.push(reply); } }
        } else {
            let error = result.err().unwrap_or_else(|| "result_unknown".into());
            turn.deferred.clear();
            turn.operation_binding = false;
            turn.id = None;
            let rejected = matches!(error.as_str(), "codex_operation_unsupported" | "engine_rejected");
            view.status = if rejected { "idle" } else { "unknown" }.into();
            view.error = Some(error.clone());
            view.push("error", Some(error.clone()), json!({"run_status":if rejected { "failed" } else { "unknown" },"native_status":"operation_failed"}));
            view.active_run = None;
            return Err(error);
        }
        drop(turn);
        drop(view);
        for reply in replies { let _ = self.wire.write(reply).await; }
        if method == "thread/compact/start" {
            let session = self.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                let mut view = session.view.lock().await;
                let mut turn = session.turn.lock().await;
                expire_binding(&mut view, &mut turn, &run_id);
            });
        }
        Ok(())
    }
    pub(crate) async fn execute_operation(&self, action: CodexAction, run_id: String) -> Result<Option<String>, String> {
        action.validate()?;
        if !self.native_available() { return Err("session_closed".into()); }
        let root = self.snapshot().await.native_id.ok_or("session_closed")?;
        match action {
            CodexAction::Review { target } => self.operation_turn("review/start", json!({"threadId":root,"target":target,"delivery":"inline"}), run_id, "review").await?,
            CodexAction::Compact {} => self.operation_turn("thread/compact/start", json!({"threadId":root}), run_id, "compact").await?,
            CodexAction::QueueStart { submission_id } => {
                let (rows, incomplete) = self.operation_pages("thread/queue/list", json!({"threadId":root}), "queue").await?;
                if incomplete { return Err("codex_observation_incomplete".into()); }
                let selected = submission_id.or_else(|| rows.first().and_then(|row| row["id"].as_str()).map(str::to_owned)).ok_or("codex_queue_empty")?;
                if !rows.iter().any(|row| row["id"].as_str() == Some(selected.as_str())) { return Err("codex_queue_unobserved".into()); }
                self.operation_turn("thread/queue/start", json!({"threadId":root,"queuedSubmissionId":selected}), run_id, "queue_start").await?;
            }
            CodexAction::Fork { last_turn_id, .. } => {
                self.require_operation_idle().await?;
                if let Some(id) = &last_turn_id { self.require_history_turn(&root, id, true).await?; }
                let reply = self.operation_rpc("thread/fork", json!({"threadId":root,"lastTurnId":last_turn_id,"excludeTurns":true,"deferGoalContinuation":true,"ephemeral":false}), "fork").await?;
                let id = reply.pointer("/thread/id").and_then(Value::as_str).filter(|id| valid_id(id) && *id != root).ok_or("result_unknown")?;
                return Ok(Some(id.into()));
            }
            CodexAction::Revert { before_turn_id } => {
                self.require_operation_idle().await?;
                self.require_history_turn(&root, &before_turn_id, false).await?;
                let refresh_after = self.view.lock().await.events.last().map_or(0, |event| event.seq);
                let reply = self.operation_rpc("thread/revert", json!({"threadId":root,"beforeTurnId":before_turn_id}), "revert").await?;
                let acknowledged = reply.pointer("/thread/id").and_then(Value::as_str) == Some(root.as_str());
                self.refresh_operation_history(&root, refresh_after).await.map_err(|_| "result_unknown")?;
                if !acknowledged { return Err("result_unknown".into()); }
            }
            CodexAction::InterruptChild { child_thread_id, turn_id } => {
                self.confirmed_child(&child_thread_id).await?;
                if self.current_child_turn(&child_thread_id).await?.as_deref() != Some(turn_id.as_str()) { return Err("not_running".into()); }
                self.operation_rpc("turn/interrupt", json!({"threadId":child_thread_id,"turnId":turn_id}), "interrupt_child").await?;
            }
            CodexAction::SendChild { child_thread_id, text } => {
                let child = self.confirmed_child(&child_thread_id).await?;
                if child["canAcceptDirectInput"] != true { return Err("codex_operation_unsupported".into()); }
                let current = self.current_child_turn(&child_thread_id).await?;
                if let Some(turn) = current {
                    let reply = self.operation_rpc("turn/steer", json!({"threadId":child_thread_id,"expectedTurnId":turn,"input":text_input(&text)}), "send_child").await?;
                    if reply["turnId"].as_str() != Some(turn.as_str()) { return Err("result_unknown".into()); }
                } else {
                    if child.pointer("/status/type").and_then(Value::as_str) != Some("idle") { return Err("busy".into()); }
                    let reply = self.operation_rpc("turn/start", json!({"threadId":child_thread_id,"input":text_input(&text)}), "send_child").await?;
                    if reply.pointer("/turn/id").and_then(Value::as_str).is_none_or(|id| !valid_id(id)) { return Err("result_unknown".into()); }
                }
            }
            CodexAction::ResumeChild { child_thread_id } => {
                let child = self.confirmed_child(&child_thread_id).await?;
                if self.current_child_turn(&child_thread_id).await?.is_some() || !matches!(child.pointer("/status/type").and_then(Value::as_str), Some("idle" | "notLoaded")) { return Err("busy".into()); }
                let reply = self.operation_rpc("thread/resume", json!({"threadId":child_thread_id,"excludeTurns":true}), "resume_child").await?;
                if reply.pointer("/thread/id").and_then(Value::as_str) != Some(child_thread_id.as_str()) { return Err("result_unknown".into()); }
            }
            CodexAction::CloseChild { child_thread_id } => {
                self.confirmed_child(&child_thread_id).await?;
                self.current_child_turn(&child_thread_id).await?;
                // unsubscribe does not close the agent; no close-agent client RPC
                // exists in the installed protocol. Never pretend it does.
                return Err("codex_operation_unsupported".into());
            }
            CodexAction::TerminateTerminal { process_id } => {
                let (rows, incomplete) = self.operation_pages("thread/backgroundTerminals/list", json!({"threadId":root}), "terminals").await?;
                if incomplete { return Err("codex_observation_incomplete".into()); }
                if !rows.iter().any(|row| row["processId"].as_str() == Some(process_id.as_str())) { return Err("codex_terminal_unobserved".into()); }
                let reply = self.operation_rpc("thread/backgroundTerminals/terminate", json!({"threadId":root,"processId":process_id}), "terminate_terminal").await?;
                confirmed_boolean(&reply, "terminated")?;
            }
            CodexAction::QueueAdd { text } => {
                let reply = self.operation_rpc("thread/queue/add", json!({"threadId":root,"clientUserMessageId":run_id,"input":text_input(&text)}), "queue_add").await?;
                if reply.pointer("/queuedSubmission/id").and_then(Value::as_str).is_none_or(|id| !valid_id(id)) { return Err("result_unknown".into()); }
            }
            action @ (CodexAction::QueueUpdate { .. } | CodexAction::QueueDelete { .. } | CodexAction::QueueReorder { .. }) => {
                let (rows, incomplete) = self.operation_pages("thread/queue/list", json!({"threadId":root}), "queue").await?;
                if incomplete { return Err("codex_observation_incomplete".into()); }
                match action {
                    CodexAction::QueueUpdate { submission_id, text } => {
                        require_queue_id(&rows, &submission_id)?;
                        let reply = self.operation_rpc("thread/queue/update", json!({"threadId":root,"queuedSubmissionId":submission_id,"input":text_input(&text)}), "queue_update").await?;
                        if reply.pointer("/queuedSubmission/id").and_then(Value::as_str) != Some(submission_id.as_str()) { return Err("result_unknown".into()); }
                    }
                    CodexAction::QueueDelete { submission_id } => {
                        require_queue_id(&rows, &submission_id)?;
                        let reply = self.operation_rpc("thread/queue/delete", json!({"threadId":root,"queuedSubmissionId":submission_id}), "queue_delete").await?;
                        confirmed_boolean(&reply, "deleted")?;
                    }
                    CodexAction::QueueReorder { submission_ids } => {
                        if !exact_membership(&rows, &submission_ids) { return Err("codex_queue_membership_changed".into()); }
                        self.operation_rpc("thread/queue/reorder", json!({"threadId":root,"queuedSubmissionIds":submission_ids}), "queue_reorder").await?;
                    }
                    _ => unreachable!(),
                }
            }
        }
        Ok(None)
    }
    async fn require_operation_idle(&self) -> Result<(), String> { if self.snapshot().await.status == "idle" { Ok(()) } else { Err("busy".into()) } }
    async fn require_history_turn(&self, root: &str, id: &str, completed: bool) -> Result<(), String> {
        let (turns, incomplete) = self.operation_history(root, root).await?;
        if turns.iter().any(|turn| turn.id == id && (!completed || matches!(turn.status.as_str(), "completed" | "interrupted" | "failed"))) { Ok(()) }
        else if incomplete { Err("codex_observation_incomplete".into()) }
        else { Err("codex_turn_unobserved".into()) }
    }
    async fn refresh_operation_history(&self, root: &str, after: u64) -> Result<(), String> {
        match self.operation_raw_history(root, root).await {
            Ok((turns, incomplete)) => replace_history(&mut *self.view.lock().await, root, turns, incomplete, after),
            Err(error) => {
                // The native mutation already happened. Never show the old,
                // now-invalid suffix or misclassify a refresh failure as refusal.
                replace_history(&mut *self.view.lock().await, root, vec![], true, after);
                return Err(error);
            }
        }
        Ok(())
    }
}
fn expire_binding(view: &mut crate::session::SessionView, turn: &mut super::TurnState, run_id: &str) {
    if view.active_run.as_deref() != Some(run_id) || !turn.operation_binding || turn.id.is_some() { return; }
    turn.operation_binding = false;
    view.status = "unknown".into();
    view.error = Some("result_unknown".into());
    view.push("error", Some("result_unknown".into()), json!({"run_status":"unknown","native_status":"operation_identity_unconfirmed"}));
    view.active_run = None;
}
fn replace_history(view: &mut crate::session::SessionView, root: &str, mut turns: Vec<Value>, incomplete: bool, after: u64) {
    let offset = view.events.last().map_or(1, |event| event.seq.saturating_add(1));
    // Notifications are not blocked by the control lock. Retain everything
    // observed since mutation admission, including automatic queue starts,
    // approvals and completed runs, on both successful and failed hydration.
    let live: Vec<_> = view.events.iter().filter(|event| event.seq > after).cloned().collect();
    let live_turns: HashSet<_> = live.iter().filter_map(|event| event.data.get("turn_id").or_else(|| event.data.get("turnId")).or_else(|| event.data.pointer("/turn/id")).and_then(Value::as_str)).collect();
    turns.retain(|turn| !turn["id"].as_str().is_some_and(|id| live_turns.contains(id)));
    let mut refreshed = crate::session::SessionView::new(view.workspace.clone());
    refreshed.native_id = view.native_id.clone();
    refreshed.history_truncated = incomplete || live.first().is_some_and(|event| event.seq > after.saturating_add(1));
    let mut history = super::history::History::default();
    history.seed(&mut refreshed, &json!({"thread":{"turns":turns}}));
    history.finish(&mut refreshed);
    for event in live { refreshed.push_run(&event.kind, event.text, event.data, event.run); }
    // Hydration is transcript-only. Preserve the exact current runtime projection;
    // in particular active_run must remain paired with TurnState.id for interrupt.
    refreshed.status = view.status.clone();
    refreshed.active_run = view.active_run.clone();
    refreshed.error = view.error.clone();
    refreshed.controls = view.controls.clone();
    refreshed.cli_version = view.cli_version.clone();
    refreshed.pending = view.pending.clone();
    refreshed.persistence_error = view.persistence_error.clone();
    refreshed.terminal_run = view.terminal_run.clone();
    refreshed.push_run("history/reverted", None, json!({"threadId":root,"incomplete":incomplete}), None);
    // A sequence gap forces all pre-revert cursors to reset.
    for event in &mut refreshed.events { event.seq = event.seq.saturating_add(offset); }
    *view = refreshed;
}
fn confirmed_boolean(reply: &Value, field: &str) -> Result<(), String> {
    match reply.get(field).and_then(Value::as_bool) {
        Some(true) => Ok(()),
        Some(false) => Err("engine_rejected".into()),
        None => Err("result_unknown".into()),
    }
}
fn require_queue_id(rows: &[Value], id: &str) -> Result<(), String> {
    if rows.iter().any(|row| row["id"].as_str() == Some(id)) { Ok(()) } else { Err("codex_queue_unobserved".into()) }
}
pub(super) fn bind_automatic(view: &mut crate::session::SessionView, turn: &mut super::TurnState, params: &Value) {
    let Some(native) = params.pointer("/turn/id").and_then(Value::as_str).filter(|id| valid_id(id)) else { return; };
    if view.active_run.is_some() {
        if turn.id.as_deref() == Some(native) && !turn.operations.seen_turns.contains(native) {
            if turn.operations.seen_turns.len() >= 4096 {
                view.status = "unknown".into();
                view.error = Some("automatic_run_capacity".into());
                view.push("error", Some("automatic_run_capacity".into()), json!({"run_status":"unknown","native_status":"turn_identity_capacity"}));
                view.active_run = None;
            } else { turn.operations.seen_turns.insert(native.into()); }
        }
        return;
    }
    // Every generation is observed from opening, before any queue mutation or
    // resume output: eligibility is the exact owned root and an unbound turn.
    if !matches!(view.status.as_str(), "idle" | "error") || turn.operations.seen_turns.contains(native) { return; }
    if turn.operations.automatic.len() >= 128 || turn.operations.seen_turns.len() >= 4096 {
        view.status = "unknown".into();
        view.error = Some("automatic_run_capacity".into());
        view.push("error", Some("automatic_run_capacity".into()), json!({"turn_id":native,"source":"native_queue"}));
        return;
    }
    let run_id = uuid::Uuid::new_v4().to_string();
    turn.operations.seen_turns.insert(native.into());
    turn.operations.automatic.push(AutomaticRun { run_id: run_id.clone(), turn_id: native.into(), status: None });
    turn.id = Some(native.into());
    turn.user_echo_pending = false;
    turn.steering_echoes = 0;
    turn.requests.retain(|_, request| request.kind == "mcpServer/elicitation/request" && request.params.get("turnId").is_none_or(Value::is_null));
    view.active_run = Some(run_id.clone());
    view.status = "running".into();
    view.error = None;
    view.push("nativeRun/observed", None, json!({"run_id":run_id,"turn_id":native,"source":"native_queue"}));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionView;
    fn spawn(sender: &str, child: &str) -> Value { json!({"type":"collabAgentToolCall","id":"spawn","tool":"spawnAgent","senderThreadId":sender,"receiverThreadIds":[child],"agentsStates":{(child):{"status":"running"}}}) }
    #[test]
    fn ancestry_requires_observed_spawn_and_supports_nested_descendants() {
        let mut state = OperationsState::default();
        state.item("root", "foreign", &spawn("foreign", "bad"));
        let mut message = spawn("root", "unrelated"); message["tool"] = json!("sendInput");
        state.item("root", "root", &message);
        assert!(state.children.is_empty());
        state.item("root", "root", &spawn("root", "child"));
        state.item("root", "child", &spawn("child", "grandchild"));
        assert_eq!(state.children["grandchild"].parent_thread_id, "child");
        state.item("root", "grandchild", &spawn("grandchild", "root"));
        state.item("root", "grandchild", &spawn("grandchild", "child"));
        assert!(state.incomplete);
        assert_eq!(state.children["child"].parent_thread_id, "root");
        assert!(!state.known("root", "bad"));
    }
    #[test]
    fn queue_reorder_is_an_exact_permutation_not_a_subset_or_multiset() {
        let rows = vec![json!({"id":"a"}), json!({"id":"b"})];
        assert!(exact_membership(&rows, &["b".into(), "a".into()]));
        for ids in [vec!["a".into()], vec!["a".into(), "a".into()], vec!["a".into(), "foreign".into()]] { assert!(!exact_membership(&rows, &ids)); }
        assert!(!exact_membership(&[json!({"id":"a"}), json!({"id":"a"})], &["a".into()]));
        assert_eq!(require_queue_id(&rows, "foreign").unwrap_err(), "codex_queue_unobserved");
    }
    #[test]
    fn pagination_cycles_and_malformed_tokens_cannot_look_complete() {
        let mut cursors = HashSet::new();
        assert_eq!(continuation(&json!({"nextCursor":"next"}), &mut cursors), Ok(Some("next".into())));
        assert!(continuation(&json!({"nextCursor":"next"}), &mut cursors).is_err());
        for token in [json!(""), json!(false), json!("x".repeat(4097))] { assert!(continuation(&json!({"nextCursor":token}), &mut cursors).is_err()); }
        assert_eq!(continuation(&json!({"nextCursor":null}), &mut cursors), Ok(None));
    }
    #[test]
    fn compact_buffers_native_identity_and_attributes_terminal_to_durable_run() {
        let mut view = SessionView::new("/fixture".into());
        view.native_id = Some("root".into()); view.status = "running".into(); view.active_run = Some("durable".into());
        let mut turn = super::super::TurnState { operation_binding: true, ..Default::default() };
        // Foreign turns cannot steal the binding left by compact's empty ACK.
        CodexSession::apply_event(&mut view, &mut turn, json!({"method":"turn/started","params":{"threadId":"foreign","turn":{"id":"wrong"}}}));
        assert!(turn.id.is_none());
        for event in [
            json!({"method":"turn/started","params":{"threadId":"root","turn":{"id":"compact-turn","status":"inProgress"}}}),
            json!({"method":"item/completed","params":{"threadId":"root","turnId":"compact-turn","item":{"id":"compact-item","type":"contextCompaction"}}}),
            json!({"method":"turn/completed","params":{"threadId":"root","turn":{"id":"compact-turn","status":"completed","items":[]}}}),
        ] { CodexSession::apply_event(&mut view, &mut turn, event); }
        assert_eq!(view.status, "idle");
        assert_eq!(view.terminal_run.as_ref().unwrap().run_id, "durable");
        assert_eq!(view.terminal_run.as_ref().unwrap().status, "completed");
        assert!(view.events.iter().all(|event| event.run.as_deref() == Some("durable")));
    }
    #[test]
    fn child_history_preserves_native_identity_without_reasoning_or_image_payload() {
        let mut incomplete = false;
        let turn = history_turn(&json!({"id":"t","status":"completed","items":[{"id":"u","type":"userMessage","content":[{"type":"image","url":"data:image/png;base64,secret"},{"type":"text","text":"describe"}]},{"id":"r","type":"reasoning","text":"private","content":["secret"]}]}), &mut incomplete).unwrap();
        assert_eq!(turn.items[0].text.as_deref(), Some("describe"));
        assert!(turn.items[1].text.is_none());
        assert!(!serde_json::to_string(&turn).unwrap().contains("secret"));
        assert!(!incomplete);
    }
}

#[cfg(all(test, unix))]
#[path = "codex_operations_session_tests.rs"]
mod protocol_tests;

#[cfg(test)]
mod automatic_tests {
    use super::*;
    use crate::session::SessionView;
    fn event(view: &mut SessionView, state: &mut super::super::TurnState, native: &str, started: bool) {
        CodexSession::apply_event(view, state, json!({"method":if started { "turn/started" } else { "turn/completed" },"params":{"threadId":"root","turn":{"id":native,"status":if started { "inProgress" } else { "completed" },"items":[]}}}));
    }
    #[test]
    fn automatic_queue_runs_keep_identity_and_terminal_after_marker_eviction() {
        let mut view = SessionView::new("/fixture".into()); view.native_id = Some("root".into()); view.status = "idle".into();
        let mut state = super::super::TurnState::default();
        CodexSession::apply_event(&mut view, &mut state, json!({"method":"turn/started","params":{"threadId":"foreign","turn":{"id":"unrelated"}}}));
        assert!(state.operations.automatic.is_empty());
        event(&mut view, &mut state, "first", true);
        let first = view.active_run.clone().unwrap();
        assert_eq!(view.events.last().unwrap().kind, "nativeRun/observed");
        assert_eq!(view.events.last().unwrap().data["turn_id"], "first");
        event(&mut view, &mut state, "first", false);
        event(&mut view, &mut state, "second", true);
        let second = view.active_run.clone().unwrap();
        event(&mut view, &mut state, "second", false);
        assert_ne!(first, second);
        assert_eq!(state.operations.automatic.iter().map(|run| run.run_id.as_str()).collect::<Vec<_>>(), [first.as_str(), second.as_str()]);
        for _ in 0..2100 { view.push("bounded-output", None, Value::Null); }
        assert!(!view.events.iter().any(|event| event.kind == "nativeRun/observed"));
        assert_eq!(state.operations.automatic.iter().find(|run| run.run_id == first).unwrap().status.as_deref(), Some("completed"));
        assert_eq!(state.operations.automatic.iter().find(|run| run.run_id == second).unwrap().status.as_deref(), Some("completed"));
        state.operations.automatic.retain(|run| run.run_id != first);
        event(&mut view, &mut state, "first", true);
        assert!(view.active_run.is_none(), "a replayed started notification cannot create a second run after durable settlement");
        assert_eq!(state.operations.automatic.len(), 1);
        assert_eq!(state.operations.automatic[0].run_id, second);
    }
    #[test]
    fn automatic_queue_capacity_fails_closed_without_evicting_pending_runs() {
        let mut view = SessionView::new("/fixture".into()); view.native_id = Some("root".into()); view.status = "idle".into();
        let mut state = super::super::TurnState::default();
        for index in 0..128 { let native = format!("native-{index}"); event(&mut view, &mut state, &native, true); event(&mut view, &mut state, &native, false); }
        event(&mut view, &mut state, "over-capacity", true);
        assert_eq!(view.status, "unknown");
        assert_eq!(view.error.as_deref(), Some("automatic_run_capacity"));
        assert_eq!(state.operations.automatic.len(), 128);
        assert!(state.operations.automatic.iter().all(|run| run.status.as_deref() == Some("completed")));
    }
}
