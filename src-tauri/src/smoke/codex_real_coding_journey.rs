// Explicit opt-in coding/operations acceptance. Never used by readonly33.
// Real CLI/model evidence stays separate from deterministic protocol fixtures.
const CODING_MATH: &str = "exports.subtotal = rows => rows.reduce((sum, row) => sum + row.price - row.quantity, 0);\n";
const CODING_FORMAT: &str = "exports.money = cents => '$' + Math.floor(cents / 100);\n";
const CODING_TEST: &str = r#"const assert = require('node:assert/strict');
const { subtotal } = require('./math.cjs');
const { money } = require('./format.cjs');
assert.equal(subtotal([{price:125,quantity:2},{price:50,quantity:3}]),400);
assert.equal(money(400),'$4.00');
assert.equal(money(5),'$0.05');
"#;
const CODING_INDEPENDENT: &str = r#"const assert = require('node:assert/strict');
const path = require('node:path');
const { subtotal } = require(path.join(process.argv[2], 'math.cjs'));
const { money } = require(path.join(process.argv[2], 'format.cjs'));
for (const [rows, expected] of [[[],0],[[{price:99,quantity:0}],0],[[{price:17,quantity:3},{price:25,quantity:2}],101]]) {
  assert.equal(subtotal(rows), expected);
}
for (const [cents, expected] of [[0,'$0.00'],[1,'$0.01'],[99,'$0.99'],[101,'$1.01'],[12345,'$123.45']]) assert.equal(money(cents),expected);
"#;

async fn coding_test_status(script: &Path, project: &Path) -> Journey<bool> {
    // No test output is persisted or relayed. This verifier is outside the model's
    // project; its fixed expectations are never taken from a model's success text.
    let node = dgos_agents::session::resolve_executable("node", None).map_err(|_| "isolated Node missing")?;
    let mut command = tokio::process::Command::new(node);
    command.arg(script).arg(project).current_dir(project).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    tokio::time::timeout(Duration::from_secs(20), command.status()).await.map_err(|_| "independent test timed out")?.map(|status| status.success()).map_err(|_| "independent test could not start".into())
}

fn coding_approval_allowed(command: &str, marker: &Path) -> bool {
    let expected = format!("/usr/bin/touch {}", marker.display());
    if command == expected { return true; }
    ["/bin/zsh", "/bin/bash", "/bin/sh"].iter().any(|shell| {
        ["-c", "-lc"].iter().any(|flag| ["'", "\""].iter().any(|quote| command == format!("{shell} {flag} {quote}{expected}{quote}")))
    })
}

