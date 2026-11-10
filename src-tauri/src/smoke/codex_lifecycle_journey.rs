// Opt-in production App/WKWebView/IPC journey; all native answers are synthetic.
// DGOS_CODEX_LIFECYCLE_SMOKE=1 DGOS_PROJECT_ACCEPTANCE_PHASE=phase6-local
// DGOS_PROJECT_ACCEPTANCE_ROOT=/private/tmp/dgos-codex-lifecycle-<fresh-unique>
// cargo test --offline -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture

fn lifecycle_protocol(path: &Path) -> Journey<Vec<Value>> {
    let file = path.join("phase6-protocol.jsonl");
    if !file.exists() { return Ok(Vec::new()); }
    fs::read_to_string(file).map_err(|error| error.to_string())?.lines()
        .map(|line| serde_json::from_str(line).map_err(|error| error.to_string())).collect()
}

async fn lifecycle_session(app: &AppHandle, sid: &str, state: &str) -> Journey<Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let view = host(app, "get_agent_session", json!({"sessionId":sid})).await?;
        if view["status"] == state { return Ok(view); }
        if tokio::time::Instant::now() >= deadline { return Err(format!("synthetic lifecycle expected {state}: {view}")); }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn codex_lifecycle_journey(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, path: &Path) -> Journey<()> {
    const FIRST: &str = "SYNTHETIC LIFECYCLE first draft: configure before sending";
    const STEER: &str = "SYNTHETIC LIFECYCLE explicit steering";
    const SECOND: &str = "SYNTHETIC LIFECYCLE second explicit send";
    save(fx, "codex-lifecycle-provenance", &json!({
        "evidence":"local protocol fixture only, not real-model acceptance", "native_cli":false,
        "model_calls":0, "approved_commands_executed":0, "vault":"MemoryCredentialVault",
        "scenario":["configure before first send","automatic exact-workspace trust and prepare without dispatch","translated high reasoning tick","Plan native read-only overlay","approval response (denial)","steer","interrupt","send again","end session","resume without replay"],
        "real_model_blockers":["explicit DGOS_STAGE4_CODEX_URL/KEY/MODEL required for a separate real-provider run", "local protocol receipts cannot establish external CLI/model behavior"]
    }));
    step(app, fx, ck, ev, "model selection in automatically trusted workspace does not send", json!({"draft":FIRST}), r#"
setInput(document.getElementById('agent-message'),A.draft);
await chooseConversationModel('codex','Phase6 local protocol ONLY','fixture-model');
return {ok:!window.__phase6.events.some(e=>e.command==='send_agent_message')&&!tid('direct-conversation-authorize')&&!$$('[role=alertdialog]').length};
"#).await?;
    step(app, fx, ck, ev, "automatic workspace trust prepares without dispatch and transfers latest draft", json!({"draft":FIRST}), r#"
await authorizeConversation();
return {ok:!!await until(()=>!tid('dev-project-conversation')&&document.getElementById('agent-message')?.value===A.draft&&!document.getElementById('agent-message').disabled)&&window.__phase6.events.filter(e=>e.command==='open_agent_session').length===1&&!window.__phase6.events.some(e=>e.command==='send_agent_message')&&!tid('direct-conversation-authorize')&&!$$('[role=alertdialog]').length};
"#).await?;
    let sessions = host(app, "list_agent_sessions", json!({})).await?;
    let rows = sessions.as_array().ok_or("session list absent")?;
    require(fx, ck, ev, "prepare creates exactly one native session", rows.len() == 1, sessions.to_string())?;
    let sid = rows[0]["id"].as_str().ok_or("session ID absent")?.to_owned();
    let prepared = lifecycle_session(app, &sid, "idle").await?;
    require(fx, ck, ev, "prepared native session is idle with no user message or turn dispatch",
        prepared["active_run"].is_null()
        && !prepared["events"].as_array().into_iter().flatten().any(|event| event["kind"] == "userMessage")
        && !lifecycle_protocol(path)?.iter().any(|request| request["method"] == "turn/start"), prepared.to_string())?;
    require(fx, ck, ev, "fixture advertises confirmed Plan and steering capabilities",
        prepared["controls"]["steering"] == true
        && prepared["controls"]["collaboration"]["available"] == true
        && prepared["controls"]["collaboration"]["options"].as_array().is_some_and(|modes| modes.iter().any(|mode| mode == "plan")), prepared["controls"].to_string())?;
    step(app, fx, ck, ev, "select translated high reasoning tick and explicitly dismiss before first dispatch", json!({}), r#"
const reasoning=$$('button[aria-label]',tid('agent-composer-controls')).find(b=>['推理强度','Reasoning'].includes(b.getAttribute('aria-label'))&&!b.disabled);if(!reasoning)throw Error('reasoning popover trigger absent');reasoning.click();
const high=await until(()=>$$('.agent-reasoning-ticks button').find(b=>['高','High'].includes(txt(b))&&!b.disabled));if(!high)throw Error('high reasoning unavailable');high.click();
if(!await until(()=>high.getAttribute('aria-pressed')==='true'&&$$('.agent-control-popover-reasoning').length))throw Error('reasoning selection did not remain open');
reasoning.click();
return {ok:!!await until(()=>!$$('.agent-control-popover').length&&['高','High'].some(label=>txt(reasoning).includes(label)))&&!window.__phase6.events.some(e=>e.command==='send_agent_message')};
"#).await?;
    step(app, fx, ck, ev, "explicitly select Plan from five run modes before first model dispatch", json!({}), r#"
const mode=$$('button[aria-label]',tid('agent-composer-controls')).find(b=>['运行模式','Run mode'].includes(b.getAttribute('aria-label'))&&!b.disabled);if(!mode)throw Error('run mode popover trigger absent');mode.click();
const options=await until(()=>$$('.agent-mode-option').length===5&&$$('.agent-mode-option'));if(!options)throw Error('five run modes absent');
const plan=options.find(b=>['规划','Plan'].includes(txt(b.querySelector('strong')))&&!b.disabled);if(!plan)throw Error('Plan opt-in unavailable');plan.click();
return {ok:!!await until(()=>!$$('.agent-control-popover').length&&['规划','Plan'].some(label=>txt(mode).includes(label)))&&!window.__phase6.events.some(e=>e.command==='send_agent_message')};
"#).await?;
    step(app, fx, ck, ev, "first explicit send reaches native approval", json!({}), r#"
const send=await until(()=>$$('.agent-control-send button[aria-label]',tid('dev-panel-agent')).find(b=>['发送','Send'].includes(b.getAttribute('aria-label'))&&!b.disabled));
if(!send)throw Error('first explicit send unavailable');send.click();
return {ok:!!await until(()=>$$('.agent-dialogue-requests section',tid('dev-panel-agent')).find(e=>txt(e).includes('SYNTHETIC PROTOCOL ONLY')))};
"#).await?;
    let waiting = lifecycle_session(app, &sid, "waiting").await?;
    let first_run = waiting["active_run"].as_str().ok_or("first run absent")?.to_owned();
    let pending = waiting["pending"].as_array().ok_or("pending list absent")?;
    require(fx, ck, ev, "native approval is bound to first turn with exact high reasoning and Plan read-only wire policy",
        pending.len() == 1 && pending[0]["kind"] == "item/commandExecution/requestApproval"
        && pending[0]["data"]["turnId"] == "lifecycle-turn-1"
        && lifecycle_protocol(path)?.iter().any(|request| {
            let params = &request["params"];
            request["method"] == "turn/start" && params["effort"] == "high"
                && params["approvalPolicy"] == "never" && params["approvalsReviewer"] == "user"
                && params["sandboxPolicy"] == json!({"type":"readOnly","networkAccess":false})
                && params["collaborationMode"]["mode"] == "plan"
                && params["collaborationMode"]["settings"]["model"] == "fixture-model"
                && params["collaborationMode"]["settings"]["reasoning_effort"] == "high"
        }), waiting.to_string())?;
    step(app, fx, ck, ev, "deny labeled synthetic approval through production UI without elevating read-only", json!({}), r#"
const request=await until(()=>$$('.agent-dialogue-requests section',tid('dev-panel-agent')).find(e=>txt(e).includes('SYNTHETIC PROTOCOL ONLY')));
const deny=request&&$$('button',request).find(b=>['拒绝','Deny'].includes(txt(b))&&!b.disabled);
if(!deny)throw Error('synthetic denial action absent');deny.click();
return {ok:!!await until(()=>!$$('.agent-dialogue-requests section',tid('dev-panel-agent')).length&&txt(tid('dev-panel-agent')).includes('SYNTHETIC PROTOCOL ONLY: denial received'))};
"#).await?;
    let running = lifecycle_session(app, &sid, "running").await?;
    require(fx, ck, ev, "approval denial preserves original active run and selected default permission under Plan read-only overlay", running["active_run"] == first_run && running["pending"].as_array().is_some_and(Vec::is_empty) && running["controls"]["permission"]["current"] == "default" && running["controls"]["collaboration"]["current"] == "plan" && running["events"].as_array().is_some_and(|events| events.iter().any(|event| event["text"].as_str().is_some_and(|text| text.contains("SYNTHETIC PROTOCOL ONLY: denial received")))), running.to_string())?;
    step(app, fx, ck, ev, "steer active turn once and acknowledge only exact receipt", json!({"steer":STEER}), r#"
setInput(document.getElementById('agent-message'),A.steer);
const steer=await until(()=>$$('button',tid('dev-panel-agent')).find(b=>['追加到本次执行','Add to current turn'].includes(txt(b))&&!b.disabled));
if(!steer)throw Error('steer unavailable');steer.click();
return {ok:!!await until(()=>document.getElementById('agent-message')?.value==='')};
"#).await?;
    let steered = host(app, "get_agent_session", json!({"sessionId":sid})).await?;
    require(fx, ck, ev, "steering stays on first run with one positive native receipt",
        steered["active_run"] == first_run && steered["events"].as_array().is_some_and(|events| events.iter().filter(|event| event["kind"] == "userSteering" && event["run"] == first_run && event["text"] == STEER).count() == 1), steered.to_string())?;
    step(app, fx, ck, ev, "interrupt the original active turn explicitly", json!({}), r#"
const stop=await until(()=>$$('.agent-control-send button[aria-label]',tid('dev-panel-agent')).find(b=>['停止本次执行','Stop this turn'].includes(b.getAttribute('aria-label'))&&!b.disabled));
if(!stop)throw Error('interrupt unavailable');stop.click();return {ok:true};
"#).await?;
    let interrupted = lifecycle_session(app, &sid, "idle").await?;
    require(fx, ck, ev, "interrupt confirms terminal cancellation before next send",
        interrupted["active_run"].is_null() && interrupted["events"].as_array().is_some_and(|events| events.iter().any(|event| event["run"] == first_run && event["data"]["run_status"] == "cancelled")), interrupted.to_string())?;
    step(app, fx, ck, ev, "send again only after explicit confirmed interrupt", json!({"text":SECOND}), r#"
setInput(document.getElementById('agent-message'),A.text);
const send=await until(()=>$$('.agent-control-send button[aria-label]',tid('dev-panel-agent')).find(b=>['发送','Send'].includes(b.getAttribute('aria-label'))&&!b.disabled));
if(!send)throw Error('second explicit send unavailable');send.click();
return {ok:!!await until(()=>document.getElementById('agent-message')?.value==='')};
"#).await?;
    let second = lifecycle_session(app, &sid, "idle").await?;
    let workspace = host(app, "read_development_workspace", json!({})).await?;
    let task = workspace["selection"]["task_id"].as_str().ok_or("task absent")?.to_owned();
    // Native idle precedes the asynchronous durable run projection; observe both boundaries.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let runs = loop {
        let runs = host(app, "list_development_runs", json!({"taskId":task})).await?;
        let rows = runs.as_array().ok_or("run list absent")?;
        let finished = rows.iter().filter(|run| run["session_id"] == sid).all(|run| run["finished_at"].is_number());
        if finished && rows.iter().filter(|run| run["session_id"] == sid).count() == 2 || tokio::time::Instant::now() >= deadline { break runs; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let owned_runs = runs.as_array().ok_or("run list absent")?.iter().filter(|run| run["session_id"] == sid).collect::<Vec<_>>();
    require(fx, ck, ev, "two explicit sends persist distinct terminal runs",
        owned_runs.len() == 2
        && owned_runs.iter().any(|run| run["id"] == first_run && run["status"] == "cancelled")
        && owned_runs.iter().any(|run| run["id"] != first_run && run["status"] == "completed"), runs.to_string())?;
    step(app, fx, ck, ev, "explicit session end is separate from interrupt", json!({}), r#"
const close=await until(()=>$$('.agent-dialogue-actions button',tid('dev-panel-agent')).find(b=>['结束会话','End session'].includes(txt(b))&&!b.disabled));
if(!close)throw Error('end session unavailable');close.click();return {ok:!!await until(()=>tid('saved-conversation-composer'))};
"#).await?;
    let closed = lifecycle_session(app, &sid, "closed").await?;
    require(fx, ck, ev, "explicit close retains original native thread identity", closed["native_id"] == second["native_id"], closed.to_string())?;
    step(app, fx, ck, ev, "focusing authorized saved composer resumes native session without replay", json!({"sid":sid}), r#"
await prepareSavedConversation(A.sid);
return {ok:!!document.getElementById('agent-message')&&!tid('saved-conversation-composer')};
"#).await?;
    let resumed = lifecycle_session(app, &sid, "idle").await?;
    require(fx, ck, ev, "resume creates fresh generation on same thread with no active run",
        resumed["native_id"] == prepared["native_id"] && resumed["generation"] != prepared["generation"] && resumed["active_run"].is_null(), resumed.to_string())?;
    let requests = lifecycle_protocol(path)?;
    let count = |method: &str| requests.iter().filter(|request| request["method"] == method).count();
    let approvals = requests.iter().filter(|request| request["id"] == "lifecycle-SYNTHETIC-approval" && request["result"]["decision"] == "decline").count();
    require(fx, ck, ev, "wire proves one prepare, approval denial, steer, interrupt, resend and resume without auto replay",
        count("thread/start") == 1 && count("thread/resume") == 1 && count("turn/start") == 2
        && count("turn/steer") == 1 && count("turn/interrupt") == 1 && approvals == 1
        && !requests.iter().any(|request| request["result"]["decision"] == "accept")
        && requests.iter().any(|request| request["method"] == "turn/steer" && request["params"]["expectedTurnId"] == "lifecycle-turn-1"), json!(requests).to_string())?;
    let observed = step(app, fx, ck, ev, "production UI dispatches every lifecycle action exactly once", json!({}), r#"
const events=window.__phase6.events,count=command=>events.filter(e=>e.command===command).length;
return {ok:count('open_agent_session')===2&&count('send_agent_message')===2&&count('respond_agent_request')===1&&count('steer_agent_turn')===1&&count('interrupt_agent_session')===1&&count('close_agent_session')===1,events};
"#).await?;
    save(fx, "codex-lifecycle", &json!({"source":"SYNTHETIC local protocol, not real-model acceptance","session":sid,"prepared":prepared,"waiting":waiting,"steered":steered,"interrupted":interrupted,"second":second,"closed":closed,"resumed":resumed,"runs":runs,"wire":requests,"ipc":observed}));
    Ok(())
}
