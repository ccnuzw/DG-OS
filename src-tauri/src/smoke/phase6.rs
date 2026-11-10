//! D133 joint journey. Production UI/IPC and isolated SQLite; upstream is loopback, not a vendor.
use super::*;

pub(super) async fn run(app: &AppHandle, checks: &mut Checks, fx: &Fixture) {
    let loaded = goto(app, MAIN, "/").await;
    checks.check("[12] desktop opens through production shell", loaded, "desktop load");
    let preferences=js(app,r#"
      const organize=await until(()=>$('.desktop-header details summary')); organize.click();
      const entries=await until(()=>{const c=$$('.desktop-header details input[type=checkbox]')[0];return c&&!c.disabled&&c;}); entries.click();
      if (!await until(()=>!$('[aria-label="快捷入口"]'))) return false;
      for(let i=0;i<100;i++){const snapshot=await window.__TAURI_INTERNALS__.invoke('get_desktop_preferences');if(snapshot.success&&snapshot.data.values.showEntries===false)return true;await wait(40);} return false;
    "#).await;
    let snapshot=ipc(app,MAIN,"get_desktop_preferences",json!({})).await;
    let reloaded=goto(app,MAIN,"/").await;
    let restored=js(app,"return !!await until(()=>{const c=$$('.desktop-header details input[type=checkbox]')[0];return c&&!c.disabled&&!c.checked;});").await;
    checks.check("[12] desktop preference saves through UI and survives a new renderer lifecycle",preferences==true&&snapshot.data()["values"]["showEntries"]==false&&reloaded&&restored==true,snapshot.describe());
    let created = js(app, r#"
      const link = $$('a').find(a=>a.getAttribute('href')==='/workbench'); link.click();
      await until(()=>location.pathname==='/workbench');
      const button = await until(()=>$('button[aria-label="新建会话"]')||btn('新建会话')); button.click();
      const target=await until(()=>$('button[aria-label="会话目标"]')); target.click();
      await choose('Provider','Smoke Loopback'); await choose('模型','chat-model');
      const choices=await until(()=>{const c=$$('fieldset [role=checkbox]');return c.length>=2&&c;});
      choices[0].click(); await until(()=>choices[0].getAttribute('aria-checked')==='true'); choices[1].click(); await until(()=>choices[1].getAttribute('aria-checked')==='true');
      const move=await until(()=>$$('button').find(b=>b.getAttribute('aria-label')?.startsWith('上移 ')&&!b.disabled)); const moved=move.getAttribute('aria-label'); move.click();
      await until(()=>$$('button').find(b=>b.getAttribute('aria-label')===moved)?.disabled);
      const order=$$('button').map(b=>b.getAttribute('aria-label')||'').filter(label=>label.startsWith('上移 ')).map(label=>label.slice(3));
      await choose('回复长度','中 · 4,096 tokens');
      const guide=$$('textarea').find(el=>labelOf(el)==='自定义指导'); setVal(guide,'PHASE6-GUIDANCE');
      btn('应用目标').click(); await until(()=>!$('section[aria-label="会话目标"]'));
      const input=await until(()=>$('textarea[aria-label="消息输入"]')); setVal(input,'PHASE6-DRAFT'); window.__p6Input=input;
      return {ready:!!input,order};
    "#).await;
    checks.check("[12] model and generation controls save before send",created["ready"]==true,created.to_string());
    let long_partial = format!("{}\nPHASE6-PARTIAL", (0..60).map(|i| format!("PHASE6-SCROLL-{i}")).collect::<Vec<_>>().join("\n"));
    fx.up.script(Script::Sse(vec![delta(&long_partial),Step::Gate("p6-tail".into()),delta("-FINAL")].into_iter().chain(stop_steps()).collect()));
    let before=fx.up.bodies();
    let started=js(app,"await send('PHASE6-MESSAGE'); return !!await until(()=>txt($(STREAM)).includes('PHASE6-PARTIAL')); ").await;
    checks.check("[12] production Channel renders loopback partial",started==true,started.to_string());
    let kept=js(app,r#"
      setVal($('textarea[aria-label="消息输入"]'),'PHASE6-NEXT-DRAFT');
      window.__p6Input.setSelectionRange(3,8);
      const scroller=$('.chat-message-scroll'); scroller.scrollTop=40; scroller.dispatchEvent(new Event('scroll')); window.__p6Scroll=scroller.scrollTop;
      $$('a').find(a=>a.getAttribute('href')==='/settings').click(); await until(()=>$('[data-settings-shell]'));
      const category=$$('[role=tab]').find(el=>txt(el)==='外观'); kd(category,'Enter');
      const dark=await until(()=>{const label=$$('label').find(el=>txt(el)==='暗色');const radio=label&&$('[role=radio]',label.parentElement);return radio&&!radio.disabled&&radio;});
      if(!dark)return {error:'theme unavailable',groups:$$('[role=radiogroup]').map(el=>({name:labelOf(el),text:txt(el),html:el.outerHTML})),body:document.body.textContent};
      window.__p6OldTheme=document.documentElement.dataset.theme; dark.click();
      if(!await until(()=>document.documentElement.dataset.theme!==window.__p6OldTheme))return {error:'preview did not change',body:document.body.textContent};
      btn('取消预览').click();
      return {cancelled:document.documentElement.dataset.theme===window.__p6OldTheme};
    "#).await;
    let applied=js(app,r#"
      const label=$$('label').find(el=>txt(el)==='暗色'); $('[role=radio]',label.parentElement).click();
      const apply=await until(()=>{const b=btn('应用外观');return b&&!b.disabled&&b;});
      if(!apply)return {error:'apply unavailable',body:document.body.textContent};
      apply.click();
      const saved=await until(()=>{const b=btn('应用外观');return b&&b.disabled&&!b.getAttribute('aria-busy')&&document.documentElement.dataset.theme==='dark';});
      return {saved:!!saved,body:saved?null:document.body.textContent};
    "#).await;
    let restored_work=js(app,r#"
      btn('关闭设置').click();
      if(!await until(()=>!$('[data-settings-shell]')))return {error:'settings did not close',body:document.body.textContent};
      return {same:window.__p6Input===$('textarea[aria-label="消息输入"]'),draft:$('textarea[aria-label="消息输入"]').value,selection:window.__p6Input.selectionStart===3&&window.__p6Input.selectionEnd===8,scroll:window.__p6Scroll>0&&Math.abs($('.chat-message-scroll').scrollTop-window.__p6Scroll)<2,partial:txt($(STREAM)).includes('PHASE6-PARTIAL')};
    "#).await;
    checks.check("[12] running chat preserves draft, selection and scroll across theme cancel/apply",restored_work["same"]==true&&restored_work["draft"]=="PHASE6-NEXT-DRAFT"&&restored_work["selection"]==true&&restored_work["scroll"]==true&&restored_work["partial"]==true&&kept["cancelled"]==true&&applied["saved"]==true,format!("preview={kept} apply={applied} work={restored_work}"));
    let route = js(app, "$$('a').find(a=>a.getAttribute('href')==='/settings').click(); await until(()=>$('[data-settings-shell]')); const storage=await until(()=>$$('[role=tab]').find(el=>txt(el)==='存储')); kd(storage,'Enter'); return !!await until(()=>location.pathname==='/settings/storage');").await;
    let busy_preview = ipc(app,MAIN,"preview_vacuum",json!({})).await;
    let busy = ipc(app,MAIN,"execute_vacuum",json!({"input":{"previewId":busy_preview.data()["preview_id"],"confirmed":true}})).await;
    checks.check("[12] live generation refuses database maintenance", route==true && busy_preview.success() && !busy.success(), busy.describe());
    js(app,"btn('关闭设置').click(); await until(()=>!$('[data-settings-shell]')); return true;").await;
    fx.up.release("p6-tail");
    let done=js(app,"return !!await until(()=>txt(lastDone()).includes('PHASE6-PARTIAL-FINAL'),12000);").await;
    let body=if fx.up.bodies()>before {fx.up.body(before)}else{Value::Null};
    let ordered = body["messages"].as_array().is_some_and(|messages| {
        let ids = created["order"].as_array().map(|ids| ids.iter().filter_map(Value::as_str).collect::<Vec<_>>()).unwrap_or_default();
        let positions = ids.iter().map(|id| messages.iter().position(|message| message["content"].as_str().is_some_and(|text| text.contains(id)))).collect::<Option<Vec<_>>>();
        positions.is_some_and(|positions| positions.len() == 2 && positions[0] < positions[1])
    });
    checks.check("[12] saved controls and ordered skills reach loopback exactly once",done==true&&fx.up.bodies()==before+1&&body["max_tokens"]==4096&&body.to_string().contains("PHASE6-GUIDANCE")&&ordered,format!("done={done} requests={} body={body}",fx.up.bodies()-before));
    let focus=js(app,r#"
      const focus=$('button[aria-label="专注"]'); focus.click(); const focused=await until(()=>$('button[aria-label="专注"][aria-pressed="true"]')); focused.click();
      const skills=await until(()=>$$('a').find(a=>a.getAttribute('href')==='/skills')); skills.click(); await until(()=>location.pathname==='/skills');
      const chat=await until(()=>$$('a').find(a=>a.getAttribute('href')==='/workbench')); chat.click(); await until(()=>location.pathname==='/workbench');
      return {same:window.__p6Input===$('textarea[aria-label="消息输入"]'),draft:$('textarea[aria-label="消息输入"]').value,done:txt(lastDone()).includes('PHASE6-PARTIAL-FINAL')};
    "#).await;
    checks.check("[12] resource navigation and focus restore preserve original work surface",focus["same"]==true&&focus["draft"]=="PHASE6-NEXT-DRAFT"&&focus["done"]==true,focus.to_string());
    let module_setup = js(app,r#"
      $$('a').find(a=>a.getAttribute('href')==='/settings').click(); await until(()=>$('[data-settings-shell]'));
      const category=await until(()=>$$('[role=tab]').find(el=>txt(el)==='模块')); kd(category,'Enter');
      return !!await until(()=>location.pathname==='/settings/modules');
    "#).await;
    let enable=ipc(app,MAIN,"enable_module",json!({"moduleId":"provider-console"})).await;
    let operation=if let Some(id)=enable.data()["operation_id"].as_str(){super::super::poll_operation(app,MAIN,id).await}else{Value::Null};
    let grant=ipc(app,MAIN,"grant_module_permission",json!({"moduleId":"provider-console","permission":"providers.call"})).await;
    checks.check("[12] module starts with explicit host permission",module_setup==true&&grant.success()&&operation["state"]=="succeeded",format!("{} {} {operation}",grant.describe(),enable.describe()));
    let start=ipc(app,MAIN,"start_module",json!({"moduleId":"provider-console"})).await;
    let started=if let Some(id)=start.data()["operation_id"].as_str(){super::super::poll_operation(app,MAIN,id).await}else{Value::Null};
    checks.check("[12] enabled module reaches ready runtime",started["state"]=="succeeded",started.to_string());
    fx.up.script(answer_json("PHASE6-MODULE-RESULT"));
    let module_catalog=js(app,r#"
      btn('关闭设置').click(); await until(()=>!$('[data-settings-shell]'));
      history.pushState({},'', '/apps/provider-console/documents/phase6-one'); window.dispatchEvent(new PopStateEvent('popstate'));
      await until(()=>$('[data-testid="provider-console"]'));
      await choose('服务','Smoke Loopback'); await choose('模型','chat-model');
      const input=$$('[data-testid="provider-console"] textarea')[0]; setVal(input,'PHASE6-MODULE-DRAFT'); window.__p6Module=input;
      const surface=$('[data-testid="provider-console"]'); btn('发送',surface).click();
      if(!await until(()=>txt(surface).includes('PHASE6-MODULE-RESULT'),12000))return false;
      history.pushState({},'', '/apps/provider-console/documents/phase6-two'); window.dispatchEvent(new PopStateEvent('popstate'));
      return !!await until(()=>$$('[data-module-view]').length===2);
    "#).await;
    let multi=js(app,r#"
      const views=$$('[data-module-view]'); const first=window.__p6Module;
      const runtimeIds=[...new Set(views.map(v=>v.closest('[data-runtime-id]')?.getAttribute('data-runtime-id')))];
      const second=views.find(v=>!v.contains(first));
      return {two:views.length===2,separate:!!second&&!second.querySelector('textarea')?.value,kept:first.isConnected&&first.value==='PHASE6-MODULE-DRAFT',oneRuntime:runtimeIds.length===1};
    "#).await;
    checks.check("[12] real Provider catalog and independent module views preserve the first draft",module_catalog==true&&multi["two"]==true&&multi["separate"]==true&&multi["kept"]==true&&multi["oneRuntime"]==true,multi.to_string());
    js(app,r#"$$('a').find(a=>a.getAttribute('href')==='/workbench').click(); await until(()=>location.pathname==='/workbench'); return true;"#).await;
    if fx.mcp_note.is_none() {
        fx.up.script(proposal("PHASE6-APPROVAL"));
        fx.up.script(answer_json("PHASE6-APPROVED"));
        let before = fx.up.bodies();
        let approved = js(app, r#"
          btnHas('MCP 工具').click();
          const tool=await until(()=>$$('[role=checkbox]').find(el=>labelOf(el).includes('fixture-server / echo'))); tool.click();
          await send('PHASE6-TOOL');
          const proposal=await until(()=>$(PROPOSAL),12000);
          btn('批准本次调用',proposal).click();
          return !!await until(()=>idle()&&txt(lastDone()).includes('PHASE6-APPROVED'),15000);
        "#).await;
        checks.check("[12] tool configuration and approval execute one controlled MCP round trip",approved==true&&fx.up.bodies()==before+2,approved.to_string());
    } else {
        checks.check("[12] tool fixture available",false,fx.mcp_note.as_deref().unwrap_or("missing fixture"));
    }
    fx.up.script(Script::Sse(vec![delta("PHASE6-STOP"),Step::Hold]));
    let closed = js(app,r#"
      await send('PHASE6-STOP-MESSAGE'); await until(()=>txt($(STREAM)).includes('PHASE6-STOP'));
      $('button[aria-label="关闭 AI 助手"]').click();
      const stop=await until(()=>btn('停止并关闭')); stop.click();
      return !!await until(()=>!$('button[aria-label="关闭 AI 助手"]'),12000);
    "#).await;
    let terminal = wait_rows(&fx.db,&turn_sql("PHASE6-STOP-MESSAGE"),terminal,10).await;
    checks.check("[12] stop-and-close waits for the persisted interrupted terminal",closed==true&&terminal.first().is_some_and(|r|r["status"]=="interrupted"),format!("closed={closed} rows={terminal:?}"));
    let reopened = js(app,r#"
      $$('a').find(a=>a.getAttribute('href')==='/workbench').click(); await until(()=>location.pathname==='/workbench');
      const history=await until(()=>btn('会话历史')||$('button[aria-label="会话历史"]')); history.click();
      const row=await until(()=>$$('[aria-label="会话列表"] button').find(b=>txt(b).includes('PHASE6-MESSAGE'))); row.click();
      return !!await until(()=>document.body.textContent.includes('PHASE6-PARTIAL-FINAL'));
    "#).await;
    checks.check("[12] reopening the saved conversation restores its real message history",reopened==true,reopened.to_string());
    let cli = ipc(app,MAIN,"probe_cli",json!({"tool":"node","path":null})).await;
    checks.check("[12] real installed Node version probe",cli.success()&&cli.data()["status"]=="available"&&cli.data()["managed"]==false,cli.describe());
    media_journey(app, checks).await;
    storage_journey(app, checks, fx).await;
    block_account_workspace(app,checks,fx).await;
    println!("INFO phase6 external service boundary: loopback HTTP and isolated SQLite; no microphone or vendor evidence");
}

async fn media_journey(app: &AppHandle, checks: &mut Checks) {
    let imported = ipc(app, MAIN, "import_chat_asset", json!({"input":{"name":"phase6.txt","mime":"text/plain","base64":"UEhBU0U2LUFUVEFDSE1FTlQ="}})).await;
    let id = imported.data()["id"].as_str().unwrap_or("").to_owned();
    let preview = ipc(app, MAIN, "preview_chat_asset", json!({"assetId":id})).await;
    let removed = ipc(app, MAIN, "remove_chat_asset", json!({"assetId":id})).await;
    let absent = ipc(app, MAIN, "preview_chat_asset", json!({"assetId":id})).await;
    checks.check("[12] controlled text asset imports previews removes and becomes unavailable", imported.success() && preview.data()["base64"] == "UEhBU0U2LUFUVEFDSE1FTlQ=" && removed.data()["removed"] == true && !absent.success(), format!("{} {} {} {}", imported.describe(), preview.describe(), removed.describe(), absent.describe()));
    let providers = ipc(app, MAIN, "list_providers", json!({})).await;
    let provider = providers.data().as_array().and_then(|rows| rows.iter().find(|row| row["name"] == PROVIDER_NAME)).and_then(|row| row["id"].as_str()).unwrap_or("").to_owned();
    let voice = ipc(app, MAIN, "transcribe_chat_audio", json!({"input":{"request_id":"phase6-voice","provider_id":provider,"model_id":"whisper-1","mime":"audio/ogg","base64":"T2dnU3BoYXNlNi1jb250cm9sbGVkLWF1ZGlv"}})).await;
    checks.check("[12] real host transcription validates and sends controlled multipart to loopback", voice.success() && voice.data()["text"] == "PHASE6-TRANSCRIBED", voice.describe());
}

async fn storage_journey(app: &AppHandle, checks: &mut Checks, fx: &Fixture) {
    let assets = dgos_db::AssetRepository::new(fx.db.clone());
    assets.insert(LOCAL_USER, "phase6-cleanup-owned", "discard.txt", "text/plain", b"isolated cleanup", 0).await.unwrap();
    assets.insert("phase6-other-owner", "phase6-cleanup-other", "keep.txt", "text/plain", b"other account", 0).await.unwrap();
    goto(app, MAIN, "/settings/storage").await;
    let cancelled = js(app, r#"
      const row=await until(()=>$$('li').find(el=>txt(el.querySelector('strong'))==='会话附件'));
      await until(()=>{const b=btn('预览',row);return b&&!b.disabled&&b;});
      btn('预览',row).click();
      const confirm=await until(()=>btn('确认清理')); if(!confirm)return false;
      btn('取消').click(); return !!await until(()=>!btn('确认清理'));
    "#).await;
    let kept = assets.bytes(LOCAL_USER, "phase6-cleanup-owned").await.unwrap().is_some();
    checks.check("[12] storage preview cancellation preserves isolated data", cancelled==true && kept, cancelled.to_string());
    let cleaned = js(app, r#"
      const row=$$('li').find(el=>txt(el.querySelector('strong'))==='会话附件');
      btn('预览',row).click(); (await until(()=>btn('确认清理'))).click();
      return !!await until(()=>$$('[role=status]').some(el=>txt(el).includes('1'))&&!btn('确认清理'));
    "#).await;
    let owned_gone = assets.bytes(LOCAL_USER, "phase6-cleanup-owned").await.unwrap().is_none();
    let other_kept = assets.bytes("phase6-other-owner", "phase6-cleanup-other").await.unwrap().is_some();
    let refreshed = ipc(app, MAIN, "get_storage_overview", json!({})).await;
    let count = refreshed.data()["categories"].as_array().and_then(|rows| rows.iter().find(|row| row["category"]=="session_assets")).map(|row| row["count"].clone());
    checks.check("[12] storage confirms real cleanup and refreshes without cross-account deletion", cleaned==true && owned_gone && other_kept && count==Some(json!(0)), format!("cleaned={cleaned} owned_gone={owned_gone} other_kept={other_kept} {}", refreshed.describe()));
    let vacuum_preview=ipc(app,MAIN,"preview_vacuum",json!({})).await;
    let vacuum=ipc(app,MAIN,"execute_vacuum",json!({"input":{"previewId":vacuum_preview.data()["preview_id"],"confirmed":true}})).await;
    checks.check("[12] idle isolated database executes confirmed VACUUM",vacuum_preview.success()&&vacuum.success()&&vacuum.data()["bytes_after"].as_i64().is_some(),vacuum.describe());
    goto(app, MAIN, "/workbench").await;
}