async fn coding_wait<F: Fn(&Value) -> bool>(app: &AppHandle, sid: &str, seconds: u64, marker: &Path, approvals: &mut std::collections::HashSet<String>, ready: F) -> Journey<Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    loop {
        let view = real_host(app, "get_agent_session", json!({"sessionId":sid})).await?;
        if matches!(view["status"].as_str(), Some("error" | "unknown")) { return Err("real coding entered error/unknown; no replay".into()); }
        if let Some(pending) = view["pending"].as_array().filter(|rows| !rows.is_empty()) {
            for request in pending {
                let id = request["id"].as_str().ok_or("approval identity missing")?;
                if approvals.contains(id) { return Err("approval result not settled; never replay approval".into()); }
                let allow = approvals.is_empty() && request["kind"] == "item/commandExecution/requestApproval"
                    && request["data"]["threadId"] == view["native_id"]
                    && request["data"]["command"].as_str().is_some_and(|command| coding_approval_allowed(command, marker))
                    && request["data"]["availableDecisions"].as_array().is_none_or(|decisions| decisions.iter().any(|decision| decision == "accept"));
                approvals.insert(id.to_owned());
                let reply = dom(app, json!({"allow":allow,"id":id}), r#"
const section=await until(()=>$$('[data-testid="agent-native-request"]',tid('dev-panel-agent')).find(e=>e.getAttribute('data-request-id')===A.id));
if(!section)throw Error('exact native approval absent');
const button=section.querySelector(A.allow?'button[data-decision="accept"]':'button[data-decision="decline"]');if(!button||button.disabled)throw Error('native scoped decision absent');button.click();
return {ok:!!await until(()=>!document.body.contains(button)||button.disabled)};
"#).await;
                if !ui_ok(&reply) { return Err("actual approval UI did not submit scoped decision".into()); }
                if !allow { return Err("unexpected approval denied; only isolated marker allowed".into()); }
            }
            // Wait for the exact request to disappear; do not click a second time
            // while the native response is in flight.
            let settle = tokio::time::Instant::now() + Duration::from_secs(20);
            loop {
                let next = real_host(app, "get_agent_session", json!({"sessionId":sid})).await?;
                if next["pending"].as_array().is_none_or(|rows| rows.iter().all(|row| row["id"].as_str().is_none_or(|id| !approvals.contains(id)))) { break; }
                if tokio::time::Instant::now() >= settle { return Err("approval settlement unknown; no retry".into()); }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        } else if ready(&view) { return Ok(view); }
        if tokio::time::Instant::now() >= deadline { return Err("real coding observation timed out; no replay".into()); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn coding_send(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, name: &str, text: &str) -> Journey<()> {
    real_step(app, fx, ck, ev, name, json!({"text":text}), r#"
setInput(document.getElementById('agent-message'),A.text);const send=await until(()=>$$('.agent-control-send button[aria-label]',tid('dev-panel-agent')).find(b=>['发送','Send'].includes(b.getAttribute('aria-label'))&&!b.disabled));if(!send)throw Error('explicit send absent');send.click();return {ok:!!await until(()=>document.getElementById('agent-message')?.value==='')};
"#).await?;
    Ok(())
}

fn coding_native_test_exits(view: &Value, run: &str) -> Vec<i64> {
    real_events(view).filter(|event| event["run"] == run && event["kind"] == "commandExecution"
        && event["data"]["type"] == "commandExecution"
        && matches!(event["data"]["status"].as_str(), Some("completed" | "failed"))
        && event["data"]["command"].as_str().is_some_and(|command| command.contains("node test.cjs")))
        .filter_map(|event| event["data"]["exitCode"].as_i64()).collect()
}

async fn real_coding_journey(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, approvals: &mut std::collections::HashSet<String>, operations: bool) -> Journey<()> {
    let settings = REAL_CODEX_SETTINGS.get().ok_or("real settings absent")?;
    let (account, _) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|_| "isolated account absent")?;
    let owner = account.user_id().to_owned();
    let preferences = real_host(app, "get_cli_preferences", json!({})).await?;
    let mut paths = preferences["paths"].clone(); paths["codex"] = json!(settings.executable);
    let saved = real_host(app, "set_cli_preferences", json!({"paths":paths,"expectedRevision":preferences["revision"]})).await?;
    real_require(fx, ck, ev, "real coding selects installed CLI explicitly", saved["saved"] == true, json!({"native_cli":true,"protocol_fixture":false}))?;
    let vault = app.state::<Arc<dgos_providers::test_support::MemoryCredentialVault>>();
    let key = settings.key.lock().map_err(|_| "credential lock failed")?.take().ok_or("credential already consumed")?;
    let provider = dgos_providers::test_support::seed_provider(fx.db.clone(), vault.inner(), dgos_types::ProviderConfig { name:"Authorized real coding acceptance".into(),provider_type:"openai_compatible".into(),base_url:settings.endpoint.clone(),api_key:key,enabled:true,config:None }, &owner).await.map_err(|_| "authorized Provider seed failed")?;
    app.state::<dgos_providers::ProviderManager>().add_manual_model(&provider.id, &owner, &settings.model, None).await.map_err(|_| "authorized model seed failed")?;
    let path = fx.root.join("real-coding-project"); make_repo(&path);
    for (name, content) in [("math.cjs",CODING_MATH),("format.cjs",CODING_FORMAT),("test.cjs",CODING_TEST)] { fs::write(path.join(name), content).map_err(|_| "coding fixture creation failed")?; }
    if !git(&path, &["add", "math.cjs", "format.cjs", "test.cjs"]) || !git(&path, &["-c", "user.name=dgos", "-c", "user.email=dgos@example.invalid", "-c", "core.hooksPath=/dev/null", "commit", "-q", "-m", "isolated failing multi-file fixture"]) { return Err("coding fixture baseline commit failed".into()); }
    let independent = fx.root.join("independent-tests.cjs");
    fs::write(&independent, CODING_INDEPENDENT).map_err(|_| "independent verifier creation failed")?;
    let initial_visible = coding_test_status(&path.join("test.cjs"), &path).await?;
    let initial_independent = coding_test_status(&independent, &path).await?;
    real_require(fx, ck, ev, "both actual initial test processes fail before model dispatch", !initial_visible && !initial_independent, json!({"visible_exit_success":initial_visible,"independent_exit_success":initial_independent,"model_dispatched":false,"stdout":"not captured"}))?;
    let project = real_host(app, "create_development_project", json!({"input":{"kind":"import","name":"Real Codex multi-file acceptance","path":path,"source":null}})).await?;
    real_require(fx, ck, ev, "real coding opens shipped App workbench", goto(app, "main", "/tasks/agent").await, json!({"renderer":"App/WKWebView"}))?;
    real_step(app, fx, ck, ev, "real coding observes production IPC without substituting responses", json!({"project":project["id"],"model":settings.model}), r#"
const original=window.fetch;window.__realCodex={original,events:[]};window.fetch=function(url,options,...rest){const command=String(url).split('/').at(-1);let args={};try{args=JSON.parse(options?.body||'{}')}catch{}window.__realCodex.events.push({command,action:args.input?.action?.kind,request_id:args.input?.request_id,image_count:args.input?.images?.length,text_empty:args.input?.text===''});return original.call(this,url,options,...rest)};
const projects=await need('dev-tool-projects');if(projects.getAttribute('aria-expanded')!=='true')projects.click();await click('dev-project-item-'+A.project);await need('dev-project-conversation');
await chooseConversationModel('codex','Authorized real coding acceptance',A.model);await authorizeConversation();return {ok:!!await until(()=>!tid('dev-project-conversation')&&document.getElementById('agent-message')&&!document.getElementById('agent-message').disabled)&&window.__realCodex.events.filter(e=>e.command==='open_agent_session').length===1&&!window.__realCodex.events.some(e=>e.command==='send_agent_message')&&!tid('direct-conversation-authorize')&&!$$('[role=alertdialog]').length};
"#).await?;
    if operations { real_image_draft_checks(app, fx, ck, ev).await?; }
    real_step(app, fx, ck, ev, "real coding automatically prepares trusted workspace without sending", json!({}), r#"
return {ok:!tid('dev-project-conversation')&&!!document.getElementById('agent-message')&&!document.getElementById('agent-message').disabled&&window.__realCodex.events.filter(e=>e.command==='open_agent_session').length===1&&!window.__realCodex.events.some(e=>e.command==='send_agent_message')&&!tid('direct-conversation-authorize')&&!$$('[role=alertdialog]').length};
"#).await?;
    let sessions = real_host(app, "list_agent_sessions", json!({})).await?;
    let rows = sessions.as_array().ok_or("session rows absent")?;
    real_require(fx, ck, ev, "real coding prepares exactly one session", rows.len() == 1, json!({"sessions":rows.len()}))?;
    let sid = rows[0]["id"].as_str().ok_or("session ID absent")?.to_owned();
    let marker = fx.root.join("approved-native-marker");
    let prepared = coding_wait(app, &sid, 60, &marker, approvals, |view| view["status"] == "idle").await?;
    real_require(fx, ck, ev, "real coding prepared session has native ownership and no turn", prepared["native_id"].is_string() && prepared["active_run"].is_null() && real_events(&prepared).all(|event| event["kind"] != "userMessage"), real_summary(&prepared))?;
    let prompt = format!("Authorized acceptance in this disposable project. Fix two bugs: math.cjs subtotal must sum price * quantity; format.cjs money must format nonnegative integer cents as dollars with exactly two decimals. First run node test.cjs and observe its real failure. Then request one real native command approval by executing EXACTLY `/usr/bin/touch {}` with sandbox_permissions=require_escalated and a justification asking me to authorize this one acceptance marker outside the workspace. This marker is explicitly authorized and is the ONLY allowed access outside the project. Do not change the command, add shell operations, read other directories, use network, install packages, start background processes or ask questions. After I approve, fix BOTH source files, leave test.cjs unchanged, run node test.cjs again and report the result. Keep the work in default coding mode, not Plan.", marker.display());
    real_capture_check(app, fx, ck, ev, "coding-before-send").await?;
    coding_send(app, fx, ck, ev, "explicit real coding send dispatches authorized bugfix", &prompt).await?;
    let completed = coding_wait(app, &sid, 600, &marker, approvals, |view| view["status"] == "idle" && view["active_run"].is_null() && real_events(view).any(|event| event["data"]["run_status"] == "completed")).await?;
    real_require(fx, ck, ev, "actual native approval accepted and command produced isolated marker", !approvals.is_empty() && marker.is_file(), json!({"actual_approvals":approvals.len(),"marker_exists":marker.is_file(),"synthetic_approval":false}))?;
    let coding_run = real_events(&completed).find(|event| event["kind"] == "userMessage").and_then(|event| event["run"].as_str()).ok_or("coding user run identity absent")?;
    let test_exits = coding_native_test_exits(&completed, coding_run);
    real_require(fx, ck, ev, "model actually runs failing tests then passing tests through native tools", test_exits.first().is_some_and(|code| *code != 0) && test_exits.last() == Some(&0), json!({"native_test_exits":test_exits,"tool_output":"omitted"}))?;
    let math_changed = fs::read_to_string(path.join("math.cjs")).map_err(|_| "math source missing")? != CODING_MATH;
    let format_changed = fs::read_to_string(path.join("format.cjs")).map_err(|_| "format source missing")? != CODING_FORMAT;
    let tests_unchanged = fs::read_to_string(path.join("test.cjs")).map_err(|_| "visible tests missing")? == CODING_TEST && fs::read_to_string(&independent).map_err(|_| "independent verifier missing")? == CODING_INDEPENDENT;
    real_require(fx, ck, ev, "real model edits both source files without changing acceptance tests", math_changed && format_changed && tests_unchanged, json!({"source_files_changed":usize::from(math_changed)+usize::from(format_changed),"tests_unchanged":tests_unchanged,"file_content":"omitted"}))?;
    let visible_pass = coding_test_status(&path.join("test.cjs"), &path).await?;
    let independent_pass = coding_test_status(&independent, &path).await?;
    real_require(fx, ck, ev, "host independent real test processes pass after native edits", visible_pass && independent_pass, json!({"visible_exit_success":visible_pass,"independent_exit_success":independent_pass,"verifier":"fixed external tests, not model report","stdout":"not captured"}))?;
    let workspace = real_host(app, "read_development_workspace", json!({})).await?;
    let task = workspace["selection"]["task_id"].as_str().ok_or("coding task absent")?.to_owned();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let baseline = loop {
        let runs = real_host(app, "list_development_runs", json!({"taskId":task})).await?;
        if runs.as_array().is_some_and(|rows| rows.iter().filter(|run| run["session_id"] == sid).count() == 1 && rows.iter().filter(|run| run["session_id"] == sid).all(|run| run["finished_at"].is_number())) { break runs; }
        if tokio::time::Instant::now() >= deadline { return Err("coding durable terminal projection timed out".into()); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let coding_runs: Vec<_> = baseline.as_array().ok_or("coding runs absent")?.iter().filter(|run| run["session_id"] == sid).collect();
    real_require(fx, ck, ev, "real coding terminal outcome persists in one durable run", coding_runs.len() == 1 && coding_runs[0]["status"] == "completed" && coding_runs[0]["finished_at"].is_number(), json!({"durable_runs":coding_runs.len(),"status":"completed"}))?;
    if operations {
        real_save(fx, "image-history-before-close", &coding_image_history_summary(&completed));
        real_step(app, fx, ck, ev, "prepared composer first-send image reaches actual native dispatch before close", json!({}), "const sends=window.__realCodex.events.filter(e=>e.command==='send_agent_message');return {ok:sends.length===1&&sends[0].image_count===1&&!tid('agent-image-preview-0')};").await?;
    }
    coding_close_resume(app, fx, ck, ev, &sid, &marker, approvals).await?;
    if operations {
        let restored = real_host(app, "get_agent_session", json!({"sessionId":sid})).await?;
        let metadata = coding_image_history_summary(&restored);
        real_save(fx, "image-history-after-resume", &metadata);
        let reply = dom(app, json!({}), "const present=!!await until(()=>document.querySelector('[data-testid=\"agent-image-history\"]'),5000);return {ok:present&&!tid('agent-image-preview-0'),present,draft_empty:!tid('agent-image-preview-0')};").await;
        record(ck, ev, "resumed transcript restores content-free image history metadata", ui_ok(&reply), json!({"host":metadata,"history_marker_present":reply["value"]["present"].as_bool(),"active_image_draft_empty":reply["value"]["draft_empty"].as_bool(),"browser_error_and_content":"omitted"}));
        real_save(fx, "checkpoint", &json!({"step":"resumed transcript restores content-free image history metadata","passed":ck.passed,"failed":ck.failed,"checks":ev}));
    }
    let mut operations_completed = false;
    if operations {
        match real_operation_checks(app, fx, ck, ev, &sid, &marker, approvals).await {
            Ok(()) => operations_completed = true,
            Err(error) => {
                if error == "screenshot masking restoration failed" { return Err(error); }
                record(ck, ev, "independent native operation acceptance completes", false, error);
                real_save(fx, "checkpoint", &json!({"step":"independent native operation acceptance completes","passed":ck.passed,"failed":ck.failed,"checks":ev}));
            }
        }
    }
    real_require(fx, ck, ev, "independent tests still pass after native operations and close-resume", coding_test_status(&independent, &path).await?, json!({"restored_disk_verification":"passed","verifier":"fixed external tests","output":"discarded"}))?;
    real_capture_check(app, fx, ck, ev, "coding-resumed").await?;
    real_save(fx, "coding", &json!({"source":"actual App/WKWebView → production IPC → installed Codex → authorized Provider","protocol_fixture":false,"initial_tests_failed":true,"source_files_edited":2,"actual_approval_count":approvals.len(),"independent_tests_passed":true,"native_thread_retained":true,"resume_replay_observed":false,"completed":real_summary(&completed),"raw_content_in_evidence":false,"operations_requested":operations,"operations_completed":operations_completed}));
    Ok(())
}

fn coding_image_history_summary(view: &Value) -> Value {
    let users: Vec<_> = real_events(view).filter(|event| event["kind"] == "userMessage").map(|event| {
        let content = event["data"]["content"].as_array();
        let types: Vec<_> = content.into_iter().flatten().filter_map(|part| part["type"].as_str()).map(|kind| match kind {
            "text" | "image" | "localImage" | "input_image" | "image_url" | "skill" | "mention" => kind,
            _ => "other",
        }).collect();
        let image_blocks = types.iter().filter(|kind| matches!(**kind, "image" | "localImage" | "input_image" | "image_url")).count();
        json!({"image_metadata_count":event["data"]["images"].as_array().map_or(0, Vec::len),"image_content_count":image_blocks,"content_types":types})
    }).collect();
    json!({"history_truncated":view["history_truncated"].as_bool(),"events":real_events(view).count(),"user_messages":users.len(),"per_user":users})
}

async fn coding_close_resume(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, sid: &str, marker: &Path, approvals: &mut std::collections::HashSet<String>) -> Journey<()> {
    let before = real_host(app, "get_agent_session", json!({"sessionId":sid})).await?;
    let workspace = real_host(app, "read_development_workspace", json!({})).await?;
    let task = workspace["selection"]["task_id"].as_str().ok_or("resume task absent")?;
    let runs_before = real_host(app, "list_development_runs", json!({"taskId":task})).await?;
    real_step(app, fx, ck, ev, "capture explicit-send count before close", json!({}), "window.__realCodex.sendsBeforeClose=window.__realCodex.events.filter(e=>e.command==='send_agent_message').length;return {ok:true};").await?;
    real_step(app, fx, ck, ev, "coding closes native session explicitly", json!({}), r#"
const close=await until(()=>$$('.agent-dialogue-actions button',tid('dev-panel-agent')).find(b=>['结束会话','End session'].includes(txt(b))&&!b.disabled));if(!close)throw Error('close absent');close.click();return {ok:!!await until(()=>tid('saved-conversation-composer'))};
"#).await?;
    let closed = coding_wait(app, sid, 60, marker, approvals, |view| view["status"] == "closed").await?;
    real_require(fx, ck, ev, "coding close keeps native thread", closed["native_id"] == before["native_id"], real_summary(&closed))?;
    real_step(app, fx, ck, ev, "coding focuses authorized saved composer to resume original thread", json!({"sid":sid}), r#"
await prepareSavedConversation(A.sid);return {ok:!!document.getElementById('agent-message')&&!tid('saved-conversation-composer')};
"#).await?;
    let resumed = coding_wait(app, sid, 60, marker, approvals, |view| view["status"] == "idle").await?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let quiet = real_host(app, "get_agent_session", json!({"sessionId":sid})).await?;
    let runs_after = real_host(app, "list_development_runs", json!({"taskId":task})).await?;
    let run_ids = |view: &Value| -> Vec<String> { let mut ids: Vec<_> = view.as_array().into_iter().flatten().filter(|run| run["session_id"] == sid).filter_map(|run| run["id"].as_str().map(str::to_owned)).collect(); ids.sort(); ids };
    real_require(fx, ck, ev, "coding resume retains thread and fresh generation with zero replay", resumed["native_id"] == before["native_id"] && resumed["generation"] != before["generation"] && quiet["status"] == "idle" && quiet["active_run"].is_null() && quiet["pending"].as_array().is_none_or(Vec::is_empty) && run_ids(&runs_before) == run_ids(&runs_after) && real_events(&quiet).filter(|event| event["kind"] == "userMessage").count() == real_events(&resumed).filter(|event| event["kind"] == "userMessage").count(), json!({"native_thread_retained":true,"fresh_generation":true,"new_runs":0,"quiet_seconds":2,"raw_history":"omitted"}))?;
    real_step(app, fx, ck, ev, "coding resume emits no implicit ordinary send", json!({}), "return {ok:window.__realCodex.events.filter(e=>e.command==='send_agent_message').length===window.__realCodex.sendsBeforeClose};").await?;
    Ok(())
}

include!("codex_real_operations_journey.rs");

#[cfg(test)]
mod real_acceptance_boundary_fixtures {
    use super::*;
    #[test]
    fn synthetic_approval_command_fixture_rejects_scope_expansion() {
        let marker = Path::new("/private/tmp/dgos-fixture/approved-native-marker");
        for command in ["/usr/bin/touch /private/tmp/dgos-fixture/approved-native-marker", "/bin/zsh -lc '/usr/bin/touch /private/tmp/dgos-fixture/approved-native-marker'"] { assert!(coding_approval_allowed(command, marker)); }
        for command in ["touch /private/tmp/dgos-fixture/approved-native-marker", "/usr/bin/touch /private/tmp/dgos-fixture/approved-native-marker; echo extra", "/usr/bin/touch /private/tmp/other-marker", "/bin/zsh -lc '/usr/bin/touch /private/tmp/dgos-fixture/approved-native-marker && echo extra'", "/usr/bin/touch /private/tmp/dgos-fixture/approved-native-marker\n"] { assert!(!coding_approval_allowed(command, marker)); }
    }
    #[test]
    fn synthetic_projected_command_fixture_retains_real_exits_for_exact_run() {
        let view = json!({"events":[
            {"run":"other","kind":"commandExecution","data":{"type":"commandExecution","status":"failed","command":"node test.cjs","exitCode":99}},
            {"run":"coding","kind":"item/started","data":{"type":"commandExecution","status":"inProgress","command":"node test.cjs","exitCode":null}},
            {"run":"coding","kind":"commandExecution","data":{"type":"commandExecution","status":"failed","command":"node test.cjs","exitCode":1}},
            {"run":"coding","kind":"commandExecution","data":{"type":"commandExecution","status":"completed","command":"node unrelated.cjs","exitCode":77}},
            {"run":"coding","kind":"commandExecution","data":{"type":"commandExecution","status":"inProgress","command":"node test.cjs","exitCode":88}},
            {"run":"coding","kind":"commandExecution","data":{"type":"commandExecution","status":"completed","command":"node test.cjs","exitCode":0}}
        ]});
        assert_eq!(coding_native_test_exits(&view, "coding"), vec![1, 0]);
        assert!(coding_native_test_exits(&view, "absent").is_empty());
    }
}
