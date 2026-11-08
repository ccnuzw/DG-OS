//! Native layout-scale evidence, run inside appearance_host's file-backed real-host harness.
use super::{eval, goto, ipc, Checks};
use super::ui_keys::{key, prepare, repeat_key, Key};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

#[cfg(target_os = "macos")]
async fn native_state(app: &AppHandle) -> Value {
    use objc2::{class, msg_send};
    use objc2::runtime::AnyObject;
    use std::ffi::CStr;
    unsafe fn menu_keys(menu: *mut AnyObject, out: &mut Vec<Value>) {
        if menu.is_null() { return; }
        let count: isize = msg_send![menu, numberOfItems];
        for index in 0..count {
            let item: *mut AnyObject = msg_send![menu, itemAtIndex: index];
            let equivalent: *mut AnyObject = msg_send![item, keyEquivalent];
            let bytes: *const std::ffi::c_char = msg_send![equivalent, UTF8String];
            let key = if bytes.is_null() { String::new() } else { CStr::from_ptr(bytes).to_string_lossy().into_owned() };
            let modifiers: usize = msg_send![item, keyEquivalentModifierMask];
            if !key.is_empty() { out.push(json!({"key":key,"modifiers":modifiers})); }
            let submenu: *mut AnyObject = msg_send![item, submenu];
            menu_keys(submenu, out);
        }
    }
    let (send, receive) = tokio::sync::oneshot::channel();
    let Some(window) = app.get_webview_window("main") else { return json!({"error":"missing window"}); };
    if window.with_webview(move |webview| unsafe {
        let view = webview.inner() as *mut AnyObject;
        let zoom: f64 = msg_send![view, pageZoom];
        let ns_app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let menu: *mut AnyObject = msg_send![ns_app, mainMenu];
        let mut keys = Vec::new();
        menu_keys(menu, &mut keys);
        let _ = send.send(json!({"pageZoom":zoom,"menuPresent":!menu.is_null(),"accelerators":keys}));
    }).is_err() { return json!({"error":"native inspection failed"}); }
    receive.await.unwrap_or_else(|_| json!({"error":"native inspection dropped"}))
}

#[cfg(not(target_os = "macos"))]
async fn native_state(_: &AppHandle) -> Value { json!({"error":"macOS NSEvent evidence required"}) }

