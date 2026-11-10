// Real operations opt-in: all mutations use visible production controls.
// Synthetic image bytes are input data, not a synthetic native/model response.
const ACCEPTANCE_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";

async fn real_image_draft_checks(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>) -> Journey<()> {
    real_image_phase(app, fx, ck, ev, "image-picker-present", json!({}), r#"
const input=await until(imagePicker);return {ok:!!input&&!input.disabled,present:!!input,enabled:!!input&&!input.disabled,file_input:input?.type==='file',file_api:typeof File==='function',transfer_api:typeof DataTransfer==='function'};
"#).await?;
    real_image_phase(app, fx, ck, ev, "image-picker-selection", json!({"png":ACCEPTANCE_PNG}), r#"
const input=imagePicker(),transfer=new DataTransfer();transfer.items.add(new File([Uint8Array.from(atob(A.png),c=>c.charCodeAt(0))],'acceptance-pixel.png',{type:'image/png'}));input.files=transfer.files;const assigned=input.files?.length===1;input.dispatchEvent(new Event('change',{bubbles:true}));return {ok:assigned,assigned};
"#).await?;
    real_image_phase(app, fx, ck, ev, "image-preview-button", json!({}), r#"
const preview=await until(()=>tid('agent-image-preview-0'));return {ok:!!preview,present:!!preview,alert_present:!!tid('agent-image-list')?.querySelector('[role="alert"]'),busy:tid('agent-image-list')?.getAttribute('aria-busy')==='true'};
"#).await?;
    real_image_phase(app, fx, ck, ev, "image-preview-dialog", json!({}), r#"
tid('agent-image-preview-0').click();const dialog=await until(()=>tid('agent-image-dialog'));return {ok:!!dialog,present:!!dialog,image_present:!!dialog?.querySelector('img')};
"#).await?;
    real_image_phase(app, fx, ck, ev, "image-preview-decode", json!({}), r#"
const image=tid('agent-image-dialog')?.querySelector('img');const decoded=!!await until(()=>image?.complete&&image.naturalWidth===1&&image.naturalHeight===1);return {ok:decoded,decoded,complete:!!image?.complete,expected_width:image?.naturalWidth===1,expected_height:image?.naturalHeight===1};
"#).await?;
    real_image_phase(app, fx, ck, ev, "image-preview-close", json!({}), r#"
document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));const closed=!!await until(()=>!tid('agent-image-dialog'));return {ok:closed,closed};
"#).await?;
    real_image_phase(app, fx, ck, ev, "image-remove", json!({}), r#"
const remove=tid('agent-image-remove-0');if(remove&&!remove.disabled)remove.click();const removed=!!await until(()=>!tid('agent-image-preview-0'));return {ok:removed,removed,no_dispatch:!window.__realCodex.events.some(e=>e.command==='send_agent_message')};
"#).await?;
    real_image_phase(app, fx, ck, ev, "image-reselect-first-send-draft", json!({"png":ACCEPTANCE_PNG}), r#"
const input=imagePicker(),transfer=new DataTransfer();transfer.items.add(new File([Uint8Array.from(atob(A.png),c=>c.charCodeAt(0))],'acceptance-pixel.png',{type:'image/png'}));input.files=transfer.files;input.dispatchEvent(new Event('change',{bubbles:true}));const selected=!!await until(()=>tid('agent-image-preview-0'));const noDispatch=!window.__realCodex.events.some(e=>e.command==='send_agent_message');return {ok:selected&&noDispatch,present:selected,no_dispatch:noDispatch};
"#).await?;
    Ok(())
}

async fn real_image_phase(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, phase: &str, args: Value, body: &str) -> Journey<()> {
    // The file input is intentionally hidden. Resolve only beneath its visible
    // owning composer/list; never weaken stage6's global visibility filter.
    let scope = r#"const imageList=()=>$$('.agent-dialogue-composer').map(composer=>composer.querySelector('[data-testid="agent-image-list"]')).find(list=>visible(list));const imagePicker=()=>imageList()?.querySelector('input[type="file"][data-testid="agent-image-picker"]');"#;
    let reply = dom(app, args, &format!("{scope}\n{body}")).await;
    // Whitelist booleans only: no browser errors, filenames, content or DOM text.
    let mut flags = serde_json::Map::new();
    for key in ["present", "enabled", "file_input", "file_api", "transfer_api", "assigned", "alert_present", "busy", "image_present", "decoded", "complete", "expected_width", "expected_height", "closed", "removed", "no_dispatch"] {
        if let Some(value) = reply["value"][key].as_bool() { flags.insert(key.into(), json!(value)); }
    }
    real_require(fx, ck, ev, phase, ui_ok(&reply), json!({"phase":phase,"flags":flags,"input_method":"synthetic DOM File/DataTransfer picker events; not physical OS picker","browser_error_and_content":"omitted"}))
}

async fn real_operations_snapshot(app: &AppHandle, sid: &str) -> Journey<Value> {
    let view = real_host(app, "get_agent_session", json!({"sessionId":sid})).await?;
    real_host(app, "observe_codex_operations", json!({"input":{"session_id":sid,"generation":view["generation"]}})).await
}

async fn real_operation_click(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, sid: &str, selector: &str, kind: &str, row: Option<&str>) -> Journey<Value> {
    let dispatched = real_step(app, fx, ck, ev, "explicit new-operation production control dispatch", json!({"selector":selector,"kind":kind,"row":row}), r#"
const before=window.__realCodex.events.filter(e=>e.command==='control_codex_operation').length;const button=await until(()=>{const parent=A.row?$$('li[data-submission-id]').find(e=>e.dataset.submissionId===A.row):document;const b=parent?.querySelector('[data-testid="'+A.selector+'"]');return b&&!b.disabled&&b});if(!button)throw Error('native operation control disabled');button.click();
const event=await until(()=>window.__realCodex.events.filter(e=>e.command==='control_codex_operation').length>before&&window.__realCodex.events.filter(e=>e.command==='control_codex_operation').at(-1));return {ok:!!event&&event.action===A.kind,request_id:event?.request_id};
"#).await?;
    let request_id = dispatched["request_id"].as_str().ok_or("operation request identity absent")?;
    let view = real_host(app, "get_agent_session", json!({"sessionId":sid})).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    loop {
        let receipt = match real_host(app, "inspect_codex_operation", json!({"input":{"session_id":sid,"generation":view["generation"],"request_id":request_id}})).await {
            Ok(receipt) => receipt,
            Err(error) => { real_operation_ui_settled(app).await?; return Err(error); },
        };
        if receipt["state"] == "applied" {
            real_require(fx, ck, ev, "new-operation receipt belongs to exact session generation and request", receipt["session_id"] == sid && receipt["generation"] == view["generation"] && receipt["request_id"] == request_id, json!({"operation":kind,"state":"applied","raw_payload":"omitted"}))?;
            real_operation_ui_settled(app).await?;
            return Ok(receipt);
        }
        // Unknown also describes an in-flight journal. Inspect is read-only;
        // it must never cause the original mutation to be dispatched again.
        if tokio::time::Instant::now() >= deadline {
            real_save(fx, "operation-settlement", &json!({"operation":kind,"state":if receipt["state"] == "unknown" {"unknown"}else{"missing"},"deadline_seconds":90,"replayed":false}));
            return Err("native operation did not settle before inspection deadline; no replay".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn real_operation_ui_settled(app: &AppHandle) -> Journey<()> {
    let reply = dom(app, json!({}), r#"
const settled=!!await until(()=>{const panel=tid('codex-operations'),refresh=tid('codex-operations-refresh');return panel?.dataset.pending==='false'&&panel?.dataset.reading==='false'&&refresh&&!refresh.disabled},15000);return {ok:settled};
"#).await;
    if ui_ok(&reply) { Ok(()) } else { Err("native operation renderer still busy after host settlement; no further mutation".into()) }
}

async fn real_operations_refresh(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>) -> Journey<()> {
    real_operation_phase(app, fx, ck, ev, "operation-panel-present", r#"
const panel=tid('codex-operations');const scopeMissing=!!tid('codex-operations-unavailable');return {ok:!!panel,present:!!panel,scope_missing:scopeMissing};
"#).await?;
    real_operation_phase(app, fx, ck, ev, "operation-panel-open", r#"
const panel=tid('codex-operations');if(panel.tagName==='DETAILS'&&!panel.open)panel.querySelector('summary')?.click();return {ok:panel.tagName!=='DETAILS'||panel.open,present:!!panel,open:panel.open===true};
"#).await?;
    real_operation_phase(app, fx, ck, ev, "operation-refresh-enabled", r#"
await until(()=>{const panel=tid('codex-operations'),button=tid('codex-operations-refresh');return panel?.dataset.reading==='false'&&button&&!button.disabled},15000);const panel=tid('codex-operations'),refresh=tid('codex-operations-refresh');return {ok:!!refresh&&!refresh.disabled&&panel?.dataset.reading==='false',present:!!refresh,enabled:!!refresh&&!refresh.disabled,reading:panel?.dataset.reading==='true',stale:panel?.dataset.stale==='true',pending:panel?.dataset.pending==='true',scope_ready:panel?.dataset.scopeReady==='true'};
"#).await?;
    real_operation_phase(app, fx, ck, ev, "operation-observation-dispatched", r#"
const count=()=>window.__realCodex.events.filter(e=>e.command==='observe_codex_operations').length;const before=count();tid('codex-operations-refresh').click();await until(()=>count()>before,5000);const panel=tid('codex-operations');return {ok:count()>before,dispatched:count()>before,reading:panel?.dataset.reading==='true',scope_ready:panel?.dataset.scopeReady==='true'};
"#).await?;
    real_operation_phase(app, fx, ck, ev, "operation-observation-settled", r#"
await until(()=>{const panel=tid('codex-operations');return panel&&panel.dataset.reading==='false'},15000);const panel=tid('codex-operations'),refresh=tid('codex-operations-refresh');const settled=panel?.dataset.reading==='false',fresh=panel?.dataset.stale==='false';return {ok:settled&&fresh,settled,fresh,present:!!panel,enabled:!!refresh&&!refresh.disabled,reading:panel?.dataset.reading==='true',stale:panel?.dataset.stale==='true',pending:panel?.dataset.pending==='true',scope_ready:panel?.dataset.scopeReady==='true',observation_error:!!panel?.dataset.operationError,operation_error:panel?.dataset.operationError??null};
"#).await?;
    Ok(())
}

async fn real_operation_phase(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, phase: &str, body: &str) -> Journey<()> {
    let reply = dom(app, json!({}), body).await;
    let mut flags = serde_json::Map::new();
    for key in ["present", "enabled", "open", "scope_missing", "scope_ready", "reading", "stale", "pending", "dispatched", "settled", "fresh", "observation_error"] {
        if let Some(value) = reply["value"][key].as_bool() { flags.insert(key.into(), json!(value)); }
    }
    // Revalidate the component's normalized code against the same bounded public
    // error vocabulary. No unrecognized native error or DOM string is persisted.
    let operation_error = reply["value"]["operation_error"].as_str().map(|code| match code {
        "invalid_codex_operation" | "invalid_request_id" | "session_not_found" | "stale_generation" | "session_closed" | "project_archived" | "file_access_denied" | "busy" | "session_busy" | "task_not_found" | "codex_operation_unsupported" | "engine_rejected" | "method_unsupported" | "codex_child_unobserved" | "not_running" | "codex_terminal_unobserved" | "codex_queue_unobserved" | "codex_queue_empty" | "codex_queue_membership_changed" | "codex_turn_unobserved" | "codex_observation_incomplete" | "session_invalid" | "workspace_changed" | "provider_revision_changed" | "provider_binding_unconfirmed" | "capability_revoked" | "storage_failed" | "request_conflict" | "unknown" | "unavailable" => code,
        _ => "unavailable",
    });
    let diagnostic = json!({"phase":phase,"flags":flags,"operation_error":operation_error,"private_state":"omitted"});
    real_save(fx, "operation-diagnostic", &diagnostic);
    real_require(fx, ck, ev, phase, ui_ok(&reply), diagnostic)
}

async fn real_operation_checks(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, sid: &str, marker: &Path, approvals: &mut std::collections::HashSet<String>) -> Journey<()> {
    let mut coverage = serde_json::Map::new();
    let image = real_image_only_check(app, fx, ck, ev, sid, marker, approvals).await;
    coverage.insert("image_only".into(), json!(real_operation_branch_result(fx, ck, ev, "image-only", image)?));
    let review = real_review_check(app, fx, ck, ev, sid, marker, approvals).await;
    coverage.insert("review".into(), json!(real_operation_branch_result(fx, ck, ev, "review", review)?));
    let compact = real_compact_check(app, fx, ck, ev, sid, marker, approvals).await;
    coverage.insert("compact".into(), json!(real_operation_branch_result(fx, ck, ev, "compact", compact)?));
    let fork = real_fork_check(app, fx, ck, ev, sid, marker, approvals).await;
    coverage.insert("fork".into(), json!(real_operation_branch_result(fx, ck, ev, "fork", fork)?));
    let revert = real_revert_check(app, fx, ck, ev, sid, marker, approvals).await;
    coverage.insert("revert".into(), json!(real_operation_branch_result(fx, ck, ev, "revert", revert)?));
    let queue = real_queue_checks(app, fx, ck, ev, sid, marker, approvals).await;
    coverage.insert("queue".into(), json!(real_operation_branch_result(fx, ck, ev, "queue", queue)?));
    let resources = real_resource_setup_check(app, fx, ck, ev, sid, marker, approvals).await;
    coverage.insert("resources".into(), json!(real_operation_branch_result(fx, ck, ev, "resources", resources)?));
    real_capture_check(app, fx, ck, ev, "operations-complete").await?;
    real_save(fx, "operations", &json!({"source":"real App/WKWebView/IPC/installed native CLI","protocol_fixture":false,"branch_completed":coverage,"queue_scheduling":"native-controlled; explicit and automatic outcomes recorded separately","boundary_fixture_evidence":"separate synthetic protocol/tests, never promoted to real model proof","private_content":"omitted"}));
    if coverage.values().all(|value| value == true) { Ok(()) } else { Err("one or more independent native operation branches failed".into()) }
}

fn real_operation_branch_result(fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, branch: &str, result: Journey<()>) -> Journey<bool> {
    match result {
        Ok(()) => Ok(true),
        Err(error) => {
            if error == "screenshot masking restoration failed" { return Err(error); }
            let name = format!("independent native operation branch: {branch}");
            record(ck, ev, &name, false, error);
            real_save(fx, "checkpoint", &json!({"step":name,"passed":ck.passed,"failed":ck.failed,"checks":ev}));
            Ok(false)
        }
    }
}

async fn real_operation_branch_ready(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, sid: &str, marker: &Path, approvals: &mut std::collections::HashSet<String>) -> Journey<()> {
    let reply = dom(app, json!({}), r#"
document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));
const dialog=()=>$$('[role="dialog"]').find(d=>d.querySelector('[data-testid="codex-revert-confirm"],[data-testid="codex-fork-confirm"],[data-testid="codex-review-start"]'));
const current=dialog();if(current){const cancel=$$('button',current).find(b=>['取消','Cancel'].includes(txt(b))&&!b.disabled);if(cancel)cancel.click();}
const closed=!!await until(()=>!dialog(),15000);return {ok:closed};
"#).await;
    if !ui_ok(&reply) { return Err("native operation dialog recovery unavailable; branch not dispatched".into()); }
    let settled = dom(app, json!({}), "await until(()=>!tid('codex-operations')||tid('codex-operations')?.dataset.pending==='false',15000);return {ok:!tid('codex-operations')||tid('codex-operations')?.dataset.pending==='false'};").await;
    if !ui_ok(&settled) { return Err("prior native operation remains pending; independent branch not dispatched".into()); }
    let view = real_host(app, "get_agent_session", json!({"sessionId":sid})).await?;
    if view["active_run"].is_string() {
        real_step(app, fx, ck, ev, "explicitly interrupt active native parent before independent operation", json!({}), r#"
const stop=await until(()=>$$('.agent-control-send button[aria-label]',tid('dev-panel-agent')).find(b=>['停止本次执行','Stop this turn'].includes(b.getAttribute('aria-label'))&&!b.disabled));if(!stop)throw Error('native interrupt unavailable');stop.click();return {ok:true};
"#).await?;
    }
    coding_wait(app, sid, 60, marker, approvals, |view| view["status"] == "idle" && view["active_run"].is_null()).await?;
    real_operations_refresh(app, fx, ck, ev).await
}

async fn real_review_check(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, sid: &str, marker: &Path, approvals: &mut std::collections::HashSet<String>) -> Journey<()> {
    real_operation_branch_ready(app, fx, ck, ev, sid, marker, approvals).await?;
    real_step(app, fx, ck, ev, "native review form is explicit and does not submit an ordinary message", json!({}), r#"
const before=window.__realCodex.events.filter(e=>e.command==='send_agent_message').length;await click('codex-review-open');return {ok:!!await need('codex-review-start')&&window.__realCodex.events.filter(e=>e.command==='send_agent_message').length===before};
"#).await?;
    let review = real_operation_click(app, fx, ck, ev, sid, "codex-review-start", "review", None).await?;
    let run = review["run_id"].as_str().ok_or("review durable run absent")?;
    let reviewed = coding_wait(app, sid, 300, marker, approvals, |view| view["status"] == "idle" && view["active_run"].is_null() && real_events(view).any(|event| event["run"] == run && event["data"]["run_status"] == "completed")).await?;
    real_operation_durable(app, sid, run).await?;
    let completed = real_events(&reviewed).any(|event| event["run"] == run && matches!(event["kind"].as_str(), Some("exitedReviewMode" | "agentMessage")));
    real_require(fx, ck, ev, "actual native review completes its own supervised run", completed, json!({"native_review":true,"durable_run":true,"result_content":"omitted"}))
}

async fn real_compact_check(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, sid: &str, marker: &Path, approvals: &mut std::collections::HashSet<String>) -> Journey<()> {
    real_operation_branch_ready(app, fx, ck, ev, sid, marker, approvals).await?;
    let receipt = real_operation_click(app, fx, ck, ev, sid, "codex-compact", "compact", None).await?;
    let run = receipt["run_id"].as_str().ok_or("compaction durable run absent")?;
    let compacted = coding_wait(app, sid, 180, marker, approvals, |view| view["status"] == "idle" && view["active_run"].is_null() && real_events(view).any(|event| event["run"] == run && event["data"]["run_status"] == "completed")).await?;
    real_operation_durable(app, sid, run).await?;
    let completed = real_events(&compacted).any(|event| event["run"] == run && matches!(event["kind"].as_str(), Some("thread/compacted" | "contextCompaction")));
    real_require(fx, ck, ev, "actual native compaction reaches completion not mere request receipt", completed, json!({"native_compaction_completed":true,"content":"omitted"}))
}

async fn real_fork_check(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, sid: &str, marker: &Path, approvals: &mut std::collections::HashSet<String>) -> Journey<()> {
    real_operation_branch_ready(app, fx, ck, ev, sid, marker, approvals).await?;
    real_step(app, fx, ck, ev, "native fork requires explicit confirmation without implicit switch", json!({}), "await click('codex-fork');return {ok:!!await need('codex-fork-confirm')};").await?;
    let receipt = real_operation_click(app, fx, ck, ev, sid, "codex-fork-confirm", "fork", None).await?;
    let fork_sid = receipt["fork_session_id"].as_str().ok_or("fork session receipt absent")?;
    let fork = real_host(app, "get_agent_session", json!({"sessionId":fork_sid})).await?;
    let original = real_host(app, "get_agent_session", json!({"sessionId":sid})).await?;
    real_require(fx, ck, ev, "native fork keeps original thread and stays suspended without automatic dispatch", fork_sid != sid && fork["native_id"].is_string() && fork["native_id"] != original["native_id"] && fork["status"] == "closed" && fork["active_run"].is_null() && original["status"] == "idle", json!({"distinct_native_thread":true,"original_retained":true,"fork_status":"closed; explicit resume required","automatic_send":false}))?;
    real_host(app, "close_agent_session", json!({"sessionId":fork_sid})).await?;
    Ok(())
}

async fn real_revert_check(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, sid: &str, marker: &Path, approvals: &mut std::collections::HashSet<String>) -> Journey<()> {
    real_operation_branch_ready(app, fx, ck, ev, sid, marker, approvals).await?;
    let original = real_host(app, "get_agent_session", json!({"sessionId":sid})).await?;
    let math_before = fs::read(fx.root.join("real-coding-project/math.cjs")).map_err(|_| "pre-revert source missing")?;
    let format_before = fs::read(fx.root.join("real-coding-project/format.cjs")).map_err(|_| "pre-revert source missing")?;
    let snapshot = real_operations_snapshot(app, sid).await?;
    let target = snapshot["turns"].as_array().and_then(|turns| turns.last()).and_then(|turn| turn["id"].as_str()).ok_or("owned revert target absent")?;
    real_revert_phase(app, fx, ck, ev, "revert-open", json!({}), "await click('codex-revert-open');const present=!!await until(()=>tid('codex-revert-confirm')?.closest('[role=dialog]'));return {ok:present,present};").await?;
    real_revert_phase(app, fx, ck, ev, "revert-selector", json!({}), "const select=await until(()=>{const e=document.getElementById('codex-revert-turn');return e&&visible(e)&&!e.disabled&&e});return {ok:!!select,present:!!select,enabled:!!select&&!select.disabled};").await?;
    real_revert_phase(app, fx, ck, ev, "revert-option", json!({"turn":target}), r#"
document.getElementById('codex-revert-turn').click();const option=await until(()=>$$('[role="option"]').find(e=>visible(e)&&txt(e).startsWith(A.turn+' · ')));const present=!!option;if(option)option.click();return {ok:present,present};
"#).await?;
    real_revert_phase(app, fx, ck, ev, "revert-selected", json!({"turn":target}), "const selected=!!await until(()=>txt(document.getElementById('codex-revert-turn')).startsWith(A.turn+' · '));return {ok:selected,selected};").await?;
    real_revert_phase(app, fx, ck, ev, "revert-enabled", json!({}), "const confirm=await until(()=>{const e=tid('codex-revert-confirm');return e&&visible(e)&&!e.disabled&&e});return {ok:!!confirm,present:!!confirm,enabled:!!confirm&&!confirm.disabled};").await?;
    real_operation_click(app, fx, ck, ev, sid, "codex-revert-confirm", "revert", None).await?;
    let after = real_host(app, "get_agent_session", json!({"sessionId":sid})).await?;
    real_require(fx, ck, ev, "actual native revert leaves runtime generation and disk fixes intact with no old approvals", after["generation"] == original["generation"] && after["pending"].as_array().is_none_or(Vec::is_empty) && fs::read(fx.root.join("real-coding-project/math.cjs")).ok().as_ref() == Some(&math_before) && fs::read(fx.root.join("real-coding-project/format.cjs")).ok().as_ref() == Some(&format_before), json!({"history_only":true,"disk_unchanged":true,"pending_old_approvals":0}))
}

async fn real_revert_phase(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, phase: &str, args: Value, body: &str) -> Journey<()> {
    let reply = dom(app, args, body).await;
    let mut flags = serde_json::Map::new();
    for key in ["present", "enabled", "selected"] {
        if let Some(value) = reply["value"][key].as_bool() { flags.insert(key.into(), json!(value)); }
    }
    let diagnostic = json!({"phase":phase,"flags":flags,"browser_error_and_content":"omitted"});
    real_save(fx, "revert-diagnostic", &diagnostic);
    real_require(fx, ck, ev, phase, ui_ok(&reply), diagnostic)
}

include!("codex_real_queue_journey.rs");

async fn real_image_only_check(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, sid: &str, marker: &Path, approvals: &mut std::collections::HashSet<String>) -> Journey<()> {
    let before = real_host(app, "get_agent_session", json!({"sessionId":sid})).await?;
    let messages = real_events(&before).filter(|event| event["kind"] == "userMessage").count();
    real_step(app, fx, ck, ev, "production paste accepts image-only draft before explicit send", json!({"png":ACCEPTANCE_PNG}), r#"
const input=document.getElementById('agent-message');setInput(input,'');const transfer=new DataTransfer();transfer.items.add(new File([Uint8Array.from(atob(A.png),c=>c.charCodeAt(0))],'pasted-acceptance-pixel.png',{type:'image/png'}));input.dispatchEvent(new ClipboardEvent('paste',{clipboardData:transfer,bubbles:true,cancelable:true}));return {ok:!!await need('agent-image-preview-0')&&input.value===''};
"#).await?;
    coding_send(app, fx, ck, ev, "empty-text image-only explicit send reaches production host", "").await?;
    let completed = coding_wait(app, sid, 180, marker, approvals, |view| view["status"] == "idle" && view["active_run"].is_null() && real_events(view).filter(|event| event["kind"] == "userMessage").count() == messages + 1).await?;
    real_require(fx, ck, ev, "actual native image-only turn completes without exposing image content", real_events(&completed).any(|event| matches!(event["kind"].as_str(), Some("agentMessage" | "item/agentMessage/delta"))), json!({"text_empty":true,"actual_native_turn":true,"image_content":"omitted","input_method":"programmatic DOM File/paste event; not physical OS picker"}))?;
    real_step(app, fx, ck, ev, "image-only production dispatch retains one image and empty text", json!({}), "const sends=window.__realCodex.events.filter(e=>e.command==='send_agent_message');return {ok:sends.length===2&&sends[1].image_count===1&&sends[1].text_empty===true&&!tid('agent-image-preview-0')};").await?;
    Ok(())
}

async fn real_operation_durable(app: &AppHandle, sid: &str, run: &str) -> Journey<()> {
    let workspace = real_host(app, "read_development_workspace", json!({})).await?;
    let task = workspace["selection"]["task_id"].as_str().ok_or("operation task missing")?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let rows = real_host(app, "list_development_runs", json!({"taskId":task})).await?;
        if rows.as_array().is_some_and(|rows| rows.iter().any(|row| row["id"] == run && row["session_id"] == sid && row["status"] == "completed" && row["finished_at"].is_number())) { return Ok(()); }
        if tokio::time::Instant::now() >= deadline { return Err("native operation durable terminal missing".into()); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
