// Real native queue/resource acceptance; no synthetic responses or host PTYs.
async fn real_queue_checks(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, sid: &str, marker: &Path, approvals: &mut std::collections::HashSet<String>) -> Journey<()> {
    const FIRST: &str = "Queue A: reply 42; no tools, writes or network.";
    const UPDATED: &str = "Edited queue A: reply 42; no tools, writes or network.";
    // The installed add schema has no hold flag. Native scheduling is not
    // controlled by prompt text; CRUD needs an actually active parent.
    const SECOND: &str = "Queue B: reply 42; no tools, writes or network.";
    real_operation_branch_ready(app, fx, ck, ev, sid, marker, approvals).await?;
    let initial = real_operations_snapshot(app, sid).await?;
    real_require(fx, ck, ev, "native queue starts empty before explicit CRUD", initial["queue"].as_array().is_some_and(Vec::is_empty), json!({"queue_entries":0}))?;
    coding_send(app, fx, ck, ev, "explicit bounded native parent holds queue editing window", "Authorized queue CRUD acceptance: run /bin/sleep 120 as a bounded read-only native command and keep this turn active waiting for it. Do not finalize early, write files, use network, escalate, spawn agents or run another command. I will interrupt from the UI while editing the native queue.").await?;
    let active = coding_wait(app, sid, 180, marker, approvals, |view| view["active_run"].is_string() && view["status"] == "running" && real_events(view).any(|event| event["run"] == view["active_run"] && event["kind"] == "item/started" && event["data"]["type"] == "commandExecution" && event["data"]["command"].as_str().is_some_and(|command| command.contains("sleep 120")))).await?;
    let parent_run = active["active_run"].as_str().ok_or("queue parent run missing")?;
    real_operations_refresh(app, fx, ck, ev).await?;
    for text in [FIRST, SECOND] {
        real_step(app, fx, ck, ev, "queue editor retains explicit text without ordinary send", json!({"text":text}), "setInput(await need('codex-queue-text'),A.text);return {ok:true};").await?;
        real_operation_click(app, fx, ck, ev, sid, "codex-queue-add", "queue_add", None).await?;
        real_operations_refresh(app, fx, ck, ev).await?;
    }
    let added = real_operations_snapshot(app, sid).await?;
    real_queue_diagnostic(app, fx, sid, "after-add", &added).await?;
    let queue = added["queue"].as_array().ok_or("real queue list missing")?;
    let a = queue.iter().find(|row| row["text"] == FIRST).and_then(|row| row["id"].as_str()).ok_or("queue A absent")?;
    let b = queue.iter().find(|row| row["text"] == SECOND).and_then(|row| row["id"].as_str()).ok_or("queue B absent")?;
    real_step(app, fx, ck, ev, "queue row edit targets original native submission", json!({"id":a,"text":UPDATED}), r#"
const row=await until(()=>$$('li[data-submission-id]').find(e=>e.dataset.submissionId===A.id));const edit=row?.querySelector('[data-testid="codex-queue-edit"]');if(!edit||edit.disabled)throw Error('queue edit absent');edit.click();setInput(await need('codex-queue-edit-text'),A.text);return {ok:true};
"#).await?;
    real_operation_click(app, fx, ck, ev, sid, "codex-queue-save", "queue_update", Some(a)).await?;
    real_operations_refresh(app, fx, ck, ev).await?;
    real_operation_click(app, fx, ck, ev, sid, "codex-queue-up", "queue_reorder", Some(b)).await?;
    real_operations_refresh(app, fx, ck, ev).await?;
    let reordered = real_operations_snapshot(app, sid).await?;
    real_require(fx, ck, ev, "native queue update preserves identity and explicit reorder changes order", reordered["queue"][0]["id"] == b && reordered["queue"][1]["id"] == a && reordered["queue"][1]["text"] == UPDATED, json!({"entries":2,"edited_identity_retained":true,"reordered":true,"content":"omitted"}))?;
    real_operation_click(app, fx, ck, ev, sid, "codex-queue-delete", "queue_delete", Some(a)).await?;
    real_operations_refresh(app, fx, ck, ev).await?;
    let deleted = real_operations_snapshot(app, sid).await?;
    let running = real_host(app, "get_agent_session", json!({"sessionId":sid})).await?;
    real_require(fx, ck, ev, "queue deletion removes exact entry while original parent stays active", deleted["queue"].as_array().is_some_and(|rows| rows.len() == 1 && rows[0]["id"] == b) && running["active_run"] == parent_run, json!({"remaining_entries":1,"original_parent_active":true}))?;
    real_step(app, fx, ck, ev, "explicitly interrupt bounded native parent after queue CRUD", json!({}), r#"
const stop=await until(()=>$$('.agent-control-send button[aria-label]',tid('dev-panel-agent')).find(b=>['停止本次执行','Stop this turn'].includes(b.getAttribute('aria-label'))&&!b.disabled));if(!stop)throw Error('native parent interrupt absent');stop.click();return {ok:true};
"#).await?;
    coding_wait(app, sid, 180, marker, approvals, |view| view["status"] == "idle" && view["active_run"].is_null()).await?;
    let pending = real_operations_snapshot(app, sid).await?;
    real_queue_diagnostic(app, fx, sid, "after-parent-interrupt", &pending).await?;
    if pending["queue"].as_array().is_some_and(|rows| rows.iter().any(|row| row["id"] == b)) {
        real_operations_refresh(app, fx, ck, ev).await?;
        let started = real_operation_click(app, fx, ck, ev, sid, "codex-queue-start", "queue_start", Some(b)).await?;
        let run = started["run_id"].as_str().ok_or("native queue supervised run missing")?;
        coding_wait(app, sid, 180, marker, approvals, |view| view["status"] == "idle" && view["active_run"].is_null() && real_events(view).any(|event| event["run"] == run && event["data"]["run_status"] == "completed")).await?;
        real_operation_durable(app, sid, run).await?;
        real_require(fx, ck, ev, "remaining native queue submission starts explicitly and completes durable run", true, json!({"explicit_start_exercised":true}))?;
    } else {
        let automatic = real_queue_diagnostic(app, fx, sid, "automatic-consumption", &pending).await?;
        real_require(fx, ck, ev, "native automatic queue consumption enters durable terminal supervision", automatic["automatic_completed_runs"].as_u64().is_some_and(|count| count > 0), json!({"explicit_start_exercised":false,"scheduler":"native automatic consumption; prompt does not hold queue","automatic":automatic}))?;
    }
    let after = real_operations_snapshot(app, sid).await?;
    real_require(fx, ck, ev, "native scheduler consumes remaining queued submission", after["queue"].as_array().is_some_and(Vec::is_empty), json!({"queue_entries":0,"ordinary_send_substitute":false}))
}

async fn real_child_resource_checks(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, sid: &str) -> Journey<()> {
    let observed = real_operations_snapshot(app, sid).await?;
    let children = observed["children"].as_array().ok_or("native children list absent")?;
    real_require(fx, ck, ev, "real native parent-child relation is observed", !children.is_empty(), json!({"observed_children":children.len(),"source":"actual native parent-child relation, not fixture"}))?;
    let child = children.iter().find(|child| child["turn_id"].is_string()).ok_or("no genuinely running observed child; no fabricated interrupt")?;
    let child_id = child["thread_id"].as_str().ok_or("observed child identity absent")?;
    real_operations_refresh(app, fx, ck, ev).await?;
    real_step(app, fx, ck, ev, "real subagent tree selects observed child and reads scoped history", json!({"child":child_id}), r#"
const select=await until(()=>$$('button',tid('codex-agents')).find(b=>txt(b).includes(A.child)&&!b.disabled));if(!select)throw Error('observed child tree row absent');select.click();await click('codex-agent-history');return {ok:!!await need('codex-agent-history-content')};
"#).await?;
    if observed["unsupported"].as_array().is_some_and(|kinds| kinds.iter().any(|kind| kind == "send_child")) {
        real_require(fx, ck, ev, "installed native child direct input limitation is explicit", true, json!({"send_child":"unsupported","send_exercised":false}))?;
    } else {
        real_step(app, fx, ck, ev, "child steering draft uses dedicated observed-child control", json!({}), r#"
const form=tid('codex-agent-send')?.closest('form');const input=form?.querySelector('textarea');if(!input)throw Error('child draft absent');setInput(input,'Keep the existing bounded read-only task; do not run another command, change files or start children. I will interrupt this child now.');return {ok:true};
"#).await?;
        match real_operation_click(app, fx, ck, ev, sid, "codex-agent-send", "send_child", None).await {
            Ok(_) => {},
            Err(error) if error == "inspect_codex_operation: host rejected (codex_operation_unsupported)" || error == "inspect_codex_operation: rejected (codex_operation_unsupported)" => {
                real_require(fx, ck, ev, "installed native child direct input is explicitly rejected as unsupported", true, json!({"send_child":"unsupported","send_exercised":false,"native_rejection_observed":true}))?;
            },
            Err(error) => return Err(error),
        }
    }
    real_operations_refresh(app, fx, ck, ev).await?;
    real_operation_click(app, fx, ck, ev, sid, "codex-agent-interrupt", "interrupt_child", None).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let next = real_operations_snapshot(app, sid).await?;
        if next["children"].as_array().is_some_and(|rows| rows.iter().any(|row| row["thread_id"] == child_id && row["turn_id"].is_null())) { break; }
        if tokio::time::Instant::now() >= deadline { return Err("child interrupt not confirmed by native state".into()); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    real_operations_refresh(app, fx, ck, ev).await?;
    let interrupted = real_operations_snapshot(app, sid).await?;
    real_require(fx, ck, ev, "installed protocol explicitly reports child close unsupported", interrupted["unsupported"].as_array().is_some_and(|kinds| kinds.iter().any(|kind| kind == "close_child")), json!({"close_child":"unsupported; no native close-agent client RPC","close_exercised":false}))?;
    real_step(app, fx, ck, ev, "unsupported child close stays disabled without sending a mutation", json!({}), "const b=await need('codex-agent-close');return {ok:b.disabled};").await?;
    real_operation_click(app, fx, ck, ev, sid, "codex-agent-resume", "resume_child", None).await?;
    Ok(())
}

async fn real_terminal_resource_checks(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, sid: &str) -> Journey<()> {
    let initial = real_operations_snapshot(app, sid).await?;
    let count = initial["terminals"].as_array().map_or(0, Vec::len);
    real_require(fx, ck, ev, "real native background processes are observed independently of child availability", count > 0, json!({"native_processes":count,"host_pty_substitute":false}))?;
    for _ in 0..count {
        real_operations_refresh(app, fx, ck, ev).await?;
        real_operation_click(app, fx, ck, ev, sid, "codex-background-stop", "terminate_terminal", None).await?;
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let stopped = loop {
        let snapshot = real_operations_snapshot(app, sid).await?;
        if snapshot["terminals"].as_array().is_some_and(Vec::is_empty) { break snapshot; }
        if tokio::time::Instant::now() >= deadline { return Err("native background termination not confirmed".into()); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    real_require(fx, ck, ev, "native background stop is confirmed by native process enumeration", stopped["terminals"].as_array().is_some_and(Vec::is_empty), json!({"native_processes_remaining":0,"host_pty_substitute":false}))?;
    Ok(())
}

async fn real_queue_diagnostic(app: &AppHandle, fx: &Fx, sid: &str, phase: &str, snapshot: &Value) -> Journey<Value> {
    let view = real_host(app, "get_agent_session", json!({"sessionId":sid})).await?;
    let workspace = real_host(app, "read_development_workspace", json!({})).await?;
    let runs = real_host(app, "list_development_runs", json!({"taskId":workspace["selection"]["task_id"]})).await?;
    let automatic: Vec<_> = runs.as_array().into_iter().flatten().filter(|run| run["session_id"] == sid && run["reason"] == "observed_native_automatic_turn").collect();
    let diagnostic = json!({"phase":phase,"queue_count":snapshot["queue"].as_array().map(Vec::len),"active_run":view["active_run"].is_string(),"automatic_runs":automatic.len(),"automatic_completed_runs":automatic.iter().filter(|run| run["status"] == "completed" && run["finished_at"].is_number()).count(),"native_automatic_markers":real_events(&view).filter(|event| event["kind"] == "nativeRun/observed").count(),"content":"omitted"});
    real_save(fx, "queue-diagnostic", &diagnostic);
    Ok(diagnostic)
}

async fn real_resource_setup_check(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, sid: &str, marker: &Path, approvals: &mut std::collections::HashSet<String>) -> Journey<()> {
    real_operation_branch_ready(app, fx, ck, ev, sid, marker, approvals).await?;
    const PROMPT: &str = "Authorized disposable-project native operations acceptance. Do not change files or use network. Spawn exactly one child agent to perform bounded read-only research: the child should run /bin/sleep 120 and then report 42, with no writes, network, escalation or further children. Do not wait for or close the child; I will explicitly steer, interrupt and resume it. Also start exactly one native background terminal command /bin/sleep 120 using exec_command with yield_time_ms=1 (not shell ampersand, nohup or an external daemon). Do not wait for or terminate that process; I will stop its native processId from the UI. Once those two native resources exist, give a brief final answer and stop this parent turn. If a native tool is unavailable, report that accurately; do not fabricate a resource.";
    let before = real_host(app, "get_agent_session", json!({"sessionId":sid})).await?;
    let count = real_events(&before).filter(|event| event["kind"] == "userMessage").count();
    coding_send(app, fx, ck, ev, "explicit native resource setup independent of queue scheduling", PROMPT).await?;
    coding_wait(app, sid, 240, marker, approvals, |view| view["status"] == "idle" && view["active_run"].is_null() && real_events(view).filter(|event| event["kind"] == "userMessage").count() > count).await?;
    real_operations_refresh(app, fx, ck, ev).await?;
    let child = real_child_resource_checks(app, fx, ck, ev, sid).await;
    let child_ok = real_operation_branch_result(fx, ck, ev, "child-controls", child)?;
    let terminals = real_terminal_resource_checks(app, fx, ck, ev, sid).await;
    let terminal_ok = real_operation_branch_result(fx, ck, ev, "native-terminal-controls", terminals)?;
    if child_ok && terminal_ok { Ok(()) } else { Err("one or more native resource branches failed".into()) }
}