async fn metrics(app: &AppHandle) -> Value {
    eval(app,"main",r#"
const root=document.documentElement;
const sample=cls=>{const e=document.createElement('span');e.className=cls;e.textContent='Scale text';document.body.append(e);const size=getComputedStyle(e).fontSize;e.remove();return size;};
return {density:root.dataset.density,fonts:['text-xs','type-reading','type-code'].map(sample),dpr:devicePixelRatio,viewport:visualViewport?.scale,input:document.activeElement?.value,focus:document.activeElement?.id};
"#).await["value"].clone()
}

fn unchanged(before: &Value, after: &Value) -> bool {
    before["fonts"] == after["fonts"] && before["dpr"] == after["dpr"] && before["viewport"] == after["viewport"]
        && before["input"] == after["input"] && before["focus"] == after["focus"]
}

async fn snapshot(app: &AppHandle) -> Value {
    ipc(app,"main","get_appearance_preferences",json!({})).await.data()["snapshot"].clone()
}

// Native dispatch and the debounced database write complete on different event loops.
// Observe the committed state instead of assuming a fixed key-helper delay settles it.
async fn saved_step(app: &AppHandle, density: u64, revision: u64) -> Value {
    let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(5);
    loop {
        let stored=snapshot(app).await;
        if (stored["density"]==density&&stored["revision"]==revision)||tokio::time::Instant::now()>=deadline{return stored;}
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

async fn focus_text(app: &AppHandle) -> bool {
    eval(app,"main",r#"const e=document.createElement('input');e.id='native-scale-text';e.value='keep +/- = 中文';(document.querySelector('.appearance-editor')||document.body).append(e);e.focus();return document.activeElement===e;"#).await["value"]==true
}

async fn trace_keys(app: &AppHandle) {
    eval(app,"main",r#"
window.__nativeScaleKeys=[];
document.addEventListener('keydown',event=>{
  if(event.metaKey&&['-','=','+'].includes(event.key))setTimeout(()=>window.__nativeScaleKeys.push({key:event.key,repeat:event.repeat,trusted:event.isTrusted,prevented:event.defaultPrevented}),0);
},true);
return true;
"#).await;
}

async fn trace(app: &AppHandle) -> Value {
    eval(app,"main","return window.__nativeScaleKeys || []; ").await["value"].clone()
}

fn consumed_native_events(events: &Value) -> bool {
    events.as_array().is_some_and(|events| !events.is_empty() && events.iter().all(|event|event["trusted"]==true&&event["prevented"]==true))
}

/// JavaScript establishes focus and reads evidence; every tested key traverses NSApplication as an NSEvent.
pub(super) async fn run(app: &AppHandle, checks: &mut Checks) -> Value {
    checks.check("macOS native scale window takes foreground focus",prepare(app).await,"key WKWebView required");
    let native=native_state(app).await;
    let no_collision=native["accelerators"].as_array().is_some_and(|items|items.iter().all(|item| {
        let command=item["modifiers"].as_u64().unwrap_or(0)&(1<<20)!=0;
        !command || !matches!(item["key"].as_str(),Some("-"|"="|"+"))
    }));
    checks.check("actual AppKit menu has no Command -/= /+ accelerator stealing layout shortcuts",no_collision&&native["pageZoom"].is_number(),native.to_string());
    let loaded=goto(app,"main","/settings/appearance").await;
    super::appearance_host::page(app).await;
    let slider=eval(app,"main",r#"
for(let i=0;i<100&&!document.querySelector('.appearance-layout-scale [role=slider]');i++)await new Promise(r=>setTimeout(r,50));
const slider=document.querySelector('.appearance-layout-scale [role=slider]');slider?.focus();
return {found:!!slider,min:slider?.getAttribute('aria-valuemin'),max:slider?.getAttribute('aria-valuemax'),value:slider?.getAttribute('aria-valuenow')};
"#).await;
    checks.check("shipped layout Slider exposes 60..150 and saved 150",loaded&&slider["value"]["min"]=="60"&&slider["value"]["max"]=="150"&&slider["value"]["value"]=="150",slider.to_string());
    let confirmed=snapshot(app).await;
    key(app,Key::Home).await;
    checks.check("native Slider Home previews 60 without persistence",metrics(app).await["density"]=="60"&&snapshot(app).await==confirmed,"Slider keyboard draft");
    key(app,Key::Right).await;
    checks.check("native Slider Right uses ten-point steps",metrics(app).await["density"]=="70","Home then ArrowRight");
    key(app,Key::End).await;
    let focused=focus_text(app).await;
    let before=metrics(app).await;
    trace_keys(app).await;
    for (press,want,label) in [(Key::CommandMinus,"140","Command+-"),(Key::CommandEqual,"150","Command+="),(Key::CommandMinus,"140","Command+-"),(Key::CommandPlus,"150","Command+Shift+=")] {
        key(app,press).await;
        let after=metrics(app).await;
        checks.check(&format!("settings {label} previews {want} without saving, typing or zooming"),focused&&after["density"]==want&&unchanged(&before,&after)&&snapshot(app).await==confirmed&&native_state(app).await["pageZoom"]==native["pageZoom"],after.to_string());
    }
    key(app,Key::CommandMinus).await;
    repeat_key(app,Key::CommandMinus).await;
    checks.check("native repeated Command+- is consumed without a second adjustment",metrics(app).await["density"]=="140"&&native_state(app).await["pageZoom"]==native["pageZoom"],"NSEvent isARepeat=true");
    let events=trace(app).await;
    checks.check("settings shortcut evidence is trusted native keydown, including one consumed repeat",consumed_native_events(&events)&&events.as_array().is_some_and(|events|events.iter().filter(|event|event["repeat"]==true).count()==1),events.to_string());
    let cancel=eval(app,"main",r#"const b=document.querySelector('.appearance-save-actions button');b?.focus();return !!b;"#).await;
    key(app,Key::Enter).await;
    checks.check("native cancel discards shortcut draft without writing",cancel["value"]==true&&metrics(app).await["density"]=="150"&&snapshot(app).await==confirmed,"cancel preview");
    goto(app,"main","/").await;
    super::appearance_host::page(app).await;
    let focused=focus_text(app).await;
    let before=metrics(app).await;
    trace_keys(app).await;
    let mut current=confirmed;
    // Verify each independent accepted step commits exactly one CAS before the next step.
    for (press,want) in [(Key::CommandMinus,140),(Key::CommandEqual,150),(Key::CommandMinus,140),(Key::CommandPlus,150)] {
        key(app,press).await;
        let stored=saved_step(app,want,current["revision"].as_u64().unwrap()+1).await;
        let mut expected=current.clone();expected["density"]=json!(want);expected["revision"]=json!(current["revision"].as_u64().unwrap()+1);
        let after=metrics(app).await;
        checks.check(&format!("outside settings native shortcut CAS saves {want}, preserving other fields, focused text, fonts and zoom"),focused&&stored==expected&&after["density"]==want.to_string()&&unchanged(&before,&after)&&native_state(app).await["pageZoom"]==native["pageZoom"],stored.to_string());
        current=stored;
    }
    repeat_key(app,Key::CommandMinus).await;
    key(app,Key::CommandPlus).await;
    checks.check("native repeat and upper boundary consume zoom without writes",snapshot(app).await==current&&metrics(app).await["density"]=="150"&&native_state(app).await["pageZoom"]==native["pageZoom"],"repeat and 150 boundary");
    let mut lower=current.clone();
    for step in (6..=14).rev() {
        let want=step*10;
        key(app,Key::CommandMinus).await;
        let revision=lower["revision"].as_u64().unwrap()+1;
        lower=saved_step(app,want,revision).await;
        checks.check(&format!("native independent decrement commits {want} once"),lower["density"]==want&&lower["revision"]==revision,lower.to_string());
    }
    key(app,Key::CommandMinus).await;
    checks.check("native 60 lower boundary prevents browser zoom and performs no write",lower["density"]==60&&snapshot(app).await==lower&&unchanged(&before,&metrics(app).await)&&native_state(app).await["pageZoom"]==native["pageZoom"],lower.to_string());
    let mut final_snapshot=lower;
    for want in (70..=150).step_by(10) {
        key(app,Key::CommandPlus).await;
        let revision=final_snapshot["revision"].as_u64().unwrap()+1;
        final_snapshot=saved_step(app,want,revision).await;
        checks.check(&format!("native independent increment commits {want} once"),final_snapshot["density"]==want&&final_snapshot["revision"]==revision,final_snapshot.to_string());
    }
    checks.check("native shortcuts leave complete schema6/150 snapshot for process restart",final_snapshot["schemaVersion"]==6&&final_snapshot["density"]==150&&final_snapshot["typography"]==current["typography"]&&final_snapshot["variants"]==current["variants"],final_snapshot.to_string());
    let events=trace(app).await;
    checks.check("outside settings all native shortcuts, repeat and both bounds prevent browser defaults",consumed_native_events(&events)&&events.as_array().is_some_and(|events|events.iter().filter(|event|event["repeat"]==true).count()==1),events.to_string());
    final_snapshot
}
