//! FR-005 Phase 8: physical-keyboard and overlay scenarios in the visible WKWebView.
//!
//! Keys are real `NSEvent`s (keyDown + keyUp) delivered through `-[NSApplication sendEvent:]` to the key
//! window's responder, so WebKit runs its own default actions: Tab traversal, `:focus-visible`, Space/Enter activation of
//! buttons. Nothing here dispatches DOM `KeyboardEvent`s or calls `.click()`/`.focus()` as a substitute for a
//! key press, except where a step says so (starting a scenario from a known control).
//!
//! The page set is the shipped one plus `tests/fixtures/modules/p8-gallery.js`, a fixture module rendering
//! Tabs, RadioGroup, Select, Switch, Checkbox, Tooltip, a Modal with a Select and a Tooltip inside, and a Toast
//! trigger through the frozen `host.ui` (no shipped page nests those).
//!
//! Not proven here: pointer clicks (hit-testing uses `elementFromPoint`, not a mouse event), VoiceOver, any
//! other platform. The focused modal geometry slice uses the production reduced-motion preference.

use super::ui::{set_appearance, UiProbe, MAIN};
use super::{eval, goto, Checks};
use dgos_db::sea_orm::ConnectionTrait;
use serde_json::{json, Value};
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tokio::time::sleep;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Key {
    Tab,
    ShiftTab,
    Space,
    Enter,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    CommandMinus,
    CommandEqual,
    CommandPlus,
}

impl Key {
    /// (macOS virtual key code, characters, modifier flags)
    fn spec(self) -> (u16, &'static str, usize) {
        const SHIFT: usize = 1 << 17;
        const COMMAND: usize = 1 << 20;
        const NUMPAD: usize = 1 << 21;
        const FN: usize = 1 << 23;
        match self {
            Key::Tab => (48, "\t", 0),
            Key::ShiftTab => (48, "\u{19}", SHIFT),
            Key::Space => (49, " ", 0),
            Key::Enter => (36, "\r", 0),
            Key::Escape => (53, "\u{1b}", 0),
            Key::Left => (123, "\u{f702}", FN | NUMPAD),
            Key::Right => (124, "\u{f703}", FN | NUMPAD),
            Key::Down => (125, "\u{f701}", FN | NUMPAD),
            Key::Up => (126, "\u{f700}", FN | NUMPAD),
            Key::Home => (115, "\u{f729}", FN),
            Key::End => (119, "\u{f72b}", FN),
            Key::CommandMinus => (27, "-", COMMAND),
            Key::CommandEqual => (24, "=", COMMAND),
            Key::CommandPlus => (24, "+", COMMAND | SHIFT),
        }
    }
}

#[cfg(target_os = "macos")]
fn post_with_repeat(app: &AppHandle, key: Key, down: bool, repeat: bool) -> bool {
    use objc2::encode::{Encode, Encoding};
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{class, msg_send};
    use std::ffi::CString;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: f64,
        y: f64,
    }
    unsafe impl Encode for Point {
        const ENCODING: Encoding =
            Encoding::Struct("CGPoint", &[Encoding::Double, Encoding::Double]);
    }

    let Some(window) = app.get_webview_window(MAIN) else {
        return false;
    };
    let (code, chars, flags) = key.spec();
    window
        .with_webview(move |webview| unsafe {
            let win = webview.ns_window() as *mut AnyObject;
            let chars = CString::new(chars).expect("no NUL in key characters");
            let text: *mut AnyObject =
                msg_send![class!(NSString), stringWithUTF8String: chars.as_ptr()];
            let unmodified = CString::new(if key == Key::CommandPlus { "=" } else { chars.to_str().unwrap() }).unwrap();
            let ignoring: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: unmodified.as_ptr()];
            let info: *mut AnyObject = msg_send![class!(NSProcessInfo), processInfo];
            let timestamp: f64 = msg_send![info, systemUptime];
            let number: isize = msg_send![win, windowNumber];
            let kind: usize = if down { 10 } else { 11 };
            let event: *mut AnyObject = msg_send![
                class!(NSEvent),
                keyEventWithType: kind,
                location: Point { x: 0.0, y: 0.0 },
                modifierFlags: flags,
                timestamp: timestamp,
                windowNumber: number,
                context: std::ptr::null_mut::<AnyObject>(),
                characters: text,
                charactersIgnoringModifiers: ignoring,
                isARepeat: Bool::new(repeat),
                keyCode: code
            ];
            // Route through NSApplication so currentEvent and WebKit's keyboard-input modality agree.
            let ns_app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![ns_app, sendEvent: event];
        })
        .is_ok()
}

#[cfg(not(target_os = "macos"))]
fn post_with_repeat(_: &AppHandle, _: Key, _: bool, _: bool) -> bool {
    false
}

fn post(app: &AppHandle, key: Key, down: bool) -> bool {
    post_with_repeat(app, key, down, false)
}

/// Activate the application and make the window key (the app may be launched in the background).
#[cfg(target_os = "macos")]
fn activate(app: &AppHandle) -> bool {
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{class, msg_send};
    let Some(window) = app.get_webview_window(MAIN) else {
        return false;
    };
    let _ = window.show();
    window
        .with_webview(|webview| unsafe {
            let ns_app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let _: Bool = msg_send![ns_app, setActivationPolicy: 0isize];
            // A Cargo test executable has no app-bundle activation handshake. Request foreground
            // ownership through the running process as well as NSApplication before making its window key.
            let process: *mut AnyObject = msg_send![class!(NSRunningApplication), currentApplication];
            let _: Bool = msg_send![process, activateWithOptions: 3usize];
            let _: () = msg_send![ns_app, activateIgnoringOtherApps: Bool::YES];
            let win = webview.ns_window() as *mut AnyObject;
            let _: () = msg_send![win, orderFrontRegardless];
            let _: () = msg_send![win, makeMainWindow];
            let _: () = msg_send![win, makeKeyAndOrderFront: std::ptr::null_mut::<AnyObject>()];
            let view = webview.inner() as *mut AnyObject;
            let responder: *mut AnyObject = msg_send![win, firstResponder];
            let in_view = if responder.is_null() { false } else {
                let is_view: Bool = msg_send![responder, isKindOfClass: class!(NSView)];
                is_view.as_bool() && {
                    let descendant: Bool = msg_send![responder, isDescendantOf: view];
                    descendant.as_bool()
                }
            };
            // WKContentView owns editing and key modality; do not replace it on every key press.
            if !in_view {
                let _: Bool = msg_send![win, makeFirstResponder: view];
            }
        })
        .is_ok()
}

#[cfg(not(target_os = "macos"))]
fn activate(_: &AppHandle) -> bool {
    false
}

/// Make the window key and the WKWebView its first responder, so events reach WebKit. Retries until the
/// page itself reports `document.hasFocus()`; if it never does, that is printed and the checks that depend
/// on focus fail with `hasFocus:false` in their detail.
#[cfg(target_os = "macos")]
pub(super) async fn prepare(app: &AppHandle) -> bool {
    use objc2::msg_send;
    use objc2::runtime::{AnyObject, Bool};
    let Some(window) = app.get_webview_window(MAIN) else {
        return false;
    };
    let mut focused = false;
    for _ in 0..12 {
        activate(app);
        sleep(Duration::from_millis(300)).await;
        if truthy(app, "document.hasFocus()").await {
            focused = true;
            break;
        }
    }
    if !focused {
        let (send, receive) = tokio::sync::oneshot::channel();
        window.with_webview(move |webview| unsafe {
            use objc2::class;
            let ns_app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let win = webview.ns_window() as *mut AnyObject;
            let active: Bool = msg_send![ns_app, isActive];
            let policy: isize = msg_send![ns_app, activationPolicy];
            let key: Bool = msg_send![win, isKeyWindow];
            let main: Bool = msg_send![win, isMainWindow];
            let visible: Bool = msg_send![win, isVisible];
            let responder: *mut AnyObject = msg_send![win, firstResponder];
            let process: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
            let front: *mut AnyObject = msg_send![process, frontmostApplication];
            let front_pid: i32 = if front.is_null() { 0 } else { msg_send![front, processIdentifier] };
            let class_name = if responder.is_null() { "none".to_owned() } else { (&*responder).class().name().to_string_lossy().into_owned() };
            let _ = send.send(json!({"appActive":active.as_bool(),"activationPolicy":policy,"keyWindow":key.as_bool(),"mainWindow":main.as_bool(),"visible":visible.as_bool(),"firstResponderClass":class_name,"frontmostPid":front_pid,"processPid":std::process::id()}));
        }).ok();
        println!("INFO native foreground preflight failed: {:?}", receive.await);
    }
    focused
}

#[cfg(not(target_os = "macos"))]
pub(super) async fn prepare(_: &AppHandle) -> bool {
    false
}

pub(super) async fn key(app: &AppHandle, key: Key) {
    // Re-asserts key status every time: an occluded or non-key window silently drops key events and
    // freezes animations.
    activate(app);
    post(app, key, true);
    sleep(Duration::from_millis(20)).await;
    post(app, key, false);
    sleep(Duration::from_millis(220)).await;
}

/// A held-key repeat uses NSEvent's repeat flag; DOM event construction cannot exercise this path.
pub(super) async fn repeat_key(app: &AppHandle, key: Key) {
    activate(app);
    post_with_repeat(app, key, true, true);
    sleep(Duration::from_millis(20)).await;
    post(app, key, false);
    sleep(Duration::from_millis(220)).await;
}

const PRE: &str = r#"
const root = document.documentElement;
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 5000) => { const t = Date.now(); while (Date.now() - t < ms) { try { const v = fn(); if (v) return v; } catch (e) {} await wait(50); } return false; };
const css = (el, p) => getComputedStyle(el)[p];
const parse = (text) => { const m = /rgba?\(([^)]+)\)/.exec(text) || /color\(srgb ([^)]+)\)/.exec(text); if (!m) return null; const parts = m[1].split(/[ ,/]+/).filter(Boolean).map(Number); const c = text.startsWith('color(') ? parts.slice(0, 3).map((v) => Math.round(v * 255)) : parts.slice(0, 3); return { c, a: parts.length > 3 ? parts[3] : 1 }; };
const lum = ([r, g, b]) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); }; return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b); };
const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)]; return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
const bgOf = (el) => { let n = el; while (n) { const p = parse(css(n, 'backgroundColor')); if (p && p.a > 0.99) return p.c; n = n.parentElement; } return parse(css(document.body, 'backgroundColor')).c; };
const tokenColor = (v) => { const e = document.createElement('div'); e.style.color = 'var(' + v + ')'; document.body.appendChild(e); const c = parse(css(e, 'color')).c; e.remove(); return c; };
const lbl = (el) => !el ? null : (el.getAttribute('aria-label') || (el.labels && el.labels[0] && el.labels[0].textContent.trim()) || (el.innerText || el.textContent || '').trim().slice(0, 40) || el.id || el.tagName.toLowerCase());
const tabbables = () => [...document.querySelectorAll('a[href],button,input,select,textarea,[tabindex]')].filter((e) => !e.disabled && e.tabIndex >= 0 && e.getClientRects().length && css(e, 'visibility') !== 'hidden' && !e.closest('[inert],[aria-hidden="true"]'));
const focusInfo = () => {
  const el = document.activeElement;
  if (!el || el === document.body) return { body: true, hasFocus: document.hasFocus() };
  const cs = getComputedStyle(el); const bg = bgOf(el);
  const outlined = cs.outlineStyle !== 'none' && parseFloat(cs.outlineWidth) > 0;
  const bordered = ['input', 'textarea', 'select'].includes(el.tagName.toLowerCase()) || el.getAttribute('role') === 'combobox';
  const ink = parse(outlined ? cs.outlineColor : bordered ? cs.borderTopColor : '');
  const visible = outlined || bordered && cs.borderTopStyle !== 'none' && parseFloat(cs.borderTopWidth) > 0;
  return { body: false, hasFocus: document.hasFocus(), tag: el.tagName.toLowerCase(), role: el.getAttribute('role'), label: lbl(el), id: el.id,
    index: tabbables().indexOf(el), fv: el.matches(':focus-visible'), indicatorVisible: visible,
    contrast: ink ? Math.round(ratio(ink.c, outlined && parseFloat(cs.outlineOffset) >= 0 ? bgOf(el.parentElement || el) : bg) * 100) / 100 : null };
};
const geo = (el) => { const r = el.getBoundingClientRect(); return { l: Math.round(r.left), t: Math.round(r.top), r: Math.round(r.right), b: Math.round(r.bottom), w: Math.round(r.width), h: Math.round(r.height), vw: root.clientWidth, vh: innerHeight, inside: r.left >= -0.5 && r.top >= -0.5 && r.right <= root.clientWidth + 0.5 && r.bottom <= innerHeight + 0.5 }; };
const zOf = (el) => { let z = 0; for (let n = el; n && n !== document.documentElement; n = n.parentElement) { const v = parseInt(css(n, 'zIndex'), 10); if (!Number.isNaN(v)) z = Math.max(z, v); } return z; };
// The focused element's ring box (border box grown by outline width + offset) against the padding box of every
// ancestor that clips (any overflow other than visible). Returns the clipping ancestors and by how much they cut.
const ringCuts = () => { const el = document.activeElement; const cs = getComputedStyle(el); const g = cs.outlineStyle === 'none' ? 0 : Math.max(0, parseFloat(cs.outlineWidth) + parseFloat(cs.outlineOffset)); const r = el.getBoundingClientRect(); const ring = { l: r.left - g, t: r.top - g, r: r.right + g, b: r.bottom + g }; const cuts = []; for (let n = el.parentElement; n; n = n.parentElement) { const s = getComputedStyle(n); if (s.overflowX === 'visible' && s.overflowY === 'visible') continue; const b = n.getBoundingClientRect(); const over = { l: b.left + parseFloat(s.borderLeftWidth) - ring.l, t: b.top + parseFloat(s.borderTopWidth) - ring.t, r: ring.r - (b.right - parseFloat(s.borderRightWidth)), b: ring.b - (b.bottom - parseFloat(s.borderBottomWidth)) }; const bad = Object.fromEntries(Object.entries(over).filter(([, v]) => v > 0.5).map(([k, v]) => [k, Math.round(v * 10) / 10])); if (Object.keys(bad).length) cuts.push({ cls: String(n.className).slice(0, 48), ...bad }); } return cuts; };
"#;

async fn js(app: &AppHandle, body: &str) -> Value {
    let reply = eval(app, MAIN, &format!("{PRE}{body}")).await;
    if reply["ok"] == true {
        reply["value"].clone()
    } else {
        json!({"__error": reply["error"].clone()})
    }
}

async fn truthy(app: &AppHandle, expr: &str) -> bool {
    js(app, &format!("return !!({expr});")).await == true
}

/// Press Tab (or Shift+Tab) until `pred` (a JS expression) holds for the focused element; false after `max`.
async fn tab_until(app: &AppHandle, pred: &str, max: usize, shift: bool) -> bool {
    for _ in 0..max {
        key(app, if shift { Key::ShiftTab } else { Key::Tab }).await;
        if truthy(app, pred).await {
            return true;
        }
    }
    false
}

async fn ready(app: &AppHandle, path: &str) -> bool {
    if !goto(app, MAIN, path).await {
        return false;
    }
    if !prepare(app).await {
        return false;
    }
    let state = js(
        app,
        "for (let i = 0; i < 100 && root.hasAttribute('data-appearance-pending'); i++) await wait(50); await wait(500); return true;",
    )
    .await;
    state == true
}

fn ring_problem(info: &Value) -> Option<String> {
    if info["body"] == true {
        return Some("focus is on <body>".into());
    }
    let ok = info["fv"] == true
        && info["indicatorVisible"] == true
        && info["contrast"].as_f64().unwrap_or(0.0) >= 3.0;
    if ok {
        None
    } else {
        Some(info.to_string())
    }
}

async fn home(app: &AppHandle, checks: &mut Checks) {
    if !ready(app, "/").await {
        checks.check("Home: loaded", false, "page never loaded");
        return;
    }
    let n = js(app, "return tabbables().length;")
        .await
        .as_u64()
        .unwrap_or(0) as usize;
    checks.check(
        "Home: has tab stops",
        n >= 9,
        format!("{n} tabbable elements"),
    );
    let (mut order, mut labels, mut bad) = (vec![], vec![], vec![]);
    for _ in 0..n {
        key(app, Key::Tab).await;
        let info = js(app, "return focusInfo();").await;
        order.push(info["index"].clone());
        labels.push(info["label"].as_str().unwrap_or("?").to_owned());
        if let Some(p) = ring_problem(&info) {
            bad.push(format!("{}: {p}", info["label"]));
        }
    }
    println!("INFO Home tab order: {}", labels.join(" > "));
    let want: Vec<Value> = (0..n).map(|i| json!(i)).collect();
    checks.check(
        "Home: Tab visits every tab stop once, in DOM order",
        order == want,
        format!("{order:?}"),
    );
    checks.check(
        "Home: each keyboard-focused control has a visible contrasting position indicator",
        bad.is_empty(),
        bad.join(" | "),
    );
    let mut back = vec![];
    for _ in 1..n {
        key(app, Key::ShiftTab).await;
        back.push(js(app, "return focusInfo();").await["index"].clone());
    }
    let want_back: Vec<Value> = (0..n.saturating_sub(1)).rev().map(|i| json!(i)).collect();
    checks.check(
        "Home: Shift+Tab walks back in exact reverse order",
        back == want_back,
        format!("{back:?}"),
    );

    // The desktop organizer stays on the page and responds to physical Space/Enter.
    let found = tab_until(app, "document.activeElement.matches('.desktop-header summary')", n + 2, false).await;
    checks.check("Desktop: Tab reaches organizer", found, "not reached");
    if found {
        key(app, Key::Space).await;
        let opened = js(app, "return document.querySelector('.desktop-header details').open;").await;
        key(app, Key::Enter).await;
        let closed = js(app, "return !document.querySelector('.desktop-header details').open;").await;
        checks.check("Desktop: Space opens organizer and Enter closes it", opened == true && closed == true, format!("opened={opened}, closed={closed}"));
    }
}

async fn skills_tabs(app: &AppHandle, checks: &mut Checks) {
    if !ready(app, "/skills").await {
        checks.check("Skills: loaded", false, "page never loaded");
        return;
    }
    let found = tab_until(
        app,
        "document.activeElement.getAttribute('role') === 'tab' && document.activeElement.closest('[role=tablist]').getAttribute('aria-label') === 'Skills 视图'",
        30,
        false,
    )
    .await;
    checks.check(
        "Skills: Tab reaches the tab list",
        found,
        "no role=tab reached",
    );
    if !found {
        return;
    }
    let state = "const t = document.activeElement; return { role: t.getAttribute('role'), sel: t.getAttribute('aria-selected'), label: lbl(t), roving: [...t.closest('[role=tablist]').querySelectorAll('[role=tab]')].filter((x) => x.tabIndex === 0).length };";
    let s = js(app, state).await;
    checks.check("Skills: Tab lands on the selected tab; exactly one tab is in the tab order (roving tabindex)", s["sel"] == "true" && s["roving"] == 1, s.to_string());
    for (k, name, want) in [
        (Key::Right, "Right", "已安装"),
        (Key::Left, "Left", "浏览 Skills"),
        (Key::End, "End", "已安装"),
        (Key::Home, "Home", "浏览 Skills"),
    ] {
        key(app, k).await;
        let s = js(app, state).await;
        checks.check(
            &format!("Skills Tabs: {name} moves focus and selection to '{want}'"),
            s["role"] == "tab"
                && s["sel"] == "true"
                && s["label"].as_str().is_some_and(|l| l.starts_with(want)),
            s.to_string(),
        );
    }
}

async fn open_gallery(app: &AppHandle) -> bool {
    if !goto(app, MAIN, "/settings/modules/p8-gallery").await {
        return false;
    }
    let loaded = js(
        app,
        "return !!(await until(() => document.querySelector('#g-open'), 20000));",
    )
    .await
        == true;
    // A navigation can leave the window non-key (another app took focus); keys only reach WebKit in a key window.
    loaded && prepare(app).await
}

async fn gallery_controls(app: &AppHandle, checks: &mut Checks) {
    // Tabs
    let t = "const t = document.activeElement; return { role: t.getAttribute('role'), sel: t.getAttribute('aria-selected'), label: lbl(t) };";
    // D085: the Settings shell's own tablist comes first on this page; the gallery's Tabs are inside the module.
    if tab_until(
        app,
        "document.activeElement.getAttribute('role') === 'tab' && !!document.activeElement.closest('[data-module-id=\"p8-gallery\"]')",
        40,
        false,
    )
    .await
    {
        let ring = js(app, "return focusInfo();").await;
        checks.check(
            "Gallery Tabs: focus ring is visible",
            ring_problem(&ring).is_none(),
            ring.to_string(),
        );
        for (k, name, want) in [
            (Key::Right, "Right", "Two"),
            (Key::Left, "Left", "One"),
            (Key::End, "End", "Three"),
            (Key::Home, "Home", "One"),
        ] {
            key(app, k).await;
            let s = js(app, t).await;
            checks.check(
                &format!("Gallery Tabs: {name} -> '{want}'"),
                s["sel"] == "true" && s["label"] == want,
                s.to_string(),
            );
        }
    } else {
        checks.check("Gallery Tabs: reached by Tab", false, "no role=tab");
    }
    // RadioGroup
    let r = "const t = document.activeElement; return { role: t.getAttribute('role'), checked: t.getAttribute('aria-checked'), label: lbl(t), roving: [...document.querySelectorAll('[role=radio]')].filter((x) => x.tabIndex === 0).length };";
    if tab_until(
        app,
        "document.activeElement.getAttribute('role') === 'radio'",
        12,
        false,
    )
    .await
    {
        let s = js(app, r).await;
        let ring = js(app, "return focusInfo();").await;
        checks.check(
            "Gallery RadioGroup: focus ring is visible",
            ring_problem(&ring).is_none(),
            ring.to_string(),
        );
        checks.check(
            "Gallery RadioGroup: Tab lands on the checked radio; one radio in the tab order",
            s["checked"] == "true" && s["label"] == "Beta" && s["roving"] == 1,
            s.to_string(),
        );
        for (k, name, want) in [(Key::Down, "Down", "Gamma"), (Key::Up, "Up", "Beta")] {
            key(app, k).await;
            let s = js(app, r).await;
            checks.check(
                &format!("Gallery RadioGroup: {name} moves focus and checks '{want}'"),
                s["checked"] == "true" && s["label"] == want,
                s.to_string(),
            );
        }
    } else {
        checks.check("Gallery RadioGroup: reached by Tab", false, "no role=radio");
    }
    // Select (page level)
    let listbox = "const l = document.querySelector('[role=listbox]'); if (!l) return null; const o = l.querySelector('[role=option]'); const r = o.getBoundingClientRect(); const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2); return { g: geo(l), z: zOf(l), hit: l.contains(hit), options: l.querySelectorAll('[role=option]').length };";
    if tab_until(
        app,
        "document.activeElement.getAttribute('role') === 'combobox'",
        12,
        false,
    )
    .await
    {
        let before = js(app, "return focusInfo();").await;
        checks.check(
            "Gallery Select: trigger shows the focus ring",
            ring_problem(&before).is_none(),
            before.to_string(),
        );
        key(app, Key::Enter).await;
        sleep(Duration::from_millis(300)).await;
        let l = js(app, listbox).await;
        println!("INFO Gallery Select open (Enter): {l}");
        checks.check(
            "Gallery Select: Enter opens a listbox that is inside the viewport and hit-testable",
            l["g"]["inside"] == true && l["hit"] == true && l["options"] == 3,
            l.to_string(),
        );
        key(app, Key::Down).await;
        key(app, Key::Enter).await;
        sleep(Duration::from_millis(300)).await;
        let s = js(app, "const c = document.activeElement; return { role: c.getAttribute('role'), text: (c.innerText || '').trim(), open: !!document.querySelector('[role=listbox]') };").await;
        checks.check("Gallery Select: Down+Enter picks the next option, closes the list and returns focus to the trigger", s["role"] == "combobox" && s["open"] == false && s["text"].as_str().is_some_and(|t| t.contains("Banana") || t.contains("Apple")), s.to_string());
        key(app, Key::Space).await;
        sleep(Duration::from_millis(300)).await;
        let opened = truthy(app, "document.querySelector('[role=listbox]')").await;
        key(app, Key::Escape).await;
        sleep(Duration::from_millis(300)).await;
        let s = js(app, "return { role: document.activeElement.getAttribute('role'), open: !!document.querySelector('[role=listbox]') };").await;
        checks.check(
            "Gallery Select: Space opens, Escape closes and restores focus to the trigger",
            opened && s["open"] == false && s["role"] == "combobox",
            format!("opened={opened} {s}"),
        );
    } else {
        checks.check("Gallery Select: reached by Tab", false, "no role=combobox");
    }
    // Switch and Checkbox
    for (role, name) in [("switch", "Switch"), ("checkbox", "Checkbox")] {
        let pred = format!("document.activeElement.getAttribute('role') === '{role}'");
        if tab_until(app, &pred, 6, false).await {
            let ring = js(app, "return focusInfo();").await;
            checks.check(
                &format!("Gallery {name}: focus ring is visible"),
                ring_problem(&ring).is_none(),
                ring.to_string(),
            );
            let read = "return document.activeElement.getAttribute('aria-checked');";
            let b = js(app, read).await;
            key(app, Key::Space).await;
            let a = js(app, read).await;
            key(app, Key::Enter).await;
            let e = js(app, read).await;
            println!(
                "INFO Gallery {name}: aria-checked before={b} after-Space={a} after-Enter={e}"
            );
            checks.check(
                &format!("Gallery {name}: Space toggles it"),
                b == "false" && a == "true",
                format!("{b} -> {a}"),
            );
        } else {
            checks.check(
                &format!("Gallery {name}: reached by Tab"),
                false,
                "not reached",
            );
        }
    }
    // Tooltip on the page: opens on keyboard focus, Escape dismisses.
    if tab_until(app, "document.activeElement.id === 'g-tip'", 6, false).await {
        let ring = js(app, "return focusInfo();").await;
        checks.check(
            "Gallery Tooltip trigger: focus ring is visible",
            ring_problem(&ring).is_none(),
            ring.to_string(),
        );
        sleep(Duration::from_millis(700)).await;
        let open = js(app, "const tip = document.querySelector('[role=tooltip]'); return { shown: !!tip, text: tip && tip.textContent.trim(), described: !!document.activeElement.getAttribute('aria-describedby'), g: tip && geo(tip) };").await;
        checks.check("Gallery Tooltip: keyboard focus opens it, trigger gets aria-describedby, it is inside the viewport", open["shown"] == true && open["described"] == true && open["g"]["inside"] == true, open.to_string());
        key(app, Key::Escape).await;
        sleep(Duration::from_millis(300)).await;
        checks.check(
            "Gallery Tooltip: Escape dismisses it",
            !truthy(app, "document.querySelector('[role=tooltip]')").await,
            "still open",
        );
    } else {
        checks.check(
            "Gallery Tooltip: reached by Tab",
            false,
            "g-tip not reached",
        );
    }
}

async fn gallery_dialog(app: &AppHandle, checks: &mut Checks) {
    if !tab_until(app, "document.activeElement.id === 'g-open'", 6, false).await {
        checks.check(
            "Gallery Dialog: opener reached by Tab",
            false,
            "g-open not reached",
        );
        return;
    }
    key(app, Key::Enter).await;
    let opened = js(
        app,
        "return !!(await until(() => document.querySelector('[role=dialog]'), 5000));",
    )
    .await
        == true;
    checks.check(
        "Gallery Dialog: Enter opens a role=dialog",
        opened,
        "no dialog",
    );
    if !opened {
        return;
    }
    sleep(Duration::from_millis(500)).await;
    let d = js(app, "const d = document.querySelector('[role=dialog]'); const title = document.getElementById(d.getAttribute('aria-labelledby')); const desc = document.getElementById(d.getAttribute('aria-describedby')); const ov = document.querySelector('[data-state=open][class*=bg-overlay], [class*=bg-overlay]'); const a = document.activeElement; const okBtn = [...d.querySelectorAll('button')].find((b) => lbl(b) === 'OK'); return { title: title && title.textContent, desc: !!desc, g: geo(d), scrollOverflow: d.scrollWidth > d.clientWidth, z: zOf(d), overlayBg: ov && css(ov, 'backgroundColor'), overlayOpacity: ov && css(ov, 'opacity'), focusInside: d.contains(a), focus: lbl(a), focusIsClose: lbl(a) === '关闭', focusRing: ring(), rootHidden: document.getElementById('root').closest('[aria-hidden=true]') !== null || document.getElementById('root').getAttribute('aria-hidden'), bodyPtr: css(document.body, 'pointerEvents'), longWraps: (() => { const p = document.getElementById('g-dialog-long'); return p.scrollWidth <= p.clientWidth + 1; })(), okBtn: !!okBtn }; function ring() { return focusInfo(); }").await;
    println!("INFO Gallery Dialog open: {d}");
    checks.check(
        "Gallery Dialog: named by its title and described",
        d["title"] == "Gallery dialog" && d["desc"] == true,
        d.to_string(),
    );
    checks.check(
        "Gallery Dialog: inside the viewport, no horizontal scroll, long text wraps",
        d["g"]["inside"] == true && d["scrollOverflow"] == false && d["longWraps"] == true,
        d.to_string(),
    );
    checks.check("Gallery Dialog: focus moved inside onto an action button, not the close button, with a visible ring", d["focusInside"] == true && d["focusIsClose"] == false && ring_problem(&d["focusRing"]).is_none(), d["focusRing"].to_string());
    checks.check(
        "Gallery Dialog: the page behind is hidden from assistive technology (aria-hidden)",
        d["rootHidden"] != Value::Bool(false) && !d["rootHidden"].is_null(),
        d["rootHidden"].to_string(),
    );
    // Focus trap: 10 Tab and 10 Shift+Tab presses never leave the dialog, and focus wraps.
    let mut outside = vec![];
    let mut seen = vec![];
    let mut ring_bad = vec![];
    for _ in 0..10 {
        key(app, Key::Tab).await;
        let s = js(app, "const d = document.querySelector('[role=dialog]'); return { inside: !!d && d.contains(document.activeElement), label: lbl(document.activeElement) };").await;
        if s["inside"] != true {
            outside.push(s["label"].to_string());
        }
        seen.push(s["label"].as_str().unwrap_or("?").to_owned());
        let ring = js(app, "return focusInfo();").await;
        if let Some(p) = ring_problem(&ring) {
            ring_bad.push(format!("{}: {p}", s["label"]));
        }
    }
    for _ in 0..10 {
        key(app, Key::ShiftTab).await;
        let s = js(app, "const d = document.querySelector('[role=dialog]'); return { inside: !!d && d.contains(document.activeElement), label: lbl(document.activeElement) };").await;
        if s["inside"] != true {
            outside.push(s["label"].to_string());
        }
    }
    println!("INFO Gallery Dialog Tab cycle: {}", seen.join(" > "));
    checks.check(
        "Gallery Dialog: every Tab stop (including the close button) has a visible focus ring",
        ring_bad.is_empty(),
        ring_bad.join(" | "),
    );
    let wraps = seen.iter().enumerate().any(|(i, l)| seen[..i].contains(l));
    checks.check(
        "Gallery Dialog: Tab/Shift+Tab never leave the dialog and focus wraps",
        outside.is_empty() && wraps,
        format!("outside={outside:?} wraps={wraps}"),
    );

    // Select inside the dialog.
    let listbox = "const l = document.querySelector('[role=listbox]'); if (!l) return null; const o = l.querySelector('[role=option]'); const r = o.getBoundingClientRect(); const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2); const d = document.querySelector('[role=dialog]'); return { g: geo(l), z: zOf(l), dz: zOf(d), hit: l.contains(hit) };";
    if tab_until(
        app,
        "document.activeElement.getAttribute('role') === 'combobox'",
        12,
        false,
    )
    .await
    {
        key(app, Key::Enter).await;
        sleep(Duration::from_millis(300)).await;
        let l = js(app, listbox).await;
        println!("INFO Gallery Dialog Select open: {l}");
        checks.check("Dialog Select: the list is above the dialog (z-index), inside the viewport and its options are hit-testable", l["z"].as_i64() > l["dz"].as_i64() && l["g"]["inside"] == true && l["hit"] == true, l.to_string());
        key(app, Key::Down).await;
        key(app, Key::Enter).await;
        sleep(Duration::from_millis(300)).await;
        let s = js(app, "return { text: document.getElementById('g-dialog-fruit').textContent, dialog: !!document.querySelector('[role=dialog]'), list: !!document.querySelector('[role=listbox]') };").await;
        checks.check("Dialog Select: Down+Enter changes the value, the list closes and the dialog stays open", s["dialog"] == true && s["list"] == false && s["text"] != "dialog fruit=apple", s.to_string());
        key(app, Key::Space).await;
        sleep(Duration::from_millis(300)).await;
        key(app, Key::Escape).await;
        sleep(Duration::from_millis(300)).await;
        let s = js(app, "return { dialog: !!document.querySelector('[role=dialog]'), list: !!document.querySelector('[role=listbox]'), role: document.activeElement.getAttribute('role') };").await;
        checks.check("Dialog Select: Escape closes only the list; the dialog stays open and focus returns to the trigger", s["dialog"] == true && s["list"] == false && s["role"] == "combobox", s.to_string());
    } else {
        checks.check(
            "Dialog Select: reached by Tab",
            false,
            "no combobox in dialog",
        );
    }

    // Toast raised while the dialog is open.
    if tab_until(
        app,
        "document.activeElement.id === 'g-dialog-toast'",
        12,
        false,
    )
    .await
    {
        key(app, Key::Space).await;
        let shown = js(app, "return !!(await until(() => document.querySelector('[data-toast-type=success]'), 4000));").await == true;
        checks.check(
            "Dialog Toast: Space on a dialog button raises a success Toast",
            shown,
            "no toast",
        );
        if shown {
            sleep(Duration::from_millis(500)).await;
            let t = js(app, "const t = document.querySelector('[data-toast-type=success]'); const d = document.querySelector('[role=dialog]'); const close = t.querySelector('button'); const r = close.getBoundingClientRect(); const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2); const cs = getComputedStyle(t); const unit = parseFloat(css(root, 'fontSize')) * 4 / 14; return { g: geo(t), z: zOf(t), dz: zOf(d), role: t.getAttribute('role'), live: t.getAttribute('aria-live'), hidden: !!t.closest('[aria-hidden=true]'), hit: t.contains(hit), pointer: cs.pointerEvents, wraps: t.scrollWidth <= t.clientWidth + 1, minW: 75 * unit, maxW: 100 * unit, anim: cs.animationName, animMs: cs.animationDuration, reduced: matchMedia('(prefers-reduced-motion: reduce)').matches }; ").await;
            println!("INFO Gallery Dialog Toast: {t}");
            checks.check("Dialog Toast: above the dialog, not inside an aria-hidden subtree, role=status/aria-live=polite", t["z"].as_i64() > t["dz"].as_i64() && t["hidden"] == false && t["role"] == "status" && t["live"] == "polite", t.to_string());
            checks.check("Dialog Toast: its close button is the hit target at its own centre (not covered by the overlay) and it takes pointer events", t["hit"] == true && t["pointer"] == "auto", t.to_string());
            let w = &t["g"]["w"];
            checks.check("Dialog Toast: inside the viewport, 300-400px at 100%, long text wraps without horizontal scroll", t["g"]["inside"] == true && w.as_f64().unwrap_or(0.0) >= t["minW"].as_f64().unwrap_or(0.0) - 1.0 && w.as_f64().unwrap_or(9e9) <= t["maxW"].as_f64().unwrap_or(0.0) + 1.0 && t["wraps"] == true, t.to_string());
            checks.check("Dialog Toast: enter animation runs (animation-name set, duration > 0) when motion is not reduced", t["reduced"] == true || (t["anim"] != "none" && t["animMs"] != "0s"), t.to_string());
        }
    } else {
        checks.check(
            "Dialog Toast: trigger reached by Tab",
            false,
            "g-dialog-toast not reached",
        );
    }

    // Tooltip inside the dialog.
    if tab_until(
        app,
        "document.activeElement.id === 'g-dialog-tip'",
        12,
        false,
    )
    .await
    {
        sleep(Duration::from_millis(700)).await;
        let t = js(app, "const tip = document.querySelector('[role=tooltip]'); const d = document.querySelector('[role=dialog]'); return tip && { g: geo(tip), z: zOf(tip), dz: zOf(d) };").await;
        println!("INFO Gallery Dialog Tooltip: {t}");
        checks.check(
            "Dialog Tooltip: opens on focus, above the dialog, inside the viewport",
            !t.is_null() && t["z"].as_i64() >= t["dz"].as_i64() && t["g"]["inside"] == true,
            t.to_string(),
        );
        key(app, Key::Escape).await;
        sleep(Duration::from_millis(300)).await;
        let s = js(app, "return { tip: !!document.querySelector('[role=tooltip]'), dialog: !!document.querySelector('[role=dialog]') };").await;
        println!("INFO Gallery Dialog Tooltip Escape: {s}");
        checks.check(
            "Dialog Tooltip: the first Escape closes the tooltip",
            s["tip"] == false,
            s.to_string(),
        );
        if s["dialog"] == true {
            key(app, Key::Escape).await;
        }
    } else {
        checks.check(
            "Dialog Tooltip: reached by Tab",
            false,
            "g-dialog-tip not reached",
        );
        key(app, Key::Escape).await;
    }
    sleep(Duration::from_millis(500)).await;
    let s = js(app, "return { dialog: !!document.querySelector('[role=dialog]'), focus: document.activeElement.id, rootHidden: document.getElementById('root').getAttribute('aria-hidden') };").await;
    checks.check(
        "Gallery Dialog: Escape closes it, focus returns to the opener, the page is exposed again",
        s["dialog"] == false && s["focus"] == "g-open" && s["rootHidden"].is_null(),
        s.to_string(),
    );
}

/// Focused native layout checks; no route/theme matrix and no DOM keyboard-event substitutes.
pub(super) async fn modal_viewports(app: &AppHandle, checks: &mut Checks) {
    let window = app.get_webview_window(MAIN).expect("main window");
    // Independent typography is persisted through the production appearance command, never injected CSS.
    for (ui, reading, code, expected) in [(12,20,18,[13.0,20.0,18.0]), (16,20,18,[17.0,20.0,18.0]), (16,13,18,[17.0,13.0,18.0]), (16,13,12,[17.0,13.0,12.0])] {
        let read = super::ipc(app, MAIN, "get_appearance_preferences", json!({})).await;
        let mut snapshot = read.data()["snapshot"].clone();
        let revision = snapshot.as_object_mut().and_then(|v| v.remove("revision"));
        snapshot["density"] = json!(90);
        snapshot["typography"]["uiSize"] = json!(ui);
        snapshot["typography"]["readingSize"] = json!(reading);
        snapshot["typography"]["codeSize"] = json!(code);
        snapshot["typography"]["lineHeight"] = json!(2);
        snapshot["effects"]["reducedMotion"] = json!("reduce");
        let saved = super::ipc(app, MAIN, "set_appearance_preferences", json!({"snapshot":snapshot,"expectedRevision":revision})).await.success();
        let loaded = open_gallery(app).await;
        let measured = js(app, "return ['g-input-ui','g-input-reading','g-input-code'].map(id=>{const e=document.getElementById(id);return {size:parseFloat(css(e,'fontSize')),line:parseFloat(css(e,'lineHeight')),value:e.value};});").await;
        let sizes = measured.as_array().map(|values| values.iter().zip(expected).all(|(v,w)| (v["size"].as_f64().unwrap_or(0.0)-w).abs()<0.05)).unwrap_or(false);
        checks.check(&format!("Input independent UI{ui}/reading{reading}/code{code}"), saved && loaded && sizes && measured[1]["line"].as_f64() == Some(reading as f64 * 2.0) && measured[2]["line"].as_f64() == Some(code as f64 * 1.5), measured.to_string());
    }
    for value in ["zh", "en"] {
        js(app, "document.getElementById('g-long-select').focus(); return true;").await;
        if value == "en" {
            key(app, Key::Enter).await;
            js(app, "return !!(await until(()=>document.activeElement.getAttribute('role')==='option'));").await;
            key(app, Key::Down).await;
            key(app, Key::Enter).await;
        }
        let geometry = js(app, "const e=document.getElementById('g-long-select'),v=e.firstElementChild,full=document.getElementById(e.getAttribute('aria-describedby')); return {g:geo(e), h:e.clientHeight, valueH:v.getBoundingClientRect().height, overflow:e.scrollWidth>e.clientWidth+1, full:full?.textContent, title:e.title, ellipsis:css(v,'textOverflow'), white:css(v,'whiteSpace'), focus:document.activeElement===e};").await;
        checks.check(&format!("Select long {value} @90% UI16: native selection, single line and full accessible value"), geometry["g"]["inside"] == true && geometry["overflow"] == false && geometry["ellipsis"] == "ellipsis" && geometry["white"] == "nowrap" && geometry["full"] == geometry["title"] && geometry["full"].as_str().unwrap_or("").starts_with(if value == "zh" { "中文" } else { "LongEnglish" }) && geometry["valueH"].as_f64().unwrap_or(999.0) <= geometry["h"].as_f64().unwrap_or(0.0)+1.0 && geometry["focus"] == true, geometry.to_string());
    }
    window.set_theme(Some(tauri::Theme::Light)).ok();
    for (width, density) in [(520u32, 150u32), (800, 100), (800, 150)] {
        let tag = format!("Modal {width}x600 @{density}%");
        let resized = window.set_size(tauri::LogicalSize::new(width as f64, 600.0)).is_ok();
        let saved = set_appearance(app, "light", density).await;
        if !resized || !saved || !open_gallery(app).await {
            checks.check(&tag, false, "window, appearance or gallery unavailable");
            continue;
        }
        let opened = js(app, "document.getElementById('g-open-long-form').focus(); return document.activeElement.id === 'g-open-long-form';").await == true;
        key(app, Key::Enter).await;
        let found = js(app, "return !!(await until(() => document.getElementById('g-modal-form')));").await == true;
        checks.check(&format!("{tag}: native Enter opens long form consumer"), opened && found, "g-modal-form");
        if !found { continue; }
        let settled = js(app, "return !!(await until(() => { const d=document.querySelector('[data-modal-kind]'); return root.dataset.reducedMotion === 'reduce' && css(d,'transform') === 'none' && css(d,'animationName') === 'none'; }));").await == true;
        checks.check(&format!("{tag}: production reduced-motion preference settles geometry"), settled, "no active modal transform or animation");
        let initial = js(app, r#"
const d = document.querySelector('[data-modal-kind]');
const body = d.querySelector('.ui-modal-body');
const heading = d.querySelector('.ui-modal-heading');
const footer = d.querySelector('.ui-modal-footer');
const close = d.querySelector('button[aria-label="关闭"]');
const hit = el => { const r = el.getBoundingClientRect(); return el.contains(document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2)); };
const buttons = [...footer.querySelectorAll('button')];
return { dialog:geo(d), heading:geo(heading), footer:geo(footer), close:geo(close), closeHit:hit(close),
  rootPx:parseFloat(css(root,'fontSize')), density:root.dataset.density, initial:document.activeElement.id,
  bodyScrolls:body.scrollHeight > body.clientHeight + 1, horizontal:d.scrollWidth > d.clientWidth + 1 || body.scrollWidth > body.clientWidth + 1,
  footerHit:buttons.every(hit), footerRows:new Set(buttons.map(b => Math.round(b.getBoundingClientRect().top))).size,
  titleClipped:heading.scrollWidth > heading.clientWidth + 1, pageScroll:root.scrollWidth > root.clientWidth + 1,
  firstFont:parseFloat(css(document.getElementById('g-modal-first'),'fontSize')) };
"#).await;
        let unit = density as f64 / 100.0;
        let nominal = 400.0 * unit;
        let cap = (600.0 * unit).min(width as f64 - 32.0 * unit);
        let actual_width = initial["dialog"]["w"].as_f64().unwrap_or(0.0);
        checks.check(&format!("{tag}: viewport clamp, density retained, no horizontal overflow"),
            initial["dialog"]["inside"] == true && initial["dialog"]["vw"].as_u64() == Some(width as u64)
                && actual_width >= nominal.min(cap) - 1.0 && actual_width <= cap + 1.0
                && initial["horizontal"] == false && initial["pageScroll"] == false
                && initial["density"] == density.to_string()
                && (initial["rootPx"].as_f64().unwrap_or(0.0) - 14.0 * unit).abs() < 0.05
                && initial["firstFont"].as_f64().unwrap_or(0.0) >= 12.0,
            initial.to_string());
        checks.check(&format!("{tag}: title, close and wrapping footer reachable above long body"),
            initial["heading"]["inside"] == true && initial["titleClipped"] == false
                && initial["close"]["inside"] == true && initial["closeHit"] == true
                && initial["footer"]["inside"] == true && initial["footerHit"] == true
                && initial["bodyScrolls"] == true && initial["initial"] == "g-modal-first"
                && (width != 520 || initial["footerRows"].as_u64().unwrap_or(0) > 1),
            initial.to_string());

        let reached = tab_until(app, "document.activeElement.id === 'g-modal-last'", 8, false).await;
        let scrolled = js(app, r#"
const d = document.querySelector('[data-modal-kind]'); const b = d.querySelector('.ui-modal-body');
const last = document.getElementById('g-modal-last');
await until(() => { const r=last.getBoundingClientRect(); const br=b.getBoundingClientRect(); return r.top >= br.top + 4 - 0.5 && r.bottom <= br.bottom - 4 + 0.5; }, 2000);
const r = last.getBoundingClientRect(); const br = b.getBoundingClientRect();
const close = d.querySelector('button[aria-label="关闭"]'); const cr = close.getBoundingClientRect();
return { visible:r.top >= br.top - 1 && r.bottom <= br.bottom + 1, field:{top:r.top,bottom:r.bottom,height:r.height}, body:{top:br.top,bottom:br.bottom,height:br.height,clientHeight:b.clientHeight,scrollHeight:b.scrollHeight}, scrollTop:b.scrollTop,
  focus:focusInfo(), cuts:ringCuts(), transform:css(d,'transform'),
  panelScroll:d.scrollTop, footer:geo(d.querySelector('.ui-modal-footer')), close:geo(close),
  closeHit:close.contains(document.elementFromPoint(cr.left + cr.width / 2,cr.top + cr.height / 2)) };
"#).await;
        checks.check(&format!("{tag}: physical Tab scrolls to final body field while footer and close remain reachable"),
            reached && scrolled["visible"] == true && scrolled["scrollTop"].as_f64().unwrap_or(0.0) > 0.0
                && scrolled["panelScroll"] == 0 && scrolled["footer"]["inside"] == true
                && scrolled["close"]["inside"] == true && scrolled["closeHit"] == true
                && ring_problem(&scrolled["focus"]).is_none()
                && scrolled["cuts"].as_array().is_some_and(|cuts| cuts.is_empty()),
            scrolled.to_string());

        let combo = tab_until(app, "document.activeElement.getAttribute('role') === 'combobox'", 4, true).await;
        key(app, Key::Enter).await;
        let list = js(app, "return !!(await until(() => document.querySelector('[role=listbox]')));").await == true;
        key(app, Key::Escape).await;
        let nested = js(app, "await wait(300); return { dialog:!!document.querySelector('[data-modal-kind]'), list:!!document.querySelector('[role=listbox]'), role:document.activeElement.getAttribute('role') };").await;
        checks.check(&format!("{tag}: nested Select Escape dismisses only its menu"),
            combo && list && nested["dialog"] == true && nested["list"] == false && nested["role"] == "combobox", nested.to_string());

        let confirm = tab_until(app, "document.activeElement.closest('.ui-modal-footer') && document.activeElement.textContent.trim() === 'Confirm reviewed request'", 8, false).await;
        let focused = js(app, "const d=document.querySelector('[data-modal-kind]'); return {inside:d.contains(document.activeElement) && geo(document.activeElement).inside,focus:focusInfo(),cuts:ringCuts()};").await;
        checks.check(&format!("{tag}: native footer focus ring is visible and unclipped"), confirm && focused["inside"] == true && ring_problem(&focused["focus"]).is_none() && focused["cuts"].as_array().is_some_and(|cuts| cuts.is_empty()), focused.to_string());
        key(app, Key::Enter).await;
        let busy = js(app, "return !!(await until(() => document.querySelector('.ui-modal-footer [aria-busy=true]')));").await == true;
        key(app, Key::Escape).await;
        let protected = js(app, "const d=document.querySelector('[data-modal-kind]'); return !!d && d.querySelector('button[aria-label=\"关闭\"]').disabled && [...d.querySelectorAll('.ui-modal-footer button')].some(b => b.disabled);").await == true;
        checks.check(&format!("{tag}: native footer confirmation stays open and guards Escape/close while busy"), confirm && focused["inside"] == true && busy && protected, "pending native confirmation");
        let release = tab_until(app, "document.activeElement.id === 'g-modal-release'", 6, true).await;
        key(app, Key::Enter).await;
        let closed = js(app, "return !!(await until(() => !document.querySelector('[data-modal-kind]')));").await == true;
        checks.check(&format!("{tag}: resolving the async action closes and restores opener focus"),
            release && closed && truthy(app, "document.activeElement.id === 'g-open-long-form'").await, "native body release");

        // The short production consumer must also retain its nominal fit width and dismissal semantics.
        if ready(app, "/providers").await {
            let trigger = "const row=await until(() => [...document.querySelectorAll('.provider-list-item[role=group]')].find(x => x.getAttribute('aria-label')?.includes('Provider-with-an-extremely-long'))); const b=row && [...row.querySelectorAll('button[aria-label]')].find(x => x.getAttribute('aria-label').includes('删除')); b?.focus(); return !!b && document.activeElement === b;";
            if open_expect(app, checks, &format!("{tag} Provider"), trigger, "删除 Provider？").await {
                js(app, "return !!(await until(() => css(document.querySelector('[data-modal-kind]'),'transform') === 'none'));").await;
                let production = js(app, "const d=document.querySelector('[data-modal-kind]'); return {g:geo(d),scroll:d.scrollWidth>d.clientWidth+1,footer:geo(d.querySelector('.ui-modal-footer'))};").await;
                let w = production["g"]["w"].as_f64().unwrap_or(0.0);
                checks.check(&format!("{tag}: actual Provider delete confirm fits"), production["g"]["inside"] == true && production["scroll"] == false && production["footer"]["inside"] == true && w >= nominal.min(cap)-1.0 && w <= cap+1.0, production.to_string());
                key(app, Key::Escape).await;
                checks.check(&format!("{tag}: Provider Escape keeps the seeded provider"), dialog_gone(app).await && provider_present(app).await == Some(true), "production command read-back");
            } else {
                checks.check(&format!("{tag}: production Provider consumer available"), false, "seeded provider row or accessible delete button missing");
            }
        } else {
            checks.check(&format!("{tag}: production Provider route loads"), false, "route unavailable");
        }
    }
    set_appearance(app, "light", 100).await;
}

/// Dialog and Toast geometry at one density (the dialog is opened with a real Enter on the focused opener).
async fn density_geometry(app: &AppHandle, checks: &mut Checks, density: u32) {
    set_appearance(app, "light", density).await;
    if !open_gallery(app).await {
        checks.check(
            &format!("Overlay @{density}%: gallery loaded"),
            false,
            "no gallery",
        );
        return;
    }
    js(
        app,
        "document.getElementById('g-open').focus(); return true;",
    )
    .await;
    key(app, Key::Enter).await;
    let opened = js(
        app,
        "return !!(await until(() => document.querySelector('[role=dialog]'), 5000));",
    )
    .await
        == true;
    if !opened {
        checks.check(
            &format!("Overlay @{density}%: dialog opens"),
            false,
            "no dialog",
        );
        return;
    }
    sleep(Duration::from_millis(500)).await;
    js(
        app,
        "document.getElementById('g-dialog-toast').focus(); return true;",
    )
    .await;
    key(app, Key::Space).await;
    js(app, "await until(() => document.querySelector('[data-toast-type=success]'), 4000); await wait(500); return true;").await;
    let m = js(app, "const d = document.querySelector('[role=dialog]'); const t = document.querySelector('[data-toast-type=success]'); const unit = parseFloat(css(root, 'fontSize')) * 4 / 14; const p = document.getElementById('g-dialog-long'); return { unit, dialog: geo(d), dialogScroll: d.scrollWidth > d.clientWidth, longWraps: p.scrollWidth <= p.clientWidth + 1, toast: t && geo(t), toastWraps: t && t.scrollWidth <= t.clientWidth + 1, pageScroll: root.scrollWidth > root.clientWidth, minDialog: 100 * unit, maxDialog: 150 * unit };").await;
    println!("INFO Overlay @{density}%: {m}");
    let dw = m["dialog"]["w"].as_f64().unwrap_or(0.0);
    let vw = m["dialog"]["vw"].as_f64().unwrap_or(0.0);
    let unit = m["unit"].as_f64().unwrap_or(4.0);
    checks.check(
        &format!("Overlay @{density}%: Dialog inside the 800px viewport, <= min(150 units, viewport - 2x4 units), no horizontal scroll, long text wraps"),
        m["dialog"]["inside"] == true && dw <= (150.0 * unit).min(vw - 8.0 * unit) + 1.0 && m["dialogScroll"] == false && m["longWraps"] == true && m["pageScroll"] == false,
        m.to_string(),
    );
    checks.check(
        &format!("Overlay @{density}%: Toast inside the viewport and wraps its long text"),
        m["toast"]["inside"] == true && m["toastWraps"] == true,
        m.to_string(),
    );
    key(app, Key::Escape).await;
    sleep(Duration::from_millis(300)).await;
    // Same viewport, long ordinary prose (spaces) instead of one unbroken token: separates the two causes.
    js(
        app,
        "document.getElementById('g-open-prose').focus(); return true;",
    )
    .await;
    key(app, Key::Enter).await;
    if js(
        app,
        "return !!(await until(() => document.querySelector('[role=dialog]'), 5000));",
    )
    .await
        == true
    {
        sleep(Duration::from_millis(500)).await;
        let p = js(app, "const d = document.querySelector('[role=dialog]'); const p = document.getElementById('g-prose-long'); return { g: geo(d), scroll: d.scrollWidth > d.clientWidth, wraps: p.scrollWidth <= p.clientWidth + 1, unit: parseFloat(css(root, 'fontSize')) * 4 / 14 };").await;
        println!("INFO Prose dialog @{density}%: {p}");
        let unit = p["unit"].as_f64().unwrap_or(4.0);
        let cap = (150.0 * unit).min(p["g"]["vw"].as_f64().unwrap_or(800.0) - 8.0 * unit);
        checks.check(
            &format!("Prose dialog @{density}%: long ordinary text wraps, dialog inside the viewport and within min(150 units, viewport - 2x4 units)"),
            p["g"]["inside"] == true && p["scroll"] == false && p["wraps"] == true && p["g"]["w"].as_f64().unwrap_or(9e9) <= cap + 1.0,
            p.to_string(),
        );
        key(app, Key::Escape).await;
        sleep(Duration::from_millis(300)).await;
    } else {
        checks.check(
            &format!("Prose dialog @{density}%: opens"),
            false,
            "no dialog",
        );
    }
}

async fn error_boundary(app: &AppHandle, checks: &mut Checks) {
    if !ready(app, "/").await {
        checks.check("ErrorBoundary: Home loaded", false, "page never loaded");
        return;
    }
    // A failing state updater exercises the real boundary; retry mounts a fresh desktop.
    let injected = js(app, "const h1 = document.querySelector('.desktop-page h1'); if(!h1)return false; const fk = Object.keys(h1).find(k => k.startsWith('__reactFiber$')); let f = h1[fk]; while(f && !(f.tag === 0 && f.memoizedState)) f = f.return; let hook = f && f.memoizedState; while(hook && !(hook.queue && typeof hook.memoizedState === 'boolean')) hook = hook.next; if(!hook)return false; hook.queue.dispatch(() => { throw new Error('smoke render'); }); return true;").await;
    checks.check(
        "ErrorBoundary: injected a render-time error into the Home page",
        injected == true,
        injected.to_string(),
    );
    if injected != true {
        return;
    }
    let shown = js(
        app,
        "return !!(await until(() => document.querySelector('[data-error-kind=error]'), 4000));",
    )
    .await
        == true;
    checks.check(
        "ErrorBoundary: the error page replaces the crashed tree",
        shown,
        "no error page",
    );
    if !shown {
        return;
    }
    let s = js(app, "const e = document.querySelector('[data-error-kind=error]'); return { text: e.innerText, buttons: [...e.querySelectorAll('button')].map((b) => b.textContent.trim()), leaked: /not a child|Objects are not valid|at \\w+\\(/.test(e.innerText), nav: !!document.querySelector('nav') };").await;
    println!("INFO ErrorBoundary page: {s}");
    checks.check(
        "ErrorBoundary: fixed copy only (no error message or stack), 重试 and 返回首页 offered",
        s["leaked"] == false && s["buttons"] == json!(["重试", "返回首页"]),
        s.to_string(),
    );
    let found = tab_until(
        app,
        "document.activeElement.textContent.trim() === '重试'",
        6,
        false,
    )
    .await;
    checks.check(
        "ErrorBoundary: Tab reaches 重试 with a visible focus ring",
        found && ring_problem(&js(app, "return focusInfo();").await).is_none(),
        "retry not focusable",
    );
    if found {
        key(app, Key::Enter).await;
        let back = js(app, "return !!(await until(() => !document.querySelector('[data-error-kind=error]') && document.querySelector('h1'), 4000));").await == true;
        checks.check(
            "ErrorBoundary: Enter on 重试 remounts the page and recovers",
            back,
            "page did not recover",
        );
    }
}

pub(super) async fn run(app: &AppHandle, checks: &mut Checks) {
    let window = app.get_webview_window(MAIN).expect("main window");
    window.set_theme(Some(tauri::Theme::Light)).ok();
    set_appearance(app, "light", 100).await;
    if !prepare(app).await {
        checks.check(
            "keyboard: window made key with the WKWebView as first responder",
            false,
            "no macOS window handle",
        );
        return;
    }
    if !ready(app, "/").await {
        checks.check("keyboard: Home loaded", false, "page never loaded");
        return;
    }
    // Pre-flight: a real Tab must move focus off <body>, in a focused document, or nothing below means anything.
    key(app, Key::Tab).await;
    let pre = js(app, "return focusInfo();").await;
    println!("INFO keyboard preflight: {pre}");
    checks.check(
        "keyboard preflight: a physical Tab moves focus to a control in a focused document",
        pre["body"] == false && pre["hasFocus"] == true,
        pre.to_string(),
    );
    if pre["body"] != false {
        return;
    }
    p7_leftovers(app, checks).await;
    home(app, checks).await;
    skills_tabs(app, checks).await;
    if open_gallery(app).await {
        gallery_controls(app, checks).await;
        gallery_dialog(app, checks).await;
    } else {
        checks.check(
            "Gallery: p8-gallery module rendered",
            false,
            "g-open never appeared",
        );
    }
    for density in [60, 70, 80, 90, 100, 110, 120, 130, 140, 150] {
        density_geometry(app, checks, density).await;
    }
    gallery_themes(app, checks).await;
    set_appearance(app, "dark", 100).await;
    ring_walk(app, checks, "Dark theme").await;
    set_appearance(app, "light", 100).await;
    error_boundary(app, checks).await;
    appearance_e2e(app, checks).await;
    settings_tab_rings(app, checks).await;
    focus_ring_targets(app, checks).await;
    confirm_dialogs(app, checks).await;
}

/// AC23 on the real renderer: the `host.ui` components a module renders follow the theme and the scale. In each
/// (theme, density) cell the primary Button's background equals `--color-primary`, and an open Modal's background
/// and text equal `--color-bg-primary` and `--color-text-primary` (the same tokens the host pages use), with the
/// root font size at 14 x density. The two themes must also differ from each other, so a stuck palette fails.
async fn gallery_themes(app: &AppHandle, checks: &mut Checks) {
    let mut primary_by_theme: Vec<(String, Value)> = Vec::new();
    for (theme, density) in [("light", 100u32), ("dark", 100), ("dark", 150)] {
        let tag = format!("host.ui {theme}/{density}%");
        set_appearance(app, theme, density).await;
        if !open_gallery(app).await {
            checks.check(&format!("{tag}: gallery loaded"), false, "no gallery");
            continue;
        }
        let page = js(
            app,
            "const b = document.getElementById('g-open'); return { btn: parse(css(b, 'backgroundColor')).c, tokPrimary: tokenColor('--color-primary'), font: parseFloat(css(root, 'fontSize')), theme: root.dataset.theme };",
        )
        .await;
        js(
            app,
            "document.getElementById('g-open').focus(); return true;",
        )
        .await;
        key(app, Key::Enter).await;
        let dialog = js(
            app,
            "const d = await until(() => document.querySelector('[role=dialog]'), 5000); if (!d) return null; await wait(500); return { bg: parse(css(d, 'backgroundColor')).c, text: parse(css(d, 'color')).c, tokBg: tokenColor('--color-bg-primary'), tokText: tokenColor('--color-text-primary') };",
        )
        .await;
        key(app, Key::Escape).await;
        sleep(Duration::from_millis(300)).await;
        let want_font = 14.0 * f64::from(density) / 100.0;
        let ok = page["theme"] == theme
            && page["btn"] == page["tokPrimary"]
            && page["font"]
                .as_f64()
                .is_some_and(|f| (f - want_font).abs() < 0.01)
            && dialog["bg"] == dialog["tokBg"]
            && dialog["text"] == dialog["tokText"]
            && dialog["bg"].is_array();
        checks.check(
            &format!("{tag}: gallery Button and Modal use the theme tokens and the root font is {want_font}px"),
            ok,
            format!("{page} {dialog}"),
        );
        primary_by_theme.push((format!("{theme}/{density}"), page["btn"].clone()));
    }
    let light = primary_by_theme
        .iter()
        .find(|(t, _)| t == "light/100")
        .map(|(_, v)| v);
    let dark = primary_by_theme
        .iter()
        .find(|(t, _)| t == "dark/100")
        .map(|(_, v)| v);
    checks.check(
        "host.ui: the primary Button colour differs between light and dark",
        matches!((light, dark), (Some(l), Some(d)) if l.is_array() && d.is_array() && l != d),
        format!("{primary_by_theme:?}"),
    );
    set_appearance(app, "light", 100).await;
}

async fn host_appearance(app: &AppHandle) -> Value {
    super::ipc(app, MAIN, "get_appearance_preferences", json!({}))
        .await
        .0["value"]["data"]["snapshot"]
        .clone()
}

/// Poll the host until `field` equals `want` (the save is asynchronous), or 3 s pass.
async fn host_has(app: &AppHandle, field: &str, want: Value) -> bool {
    for _ in 0..30 {
        if host_appearance(app).await[field] == want {
            return true;
        }
        sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Focus the radio named `label` (a known starting control, not a key press) and report whether it took focus.
fn focus_radio(label: &str) -> String {
    format!(
        "const r = [...document.querySelectorAll('[role=\"radio\"]')].find((x) => lbl(x) === {label:?}); if (!r) return false; r.focus(); return document.activeElement === r;"
    )
}

/// Production editor: keyboard preview, explicit apply, reload and failure rollback.
async fn appearance_e2e(app: &AppHandle, checks: &mut Checks) {
    set_appearance(app,"light",100).await;
    if !ready(app,"/settings/appearance").await {
        checks.check("appearance editor loads",false,"page never loaded");return;
    }
    let before=host_appearance(app).await;
    let focused=js(app,&focus_radio("亮色")).await==true;
    key(app,Key::Right).await;
    let preview=js(app,"return !!(await until(()=>root.dataset.theme==='dark'));").await;
    checks.check("keyboard theme preview does not persist",focused&&preview==true&&host_appearance(app).await==before,preview.to_string());
    let apply="const b=[...document.querySelectorAll('button')].find(x=>x.textContent.trim()==='应用外观');if(!b)return false;b.focus();return document.activeElement===b;";
    let focused=js(app,apply).await==true;
    key(app,Key::Enter).await;
    checks.check("Enter applies full dark snapshot",focused&&host_has(app,"mode",json!("dark")).await,host_appearance(app).await.to_string());
    let loaded=ready(app,"/settings/appearance").await;
    let dark=js(app,"return root.dataset.theme==='dark';").await;
    checks.check("editor reload reads confirmed dark snapshot",loaded&&dark==true,dark.to_string());
    let probe=&app.state::<UiProbe>().0;
    let armed=probe.execute_unprepared("CREATE TRIGGER fail_theme BEFORE UPDATE ON system_settings WHEN NEW.setting_key='appearance.snapshot.v2' BEGIN SELECT RAISE(ABORT,'scripted failure'); END").await.is_ok();
    js(app,&focus_radio("暗色")).await;
    key(app,Key::Left).await;
    js(app,apply).await;
    key(app,Key::Enter).await;
    let rollback=js(app,"const alert=await until(()=>[...document.querySelectorAll('[role=alert]')].find(x=>x.textContent.includes('草稿保留')));return {alert:!!alert,theme:root.dataset.theme,leaked:document.body.textContent.includes('scripted failure')};").await;
    checks.check("failed apply restores confirmed theme and keeps draft",armed&&rollback["alert"]==true&&rollback["theme"]=="dark"&&rollback["leaked"]==false&&host_appearance(app).await["mode"]=="dark",rollback.to_string());
    let _=probe.execute_unprepared("DROP TRIGGER fail_theme").await;
    set_appearance(app,"light",100).await;
}

/// Tab through every stop of the Home page and check each focus ring, in the theme already applied.
async fn ring_walk(app: &AppHandle, checks: &mut Checks, tag: &str) {
    if !ready(app, "/").await {
        checks.check(&format!("{tag}: Home loaded"), false, "page never loaded");
        return;
    }
    let n = js(app, "return tabbables().length;")
        .await
        .as_u64()
        .unwrap_or(0) as usize;
    let mut bad = vec![];
    for _ in 0..n {
        key(app, Key::Tab).await;
        let info = js(app, "return focusInfo();").await;
        if let Some(p) = ring_problem(&info) {
            bad.push(format!("{}: {p}", info["label"]));
        }
    }
    let theme = js(app, "return root.getAttribute('data-theme');").await;
    checks.check(
        &format!("{tag}: every Home tab stop ({n}) has a visible focus ring (data-theme={theme})"),
        n >= 9 && bad.is_empty(),
        bad.join(" | "),
    );
}

/// FR-007: the vertical category list uses manual activation. Native Home/Down must visit all eight
/// categories without navigating; every focused tab retains an unclipped ring at each density.
async fn settings_tab_rings(app: &AppHandle, checks: &mut Checks) {
    for density in [60, 70, 80, 90, 100, 110, 120, 130, 140, 150] {
        let tag = format!("Settings tabs @{density}%");
        set_appearance(app, "light", density).await;
        if !ready(app, "/settings/about").await {
            checks.check(
                &format!("{tag}: /settings/about loaded"),
                false,
                "page never loaded",
            );
            continue;
        }
        let in_bar = "document.activeElement.getAttribute('role') === 'tab' && !!document.activeElement.closest('[role=\"tablist\"][aria-label=\"系统设置\"]')";
        if !tab_until(app, in_bar, 30, false).await {
            checks.check(
                &format!("{tag}: Tab reaches the Settings tab bar"),
                false,
                "no Settings tab focused",
            );
            continue;
        }
        let (mut seen, mut bad) = (vec![], vec![]);
        for step in 0..8 {
            if !prepare(app).await {
                bad.push(format!("step {step}: native window could not regain focus"));
                break;
            }
            key(app, if step == 0 { Key::Home } else { Key::Down }).await;
            // A foreground application change between key delivery and measurement invalidates
            // WebKit's keyboard modality. Regain focus and resend this idempotent key before measuring.
            if !truthy(app, "document.hasFocus()").await {
                if !prepare(app).await {
                    bad.push(format!("step {step}: native window lost focus before measurement"));
                    break;
                }
                key(app, Key::Home).await;
                for _ in 0..step { key(app, Key::Down).await; }
            }
            let info = js(app, "return { ...focusInfo(), cuts: ringCuts() };").await;
            let label = info["label"].as_str().unwrap_or("?").to_owned();
            if let Some(p) = ring_problem(&info) {
                bad.push(format!("{label}: {p}"));
            } else if info["cuts"].as_array().is_none_or(|c| !c.is_empty()) {
                bad.push(format!("{label}: clipped {}", info["cuts"]));
            }
            seen.push(label);
        }
        let want = [
            "Provider",
            "模块",
            "外观",
            "关于",
            "局域网协同",
            "账户",
            "CLI 工具",
            "存储",
        ];
        checks.check(
            &format!("{tag}: Home then Down visits the eight tabs without activating them"),
            seen == want && truthy(app, "location.pathname === '/settings/about'").await,
            seen.join(" > "),
        );
        checks.check(
            &format!(
                "{tag}: every focused tab shows a full focus ring that no clipping ancestor cuts"
            ),
            bad.is_empty(),
            bad.join(" | "),
        );
    }
    set_appearance(app, "light", 100).await;
}

/// Tab to a control and require a visible contrasting keyboard position plus an unclipped boundary.
/// Fields may use their existing border; actions may use a contained outline.
async fn ring_target(app: &AppHandle, name: &str, pred: &str, max: usize) -> Option<String> {
    if !tab_until(app, pred, max, false).await {
        return Some(format!("{name}: not reached by Tab"));
    }
    let info = js(app, "return { ...focusInfo(), cuts: ringCuts() };").await;
    if let Some(p) = ring_problem(&info) {
        return Some(format!("{name}: {p}"));
    }
    if info["cuts"].as_array().is_none_or(|c| !c.is_empty()) {
        return Some(format!("{name}: clipped {}", info["cuts"]));
    }
    None
}

/// AC24 on the real renderer, for the cases the earlier walks did not reach: the Button `ghost` and `link` variants,
/// an `Input` in its error state (`aria-invalid`), a checked `Checkbox`, and the Toast close button reached by Tab.
/// Each is focused by synthetic Tab presses from the gallery (`p8-gallery.js`, appended after the prose-dialog
/// opener) and checked with `ring_target`, in three theme/scale cells. The `Checkbox` has no error state in its API
/// (FR-005 §7.2.4), so its checked state stands in for it. Not proven: a visual look at the ring (computed style and
/// geometry only), physical keys, any platform other than macOS WKWebView.
async fn focus_ring_targets(app: &AppHandle, checks: &mut Checks) {
    let ghost = "document.activeElement.id === 'g-ghost'";
    let link = "document.activeElement.id === 'g-link'";
    let input = "document.activeElement.id === 'g-input-error' && document.activeElement.getAttribute('aria-invalid') === 'true'";
    let checkbox = "document.activeElement.getAttribute('role') === 'checkbox' && document.activeElement.getAttribute('aria-checked') === 'true'";
    let trigger = "document.activeElement.id === 'g-toast-long'";
    let close = "document.activeElement.getAttribute('aria-label') === '关闭' && !!document.activeElement.closest('[data-toast-type=\"error\"]')";
    for (theme, density) in [("light", 100u32), ("light", 150), ("dark", 100)] {
        let tag = format!("Focus ring targets {theme}/{density}%");
        set_appearance(app, theme, density).await;
        if !open_gallery(app).await {
            checks.check(&format!("{tag}: gallery loaded"), false, "no gallery");
            continue;
        }
        js(
            app,
            "document.getElementById('g-open-prose').focus(); return true;",
        )
        .await;
        for (name, pred) in [
            ("Button ghost", ghost),
            ("Button link", link),
            ("Input error state", input),
            ("Checkbox checked", checkbox),
        ] {
            let problem = ring_target(app, name, pred, 4).await;
            checks.check(
                &format!("{tag}: {name} shows a full focus ring that no clipping ancestor cuts"),
                problem.is_none(),
                problem.unwrap_or_default(),
            );
        }
        // Toast close button: raise a long-lived error Toast with a real Enter, then Tab to its close button.
        let problem = if !tab_until(app, trigger, 4, false).await {
            Some("Toast trigger not reached by Tab".to_owned())
        } else {
            key(app, Key::Enter).await;
            let shown = js(app, "return !!(await until(() => [...document.querySelectorAll('[data-toast-type=\"error\"]')].find((t) => t.textContent.includes('Focus-ring toast')), 4000));").await == true;
            if shown {
                ring_target(app, "Toast close button", close, 12).await
            } else {
                Some("the error Toast never appeared".to_owned())
            }
        };
        checks.check(
            &format!("{tag}: Tab reaches the Toast close button, which shows a full focus ring that no clipping ancestor cuts"),
            problem.is_none(),
            problem.unwrap_or_default(),
        );
        // The close button works from the keyboard and the Toast leaves.
        if truthy(app, close).await {
            key(app, Key::Enter).await;
            let gone = js(app, "return !!(await until(() => !document.querySelector('[data-toast-type=\"error\"]'), 3000));").await == true;
            checks.check(
                &format!("{tag}: Enter on the focused Toast close button dismisses the Toast"),
                gone,
                "Toast still shown",
            );
        }
    }
    set_appearance(app, "light", 100).await;
}

/// P7 hand-off questions that need the real WebView or the real host, answered by observation.
async fn p7_leftovers(app: &AppHandle, checks: &mut Checks) {
    // 1. `type="url"` was removed from the MCP Server URL input. The host must still reject a malformed URL
    //    on save (the native form validation that used to run first is gone).
    let bad = super::ipc(
        app,
        MAIN,
        "create_mcp_server",
        json!({"request": {"name": "bad-url", "transport": "http", "http": {"url": "not a url"},
                           "enabled": false, "auto_connect": false}}),
    )
    .await;
    let message = bad.0["error"].as_str().unwrap_or("").to_owned();
    checks.check(
        "P7 leftover: the host rejects a malformed MCP URL on save (replaces the removed native type=url check)",
        bad.0["ok"] == false && message.to_lowercase().contains("invalid"),
        bad.describe(),
    );

    // 2. `window.confirm` in WKWebView: observed, not asserted. wry 0.57 has no JS-confirm UI delegate, so it
    //    answers `false` at once. No shipped page calls it any more (Provider and MCP confirms use `Modal.confirm`;
    //    `confirm_dialogs` below proves those on the real renderer). Kept as the record of why they were moved.
    let confirm = js(
        app,
        "const t = Date.now(); const v = window.confirm('P8 probe: this should not hang the page'); return { value: v, ms: Date.now() - t };",
    )
    .await;
    println!("INFO window.confirm in this WKWebView returned: {confirm}");
    checks.check(
        "P7 leftover: window.confirm returns to the page instead of hanging (its value is recorded, not asserted)",
        confirm["value"].is_boolean(),
        confirm.to_string(),
    );
}

const DLG: &str = r#"
const dlg = () => [...document.querySelectorAll('[data-modal-kind][role="dialog"]')].at(-1);
const dlgTitle = () => { const d = dlg(); const t = d && document.getElementById(d.getAttribute('aria-labelledby')); return t ? t.textContent.trim() : null; };
const dlgState = () => { const d = dlg(); return d ? { title: dlgTitle(), inside: d.contains(document.activeElement), focus: lbl(document.activeElement), buttons: [...d.querySelectorAll('button')].map((b) => lbl(b)) } : null; };
const nameInput = () => { const l = [...document.querySelectorAll('label')].find((x) => x.textContent.trim().startsWith('名称')); return l && l.control; };
"#;

async fn jd(app: &AppHandle, body: &str) -> Value {
    js(app, &format!("{DLG}{body}")).await
}

/// Focus the trigger (`focus_js` returns true once it did; starting from a known control), press a physical Enter,
/// and expect the confirm dialog `title` with focus moved inside it.
async fn open_expect(
    app: &AppHandle,
    checks: &mut Checks,
    tag: &str,
    focus_js: &str,
    title: &str,
) -> bool {
    let state = if jd(app, focus_js).await == true {
        key(app, Key::Enter).await;
        jd(app, "return await until(() => dlgState());").await
    } else {
        json!({"__error": "trigger not focusable"})
    };
    let ok = state["title"] == title && state["inside"] == true;
    checks.check(
        &format!("{tag}: Enter on the trigger opens \"{title}\" with focus inside the dialog"),
        ok,
        state.to_string(),
    );
    ok
}

async fn dialog_gone(app: &AppHandle) -> bool {
    jd(app, "return !!(await until(() => !dlg(), 4000));").await == true
}

/// Move focus to the dialog button named `label` with physical Tab presses.
async fn tab_to_dialog_button(app: &AppHandle, label: &str) -> bool {
    let pred = format!(
        "(() => {{ const d = [...document.querySelectorAll('[data-modal-kind][role=\"dialog\"]')].at(-1); return !!d && d.contains(document.activeElement) && lbl(document.activeElement) === {label:?}; }})()"
    );
    truthy(app, &pred).await || tab_until(app, &pred, 5, false).await
}

async fn mcp_has(app: &AppHandle, id: &str) -> bool {
    let r = super::ipc(app, MAIN, "list_mcp_servers", json!({})).await;
    r.0["value"]
        .as_array()
        .map(|a| a.iter().any(|s| s["id"] == id))
        .unwrap_or(false)
}

async fn provider_present(app: &AppHandle) -> Option<bool> {
    let r = super::ipc(app, MAIN, "list_providers", json!({})).await;
    r.0["value"]["data"].as_array().map(|a| {
        a.iter().any(|p| {
            p["name"]
                .as_str()
                .is_some_and(|n| n.starts_with("Provider-with-an-extremely-long"))
        })
    })
}

/// The confirm flows that used `window.confirm` (which WKWebView answers `false` at once): MCP delete, Provider
/// discard-unsaved and Provider delete, driven with physical Enter/Tab/Escape on the real renderer, with the
/// outcome read back from the real host commands. Runs last: it deletes the seeded Provider.
/// Setup shortcuts that are not key presses: the disposable MCP server is created over IPC, the Provider row is
/// selected with `.click()`, and the Name draft is set with the native value setter plus an `input` event.
async fn confirm_dialogs(app: &AppHandle, checks: &mut Checks) {
    let made = super::ipc(
        app,
        MAIN,
        "create_mcp_server",
        json!({"request": {"name": "confirm-probe", "transport": "http",
                           "http": {"url": "https://mcp.example.test/probe"},
                           "enabled": false, "auto_connect": false}}),
    )
    .await;
    let id = made.0["value"]["id"].as_str().unwrap_or("").to_owned();
    checks.check(
        "confirm: disposable MCP server created through the host",
        made.0["ok"] == true && !id.is_empty(),
        made.describe(),
    );
    if id.is_empty() || !ready(app, "/mcp").await {
        checks.check("confirm: /mcp loaded", false, "page never loaded");
        return;
    }

    let delete_trigger = "const a = await until(() => [...document.querySelectorAll('article')].find((x) => x.textContent.includes('confirm-probe'))); const b = a && [...a.querySelectorAll('button')].find((x) => x.textContent.trim() === '删除'); if (!b) return false; b.focus(); return document.activeElement === b;";
    let title = "删除 MCP Server？";

    if open_expect(app, checks, "MCP delete", delete_trigger, title).await {
        key(app, Key::Escape).await;
        checks.check(
            "MCP delete: Esc closes the dialog, keeps the server and returns focus to Delete",
            dialog_gone(app).await
                && mcp_has(app, &id).await
                && truthy(app, "document.activeElement && document.activeElement.textContent.trim() === '删除'").await,
            "dialog open, server missing or focus not restored",
        );
    }
    if open_expect(app, checks, "MCP delete (Cancel)", delete_trigger, title).await {
        let on_cancel = tab_to_dialog_button(app, "取消").await;
        key(app, Key::Enter).await;
        checks.check(
            "MCP delete: Enter on Cancel closes the dialog and keeps the server",
            on_cancel && dialog_gone(app).await && mcp_has(app, &id).await,
            "cancel did not close, or the server was deleted",
        );
    }
    if open_expect(app, checks, "MCP delete (Delete)", delete_trigger, title).await {
        let on_delete = tab_to_dialog_button(app, "删除").await;
        key(app, Key::Enter).await;
        let closed = dialog_gone(app).await;
        let off_page = jd(
            app,
            "return !!(await until(() => !document.body.innerText.includes('confirm-probe')));",
        )
        .await
            == true;
        checks.check(
            "MCP delete: Enter on Delete runs delete_mcp_server, closes the dialog and removes the row",
            on_delete && closed && off_page && !mcp_has(app, &id).await,
            format!("onDelete={on_delete} closed={closed} offPage={off_page}"),
        );
    }

    if !ready(app, "/providers").await {
        checks.check("confirm: /providers loaded", false, "page never loaded");
        return;
    }
    let select = "const h = await until(() => [...document.querySelectorAll('h3')].find((x) => x.textContent.includes('Provider-with-an-extremely-long'))); if (!h) return false; h.click(); return !!(await until(() => nameInput()));";
    let dirty = "const i = nameInput(); if (!i) return false; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(i, 'Unsaved draft'); i.dispatchEvent(new Event('input', { bubbles: true })); return !!(await until(() => document.querySelector('[aria-label=\"未保存的更改\"]')));";
    let new_trigger = "const b = [...document.querySelectorAll('button')].find((x) => x.textContent.trim() === '+ 新建 Provider'); if (!b) return false; b.focus(); return document.activeElement === b;";
    let discard = "放弃未保存的更改？";
    let selected = jd(app, select).await == true;
    let drafted = selected && jd(app, dirty).await == true;
    checks.check(
        "Provider discard: the seeded Provider opens in the form and an edit marks it unsaved",
        drafted,
        format!("selected={selected}"),
    );
    if drafted {
        if open_expect(app, checks, "Provider discard", new_trigger, discard).await {
            key(app, Key::Escape).await;
            checks.check(
                "Provider discard: Esc keeps the draft in place and does not open the new-provider form",
                dialog_gone(app).await
                    && jd(app, "return !!(nameInput() && nameInput().value === 'Unsaved draft' && ![...document.querySelectorAll('h2')].some((h) => h.textContent.trim() === '新建 Provider'));").await == true,
                "draft lost or the form switched",
            );
        }
        if open_expect(
            app,
            checks,
            "Provider discard (Discard)",
            new_trigger,
            discard,
        )
        .await
        {
            let on_discard = tab_to_dialog_button(app, "放弃").await;
            key(app, Key::Enter).await;
            let closed = dialog_gone(app).await;
            let fresh = jd(app, "return !!(await until(() => [...document.querySelectorAll('h2')].some((h) => h.textContent.trim() === '新建 Provider') && nameInput() && nameInput().value === ''));").await == true;
            checks.check(
                "Provider discard: Enter on Discard closes the dialog and opens the empty new-provider form",
                on_discard && closed && fresh,
                format!("onDiscard={on_discard} closed={closed} fresh={fresh}"),
            );
        }
    }

    let delete_form = "const b = await until(() => [...document.querySelectorAll('footer button')].find((x) => x.textContent.includes('删除'))); if (!b) return false; b.focus(); return document.activeElement === b;";
    let delete_title = "删除 Provider？";
    if jd(app, select).await != true {
        checks.check(
            "Provider delete: the seeded Provider opens in the form",
            false,
            "not selected",
        );
        return;
    }
    if open_expect(app, checks, "Provider delete", delete_form, delete_title).await {
        key(app, Key::Escape).await;
        checks.check(
            "Provider delete: Esc closes the dialog and keeps the Provider",
            dialog_gone(app).await && provider_present(app).await == Some(true),
            "dialog open or Provider deleted",
        );
    }
    if open_expect(
        app,
        checks,
        "Provider delete (Delete)",
        delete_form,
        delete_title,
    )
    .await
    {
        let on_delete = tab_to_dialog_button(app, "删除").await;
        key(app, Key::Enter).await;
        let closed = dialog_gone(app).await;
        let off_page = jd(app, "return !!(await until(() => !document.body.innerText.includes('Provider-with-an-extremely-long')));").await == true;
        checks.check(
            "Provider delete: Enter on Delete runs delete_provider, closes the dialog and removes the Provider",
            on_delete && closed && off_page && provider_present(app).await == Some(false),
            format!("onDelete={on_delete} closed={closed} offPage={off_page}"),
        );
    }
}

/// Narrow the native window. DOM only seeds a draft and establishes the starting control;
/// opening, selecting, cancelling and confirming use NSEvents.
async fn settings_narrow_menu(app: &AppHandle, checks: &mut Checks) {
    let resized = app.get_webview_window(MAIN).is_some_and(|window| {
        window.set_min_size(Some(tauri::LogicalSize::new(520.0, 600.0))).is_ok()
            && window.set_size(tauri::LogicalSize::new(520.0, 600.0)).is_ok()
    });
    let loaded = set_appearance(app, "light", 100).await && ready(app, "/").await;
    let entry = js(app, "const e=document.querySelector('a[href=\"/settings\"]'); e?.focus(); return e === document.activeElement;").await == true;
    key(app, Key::Enter).await;
    let menu = tab_until(app, "document.activeElement.getAttribute('role') === 'combobox' && !!document.activeElement.closest('.workspace-settings-select')", 12, false).await;
    let geometry = js(app, "const e=document.querySelector('.workspace-settings-select [role=combobox]'); return {width:innerWidth,menu:geo(e),tabsHidden:!document.querySelector('.workspace-settings-navigation').getClientRects().length,focus:focusInfo(),cuts:ringCuts()};").await;
    checks.check("settings 520px: native Tab reaches the sole visible category control",
        resized && loaded && entry && menu && geometry["width"].as_f64().unwrap_or(999.0) <= 560.0
            && geometry["tabsHidden"] == true && geometry["menu"]["inside"] == true
            && ring_problem(&geometry["focus"]).is_none()
            && geometry["cuts"].as_array().is_some_and(|cuts| cuts.is_empty()), geometry.to_string());
    if !menu { return; }
    key(app, Key::Enter).await;
    key(app, Key::Home).await;
    for _ in 0..5 { key(app, Key::Down).await; }
    key(app, Key::Enter).await;
    let selected = js(app, r#"
const input=await until(() => location.pathname==='/settings/cli' && document.querySelector('.cli-tool-row input:not([disabled])'));
const menu=document.querySelector('.workspace-settings-select [role=combobox]');
return {loaded:!!input,focus:document.activeElement===menu,value:menu.textContent};
"#).await;
    checks.check("settings narrow menu: native selection opens CLI and returns to selector",
        selected["loaded"] == true && selected["focus"] == true && selected["value"].as_str().unwrap_or("").contains("CLI"), selected.to_string());
    if selected["loaded"] != true { return; }
    key(app, Key::Enter).await;
    key(app, Key::Home).await;
    key(app, Key::Enter).await;
    let drafted = js(app, r#"
const light=await until(() => location.pathname==='/settings/appearance' && [...document.querySelectorAll('.appearance-top [role=radio]')].find(e=>lbl(e)==='亮色' && e.getAttribute('aria-checked')==='true' && !e.disabled));
if (!light) return false;
const dark=[...document.querySelectorAll('.appearance-top [role=radio]')].find(e=>lbl(e)==='暗色');
dark?.click();
const dirty=!!(await until(() => root.dataset.theme==='dark' && dark?.getAttribute('aria-checked')==='true' && [...document.querySelectorAll('.appearance-footer button')].some(e=>e.textContent.trim()==='应用外观' && !e.disabled)));
const menu=document.querySelector('.workspace-settings-select [role=combobox]');
menu?.focus();
return dirty && document.activeElement===menu;
"#).await == true;
    key(app, Key::Enter).await;
    key(app, Key::End).await;
    key(app, Key::Enter).await;
    let prompt = truthy(app, "document.querySelector('[data-modal-kind=confirm]') && location.pathname==='/settings/appearance'").await;
    key(app, Key::Escape).await;
    let cancelled = js(app, r#"
await until(() => !document.querySelector('[data-modal-kind=confirm]'));
const menu=document.querySelector('.workspace-settings-select [role=combobox]');
return {draft:root.dataset.theme==='dark' && [...document.querySelectorAll('.appearance-top [role=radio]')].some(e=>lbl(e)==='暗色' && e.getAttribute('aria-checked')==='true') && [...document.querySelectorAll('.appearance-footer button')].some(e=>e.textContent.trim()==='应用外观' && !e.disabled),path:location.pathname,focus:document.activeElement===menu,value:menu.textContent};
"#).await;
    checks.check("settings narrow menu: native Escape keeps Appearance draft and restores category focus",
        drafted && prompt && cancelled["draft"] == true && cancelled["path"] == "/settings/appearance"
            && cancelled["focus"] == true && cancelled["value"].as_str().unwrap_or("").contains("外观"), cancelled.to_string());
    key(app, Key::Enter).await;
    key(app, Key::End).await;
    key(app, Key::Enter).await;
    let discard = tab_to_dialog_button(app, "放弃并关闭").await;
    key(app, Key::Enter).await;
    let confirmed = js(app, "await until(() => location.pathname==='/settings/about' && !document.querySelector('[data-modal-kind=confirm]')); return {path:location.pathname,focus:document.activeElement===document.querySelector('.workspace-settings-select [role=combobox]')};").await;
    checks.check("settings narrow menu: native discard changes category without losing selector focus",
        discard && confirmed["path"] == "/settings/about" && confirmed["focus"] == true, confirmed.to_string());
    key(app, Key::Escape).await;
    let closed = js(app, "await until(() => !document.querySelector('.workspace-settings-dialog')); await wait(100); return {path:location.pathname,focus:document.activeElement?.getAttribute('href')};").await;
    checks.check("settings narrow menu: native Escape closes overlay and restores settings entry",
        closed["path"] == "/" && closed["focus"] == "/settings", closed.to_string());
}

/// Shipped shell/module, native key navigation, and settings viewport/density regressions.
pub(super) async fn shell(app: AppHandle, checks: &mut Checks) {
    checks.check("shell native window ready", prepare(&app).await, "WKWebView first responder");
    key(&app, Key::Tab).await;
    checks.check("shell physical Tab moves focus", truthy(&app, "document.activeElement !== document.body && document.hasFocus()").await, "native Tab preflight");
    let launcher = js(&app, r#"
document.querySelector('.workspace-navigation button').click();
await until(() => [...document.querySelectorAll('.workspace-launcher-item')].find(e => e.title === 'p8-gallery'));
const item = [...document.querySelectorAll('.workspace-launcher-item')].find(e => e.title === 'p8-gallery');
if (!item) return {error:'empty real catalog'};
item.focus();
return {name:item.textContent, dialog:!!document.querySelector('.workspace-launcher-panel')};
"#).await;
    checks.check("shell real catalog launcher", launcher["dialog"] == true, launcher.to_string());
    // P is dispatched here as an action shortcut; native Tab/Escape are exercised separately below.
    let pinned = js(&app, r#"
document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'p',bubbles:true}));
await until(() => document.querySelector('.workspace-pinned-row'));
return !!document.querySelector('.workspace-pinned-row');
"#).await;
    checks.check("shell pins catalog app", pinned == true, pinned.to_string());
    key(&app, Key::Escape).await;
    checks.check("shell native Escape closes launcher", !truthy(&app, "document.querySelector('.workspace-launcher-panel')").await, "launcher dismissed");
    let opened = js(&app, r#"
document.querySelector('.workspace-pinned-row a').click();
return !!(await until(() => document.querySelector('#g-open'),20000));
"#).await;
    checks.check("shell opens actual module view", opened == true, opened.to_string());
    let settings = js(&app, r#"
window.__shellModule = document.querySelector('#g-open');
document.querySelector('a[href="/settings"]').click();
await until(() => document.querySelector('.workspace-settings-categories'));
const dialog = document.querySelector('.workspace-settings-dialog');
return {open:!!dialog,background:window.__shellModule?.isConnected,width:dialog?.getBoundingClientRect().width,focus:dialog?.contains(document.activeElement),direction:getComputedStyle(document.querySelector('.workspace-settings-tabs')).flexDirection};
"#).await;
    checks.check("shell settings preserves module and traps focus", settings["open"] == true && settings["background"] == true && settings["focus"] == true && settings["direction"] == "row", settings.to_string());
    key(&app, Key::Escape).await;
    let returned = js(&app, "await until(() => !document.querySelector('.workspace-settings-dialog')); return {same:document.querySelector('#g-open') === window.__shellModule,focus:document.activeElement?.getAttribute('href'),path:location.pathname};").await;
    checks.check("shell settings Escape returns same module", returned["same"] == true && returned["path"] == "/apps/p8-gallery", returned.to_string());
    let baseline = set_appearance(&app, "light", 100).await;
    let guarded = js(&app, r#"
const clickText = (selector, text) => [...document.querySelectorAll(selector)].find(e => e.textContent.trim() === text)?.click();
document.querySelector('a[href="/settings"]').click();
await until(() => document.querySelector('.workspace-settings-categories'));
const appearance = document.querySelector('.workspace-settings-category[data-settings-category="appearance"]');
appearance?.dispatchEvent(new MouseEvent('mousedown', {bubbles:true,button:0}));
const light = await until(() => location.pathname === '/settings/appearance' && [...document.querySelectorAll('.appearance-top [role=radio]')].find(e => lbl(e) === '亮色' && e.getAttribute('aria-checked') === 'true' && !e.disabled));
if (!light) return {error:'Appearance did not load the light baseline'};
const dark = [...document.querySelectorAll('.appearance-top [role=radio]')].find(e => lbl(e) === '暗色');
dark?.click();
const dirty = () => root.dataset.theme === 'dark' && dark?.isConnected && dark.getAttribute('aria-checked') === 'true' && [...document.querySelectorAll('.appearance-footer button')].some(e => !e.disabled && e.textContent.trim() === '应用外观');
const drafted = !!(await until(dirty));
if (!drafted) return {error:'Appearance preview did not become dirty'};
const about = [...document.querySelectorAll('.workspace-settings-category')].find(e => e.textContent.trim() === '关于');
about?.dispatchEvent(new MouseEvent('mousedown', {bubbles:true,button:0}));
const prompt = await until(() => document.querySelector('[data-modal-kind="confirm"]'));
if (!prompt) return {error:'category guard absent',path:location.pathname};
clickText('[data-modal-kind="confirm"] button', '取消');
await until(() => !document.querySelector('[data-modal-kind="confirm"]'));
const kept = location.pathname === '/settings/appearance' && dirty();
clickText('.workspace-settings-header button', '关闭设置');
await until(() => document.querySelector('[data-modal-kind="confirm"]'));
clickText('[data-modal-kind="confirm"] button', '取消');
await until(() => !document.querySelector('[data-modal-kind="confirm"]'));
const cancelled = location.pathname === '/settings/appearance' && dirty();
clickText('.workspace-settings-header button', '关闭设置');
await until(() => document.querySelector('[data-modal-kind="confirm"]'));
clickText('[data-modal-kind="confirm"] button', '放弃并关闭');
const returned = !!(await until(() => !document.querySelector('.workspace-settings-dialog') && location.pathname === '/apps/p8-gallery'));
return {drafted,kept,cancelled,returned,same:document.querySelector('#g-open') === window.__shellModule};
"#).await;
    checks.check("shell category leave cancels without losing Appearance draft", baseline && guarded["drafted"] == true && guarded["kept"] == true, guarded.to_string());
    checks.check("shell settings close keeps Appearance draft on cancel then returns after a single discard", guarded["cancelled"] == true && guarded["returned"] == true && guarded["same"] == true, guarded.to_string());
    let command = js(&app, "const trigger=document.querySelector('.workspace-tab-actions button[title*=Ctrl]'); trigger.focus(); trigger.click(); return !!(await until(() => document.querySelector('.workspace-command input') === document.activeElement));").await;
    checks.check("shell command autofocus", command == true, command.to_string());
    key(&app, Key::Escape).await;
    checks.check("shell command Escape restores focus", truthy(&app, "!document.querySelector('.workspace-command') && document.activeElement?.title?.includes('Ctrl')").await, "command trigger restored");
    let focus = js(&app, "document.querySelector('.workspace-tab-actions button[aria-pressed]').click(); await until(() => !document.querySelector('.workspace-rail')); return !document.querySelector('.workspace-rail');").await;
    key(&app, Key::Escape).await;
    checks.check("shell focus mode native Escape restores rail", focus == true && truthy(&app, "document.querySelector('.workspace-rail')").await, focus.to_string());
    let closed = js(&app, "document.querySelector('.workspace-tab-close').click(); return !!(await until(() => !document.querySelector('[data-workspace-tab]') && location.pathname === '/'));").await;
    checks.check("shell closes workspace and returns home", closed == true, closed.to_string());
    settings_tab_rings(&app, checks).await;
    // Exercise the production scroll owner and native traversal in wide and narrow windows.
    for (width, density) in [(800u32, 60u32), (800, 150), (520, 60), (520, 150)] {
        let resized = app.get_webview_window(MAIN).is_some_and(|window| {
            window.set_min_size(Some(tauri::LogicalSize::new(520.0, 600.0))).is_ok()
                && window.set_size(tauri::LogicalSize::new(width as f64, 600.0)).is_ok()
        });
        let read = super::ipc(&app, MAIN, "get_appearance_preferences", json!({})).await;
        let mut snapshot = read.data()["snapshot"].clone();
        let revision = snapshot.as_object_mut().and_then(|value| value.remove("revision"));
        snapshot["density"] = json!(density);
        snapshot["typography"]["uiSize"] = json!(16);
        snapshot["typography"]["codeSize"] = json!(18);
        snapshot["effects"]["reducedMotion"] = json!("reduce");
        let saved = super::ipc(&app, MAIN, "set_appearance_preferences", json!({"snapshot":snapshot,"expectedRevision":revision})).await.success();
        let loaded = ready(&app, "/settings/cli").await;
        let initial = js(&app, r#"
const input = await until(() => document.querySelector('.cli-tool-row input:not([disabled])'));
if (!input) return {error:'CLI did not load'};
input.focus();
const panel = document.querySelector('[data-settings-category="cli"]');
const body = document.querySelector('.cli-settings-body');
const footer = document.querySelector('.cli-settings-actions');
return {body:geo(body),footer:geo(footer),panelScroll:panel.scrollHeight>panel.clientHeight+1,
  horizontal:root.scrollWidth>root.clientWidth+1 || body.scrollWidth>body.clientWidth+1,
  close:geo(document.querySelector('.workspace-settings-header button'))};
"#).await;
        checks.check(&format!("settings CLI {width}px @{density}% UI16/code18: one body scroll and reachable actions"),
            resized && saved && loaded && initial["horizontal"] == false && initial["panelScroll"] == false
                && initial["body"]["inside"] == true && initial["footer"]["inside"] == true
                && initial["close"]["inside"] == true, initial.to_string());
        let reached = tab_until(&app, "document.activeElement === [...document.querySelectorAll('.cli-tool-row input')].at(-1)", 20, false).await;
        let last = js(&app, r#"
const body = document.querySelector('.cli-settings-body');
const input = document.activeElement; const r=input.getBoundingClientRect(); const b=body.getBoundingClientRect();
return {visible:r.top>=b.top+3.5 && r.bottom<=b.bottom-3.5,scrollTop:body.scrollTop,focus:focusInfo(),cuts:ringCuts()};
"#).await;
        checks.check(&format!("settings CLI {width}px @{density}%: native Tab reveals final path and focus ring"),
            reached && last["visible"] == true && ring_problem(&last["focus"]).is_none()
                && last["cuts"].as_array().is_some_and(|cuts| cuts.is_empty()), last.to_string());
        let action = tab_until(&app, "!!document.activeElement.closest('.cli-settings-actions')", 6, false).await;
        let footer = js(&app, "return {visible:geo(document.activeElement).inside,focus:focusInfo(),cuts:ringCuts()};").await;
        checks.check(&format!("settings CLI {width}px @{density}%: native Tab reaches footer with unclipped focus"),
            action && footer["visible"] == true && ring_problem(&footer["focus"]).is_none()
                && footer["cuts"].as_array().is_some_and(|cuts| cuts.is_empty()), footer.to_string());
        key(&app, Key::Escape).await;
        checks.check(&format!("settings CLI {width}px @{density}%: native Escape closes clean deep link"),
            truthy(&app, "!document.querySelector('.workspace-settings-dialog') && location.pathname==='/'").await, "clean deep link returns home");
    }
    settings_narrow_menu(&app, checks).await;
    if let Some(window) = app.get_webview_window(MAIN) {
        window.set_size(tauri::LogicalSize::new(800.0, 600.0)).ok();
        window.set_min_size(Some(tauri::LogicalSize::new(800.0, 600.0))).ok();
    }
}
