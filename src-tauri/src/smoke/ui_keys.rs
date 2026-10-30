//! FR-005 Phase 8: physical-keyboard and overlay scenarios in the visible WKWebView.
//!
//! Keys are real `NSEvent`s (keyDown + keyUp) delivered with `-[NSWindow sendEvent:]` to the WKWebView, so
//! WebKit runs its own default actions: Tab focus traversal, `:focus-visible`, Space/Enter activation of
//! buttons. Nothing here dispatches DOM `KeyboardEvent`s or calls `.click()`/`.focus()` as a substitute for a
//! key press, except where a step says so (starting a scenario from a known control).
//!
//! The page set is the shipped one plus `tests/fixtures/modules/p8-gallery.js`, a fixture module rendering
//! Tabs, RadioGroup, Select, Switch, Checkbox, Tooltip, a Modal with a Select and a Tooltip inside, and a Toast
//! trigger through the frozen `host.ui` (no shipped page nests those).
//!
//! Not proven here: pointer clicks (hit-testing uses `elementFromPoint`, not a mouse event), VoiceOver, any
//! other platform, reduced-motion behaviour.

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
}

impl Key {
    /// (macOS virtual key code, characters, modifier flags)
    fn spec(self) -> (u16, &'static str, usize) {
        const SHIFT: usize = 1 << 17;
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
        }
    }
}

#[cfg(target_os = "macos")]
fn post(app: &AppHandle, key: Key, down: bool) -> bool {
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
                charactersIgnoringModifiers: text,
                isARepeat: Bool::NO,
                keyCode: code
            ];
            let _: () = msg_send![win, sendEvent: event];
        })
        .is_ok()
}

#[cfg(not(target_os = "macos"))]
fn post(_: &AppHandle, _: Key, _: bool) -> bool {
    false
}

/// Activate the application and make the window key (the app may be launched in the background).
#[cfg(target_os = "macos")]
fn activate(app: &AppHandle) -> bool {
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{class, msg_send};
    let Some(window) = app.get_webview_window(MAIN) else {
        return false;
    };
    let _ = window.set_focus();
    window
        .with_webview(|webview| unsafe {
            let ns_app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![ns_app, activateIgnoringOtherApps: Bool::YES];
            let win = webview.ns_window() as *mut AnyObject;
            let _: () = msg_send![win, makeKeyAndOrderFront: std::ptr::null_mut::<AnyObject>()];
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
async fn prepare(app: &AppHandle) -> bool {
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
        println!("INFO the page never reported document.hasFocus() after 12 activation attempts");
    }
    window
        .with_webview(|webview| unsafe {
            let win = webview.ns_window() as *mut AnyObject;
            let view = webview.inner() as *mut AnyObject;
            let _: Bool = msg_send![win, makeFirstResponder: view];
        })
        .is_ok()
}

#[cfg(not(target_os = "macos"))]
async fn prepare(_: &AppHandle) -> bool {
    false
}

async fn key(app: &AppHandle, key: Key) {
    // Re-asserts key status every time: an occluded or non-key window silently drops key events and
    // freezes animations.
    activate(app);
    post(app, key, true);
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
  const cs = getComputedStyle(el); const ring = tokenColor('--color-focus-ring'); const oc = parse(cs.outlineColor); const bg = bgOf(el);
  return { body: false, hasFocus: document.hasFocus(), tag: el.tagName.toLowerCase(), role: el.getAttribute('role'), label: lbl(el), id: el.id,
    index: tabbables().indexOf(el), fv: el.matches(':focus-visible'), outlineW: parseFloat(cs.outlineWidth), outlineStyle: cs.outlineStyle,
    outlineIsToken: !!oc && oc.c.join() === ring.join(), contrast: oc ? Math.round(ratio(oc.c, parseFloat(cs.outlineOffset) >= 0 ? bgOf(el.parentElement || el) : bg) * 100) / 100 : null, ownContrast: oc ? Math.round(ratio(oc.c, bg) * 100) / 100 : null, offset: cs.outlineOffset };
};
const geo = (el) => { const r = el.getBoundingClientRect(); return { l: Math.round(r.left), t: Math.round(r.top), r: Math.round(r.right), b: Math.round(r.bottom), w: Math.round(r.width), h: Math.round(r.height), vw: root.clientWidth, vh: innerHeight, inside: r.left >= -0.5 && r.top >= -0.5 && r.right <= root.clientWidth + 0.5 && r.bottom <= innerHeight + 0.5 }; };
const zOf = (el) => { let z = 0; for (let n = el; n && n !== document.documentElement; n = n.parentElement) { const v = parseInt(css(n, 'zIndex'), 10); if (!Number.isNaN(v)) z = Math.max(z, v); } return z; };
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
        && info["outlineStyle"] != "none"
        && info["outlineW"].as_f64().unwrap_or(0.0) >= 2.0
        && info["outlineIsToken"] == true
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
        "Home: each focused control has a visible :focus-visible ring (>=2px, --color-focus-ring, >=3:1 on its background)",
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

    // Space and Enter activate the focused button (physical keys, not .click()).
    js(
        app,
        "window.__p8greet = 0; document.addEventListener('click', (e) => { const b = e.target.closest && e.target.closest('button'); if (e.isTrusted && e.detail === 0 && b && lbl(b) === 'Greet') window.__p8greet++; }, true); return true;",
    )
    .await;
    let found = tab_until(app, "lbl(document.activeElement) === 'Greet'", n + 2, false).await;
    checks.check("Home: Tab reaches the Greet button", found, "not reached");
    if found {
        key(app, Key::Space).await;
        let after_space = js(app, "return window.__p8greet;").await;
        key(app, Key::Enter).await;
        let after_enter = js(app, "return window.__p8greet;").await;
        sleep(Duration::from_millis(400)).await;
        let toasts = js(app, "return [...document.querySelectorAll('[data-toast-type]')].map((t) => ({ type: t.dataset.toastType, role: t.getAttribute('role'), live: t.getAttribute('aria-live') }));").await;
        checks.check(
            "Home: Space activates the focused button once",
            after_space == 1,
            after_space.to_string(),
        );
        checks.check(
            "Home: Enter activates the focused button once",
            after_enter == 2,
            after_enter.to_string(),
        );
        checks.check(
            "Home: the unregistered `greet` command surfaces an error Toast with role=alert/aria-live=assertive",
            toasts.as_array().is_some_and(|t| !t.is_empty() && t.iter().all(|x| x["type"] == "error" && x["role"] == "alert" && x["live"] == "assertive")),
            toasts.to_string(),
        );
    }
}

async fn skills_tabs(app: &AppHandle, checks: &mut Checks) {
    if !ready(app, "/skills").await {
        checks.check("Skills: loaded", false, "page never loaded");
        return;
    }
    let found = tab_until(
        app,
        "document.activeElement.getAttribute('role') === 'tab'",
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
    let state = "const t = document.activeElement; return { role: t.getAttribute('role'), sel: t.getAttribute('aria-selected'), label: lbl(t), roving: [...document.querySelectorAll('[role=tab]')].filter((x) => x.tabIndex === 0).length };";
    let s = js(app, state).await;
    checks.check("Skills: Tab lands on the selected tab; exactly one tab is in the tab order (roving tabindex)", s["sel"] == "true" && s["roving"] == 1, s.to_string());
    for (k, name, want) in [
        (Key::Right, "Right", "Installed"),
        (Key::Left, "Left", "Browse"),
        (Key::End, "End", "Installed"),
        (Key::Home, "Home", "Browse"),
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
    if tab_until(
        app,
        "document.activeElement.getAttribute('role') === 'tab'",
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
    // A real render error: put an object into the greeting state (React throws "Objects are not valid as a
    // React child" while rendering), found through the fiber that owns the "Welcome" heading.
    let injected = js(app, "const h1 = [...document.querySelectorAll('h1')].find((e) => e.textContent.includes('Welcome')); const fk = Object.keys(h1).find((k) => k.startsWith('__reactFiber$')); let f = h1[fk]; while (f && !(f.tag === 0 && f.memoizedState)) f = f.return; let hook = f && f.memoizedState; while (hook && !(hook.queue && hook.memoizedState === '')) hook = hook.next; if (!hook) return false; hook.queue.dispatch({ not: 'a child' }); return true;").await;
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
        let back = js(app, "return !!(await until(() => !document.querySelector('[data-error-kind=error]') && [...document.querySelectorAll('h1')].some((e) => e.textContent.includes('Welcome')), 4000));").await == true;
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
    for density in [75, 150, 175] {
        density_geometry(app, checks, density).await;
    }
    gallery_themes(app, checks).await;
    set_appearance(app, "dark", 100).await;
    ring_walk(app, checks, "Dark theme").await;
    set_appearance(app, "light", 100).await;
    error_boundary(app, checks).await;
    appearance_e2e(app, checks).await;
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
        .0["value"]["data"]
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

/// V1-E2E-05 (switch theme -> switch density -> show a Toast) through the real Settings > Appearance page.
/// Each step is one physical key press on a control that was focused beforehand: Enter on the link and on the
/// button, Right on a radio group. Outcomes are read from the DOM and from the real host command.
/// Setup shortcuts that are not key presses: the preferences are reset over IPC and each starting control is
/// focused with `.focus()`.
async fn appearance_e2e(app: &AppHandle, checks: &mut Checks) {
    let reset = set_appearance(app, "light", 100).await;
    checks.check(
        "E2E-05: host reset to light/100",
        reset,
        "set_appearance_preferences failed",
    );
    if !ready(app, "/settings").await {
        checks.check("E2E-05: /settings loaded", false, "page never loaded");
        return;
    }

    let link = "const a = [...document.querySelectorAll('a')].find((x) => x.textContent.trim() === 'Appearance'); if (!a) return false; a.focus(); return document.activeElement === a;";
    let focused = js(app, link).await == true;
    if focused {
        key(app, Key::Enter).await;
    }
    let opened = js(
        app,
        "return !!(await until(() => location.pathname === '/settings/appearance' && [...document.querySelectorAll('h1')].some((h) => h.textContent.trim() === '外观设置')));",
    )
    .await
        == true;
    checks.check(
        "E2E-05: Enter on the Appearance link on /settings opens /settings/appearance",
        focused && opened,
        format!("focused={focused} opened={opened}"),
    );
    if !opened {
        return;
    }

    let base = js(
        app,
        "const bg = bgOf(document.body); window.__e2e = { lum: lum(bg), font: parseFloat(css(root, 'fontSize')) }; return { theme: root.dataset.theme, density: root.dataset.density, lum: window.__e2e.lum, font: window.__e2e.font, themeChecked: [...document.querySelectorAll('[role=\"radio\"]')].filter((r) => r.getAttribute('aria-checked') === 'true').map((r) => lbl(r)) };",
    )
    .await;
    checks.check(
        "E2E-05: the page starts on the host values (light, 100%)",
        base["theme"] == "light"
            && base["density"] == "100"
            && base["themeChecked"] == json!(["亮色", "100%"]),
        base.to_string(),
    );

    // Step 1: theme. Right on the focused "亮色" radio moves the choice to "暗色".
    let on_light = js(app, &focus_radio("亮色")).await == true;
    key(app, Key::Right).await;
    let dark = js(
        app,
        "const ok = await until(() => root.dataset.theme === 'dark'); const r = [...document.querySelectorAll('[role=\"radio\"]')].find((x) => lbl(x) === '暗色'); return { ok: !!ok, checked: !!r && r.getAttribute('aria-checked') === 'true', lum: lum(bgOf(document.body)), base: window.__e2e.lum };",
    )
    .await;
    let dark_ok = on_light
        && dark["ok"] == true
        && dark["checked"] == true
        && dark["lum"].as_f64().unwrap_or(1.0) < 0.2
        && dark["base"].as_f64().unwrap_or(0.0) > 0.5;
    checks.check(
        "E2E-05 step 1: Right on the theme group applies dark (data-theme, checked radio, body background darker)",
        dark_ok,
        format!("on_light={on_light} {dark}"),
    );
    checks.check(
        "E2E-05 step 1: the host saved theme=dark",
        host_has(app, "theme", json!("dark")).await,
        host_appearance(app).await.to_string(),
    );

    // Step 2: interface scale. Two Rights on the focused "100%" radio reach "150%".
    let on_100 = js(app, &focus_radio("100%")).await == true;
    key(app, Key::Right).await;
    key(app, Key::Right).await;
    let scale = js(
        app,
        "const ok = await until(() => root.dataset.density === '150'); const r = [...document.querySelectorAll('[role=\"radio\"]')].find((x) => lbl(x) === '150%'); return { ok: !!ok, checked: !!r && r.getAttribute('aria-checked') === 'true', font: parseFloat(css(root, 'fontSize')), base: window.__e2e.font, overflowX: root.scrollWidth > root.clientWidth };",
    )
    .await;
    let ratio_ok = scale["font"]
        .as_f64()
        .zip(scale["base"].as_f64())
        .is_some_and(|(f, b)| b > 0.0 && (f / b - 1.5).abs() < 0.01);
    checks.check(
        "E2E-05 step 2: two Rights on the scale group apply 150% (data-density, checked radio, root font 1.5x, no horizontal overflow)",
        on_100 && scale["ok"] == true && scale["checked"] == true && ratio_ok && scale["overflowX"] == false,
        format!("on_100={on_100} {scale}"),
    );
    checks.check(
        "E2E-05 step 2: the host saved density=150 and still has theme=dark",
        host_has(app, "density", json!(150)).await && host_appearance(app).await["theme"] == "dark",
        host_appearance(app).await.to_string(),
    );

    // Step 3: Toast. Enter on the preview button.
    let on_button = js(
        app,
        "const b = [...document.querySelectorAll('button')].find((x) => x.textContent.trim() === '显示通知'); if (!b) return false; b.focus(); return document.activeElement === b;",
    )
    .await
        == true;
    if on_button {
        key(app, Key::Enter).await;
    }
    let toast = js(
        app,
        "const t = await until(() => [...document.querySelectorAll('[role=\"status\"]')].find((x) => x.textContent.includes('这是一条示例通知'))); return t ? { role: t.getAttribute('role'), live: t.getAttribute('aria-live'), geo: geo(t) } : null;",
    )
    .await;
    checks.check(
        "E2E-05 step 3: Enter on the preview button shows an info Toast (role=status, polite) inside the viewport",
        on_button && toast["role"] == "status" && toast["live"] == "polite" && toast["geo"]["inside"] == true,
        format!("on_button={on_button} {toast}"),
    );

    // The choices survive a reload: they come back from the host, not from the page.
    let reloaded = ready(app, "/settings/appearance").await;
    let again = js(
        app,
        "return { theme: root.dataset.theme, density: root.dataset.density, checked: [...document.querySelectorAll('[role=\"radio\"]')].filter((r) => r.getAttribute('aria-checked') === 'true').map((r) => lbl(r)) };",
    )
    .await;
    checks.check(
        "E2E-05: after a reload the page shows dark/150% from the host",
        reloaded
            && again["theme"] == "dark"
            && again["density"] == "150"
            && again["checked"] == json!(["暗色", "150%"]),
        again.to_string(),
    );

    // AC21 on the real renderer and host: a failing save rolls the page back and raises one error Toast.
    // The fault is a SQLite trigger on the second connection; the page and host code are the shipped ones.
    let probe = &app.state::<UiProbe>().0;
    let armed = probe
        .execute_unprepared(
            "CREATE TRIGGER fail_theme BEFORE UPDATE ON system_settings WHEN NEW.setting_key='appearance.theme' \
             BEGIN SELECT RAISE(ABORT, 'scripted failure'); END;",
        )
        .await
        .is_ok();
    checks.check(
        "E2E-05 failure: fault injected (theme updates abort)",
        armed,
        "trigger not created",
    );
    let on_dark = js(app, &focus_radio("暗色")).await == true;
    key(app, Key::Right).await;
    let failed = js(
        app,
        "const t = await until(() => [...document.querySelectorAll('[role=\"alert\"]')].find((x) => x.textContent.includes('操作失败'))); await wait(300); const r = (n) => [...document.querySelectorAll('[role=\"radio\"]')].find((x) => lbl(x) === n); return { toast: !!t, live: t ? t.getAttribute('aria-live') : null, alerts: [...document.querySelectorAll('[role=\"alert\"]')].filter((x) => x.textContent.includes('操作失败')).length, dark: r('暗色').getAttribute('aria-checked'), auto: r('跟随系统').getAttribute('aria-checked'), theme: root.dataset.theme, leaked: document.body.innerText.includes('scripted') || document.body.innerText.includes('system_settings') };",
    )
    .await;
    checks.check(
        "E2E-05 failure: a failed save rolls the page back to dark, raises exactly one assertive error Toast and leaks no driver text",
        on_dark
            && failed["toast"] == true
            && failed["live"] == "assertive"
            && failed["alerts"] == 1
            && failed["dark"] == "true"
            && failed["auto"] == "false"
            && failed["theme"] == "dark"
            && failed["leaked"] == false,
        format!("on_dark={on_dark} {failed}"),
    );
    checks.check(
        "E2E-05 failure: the host still has theme=dark",
        host_appearance(app).await["theme"] == "dark",
        host_appearance(app).await.to_string(),
    );
    let _ = probe.execute_unprepared("DROP TRIGGER fail_theme").await;
    set_appearance(app, "light", 100).await;
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
const dlg = () => document.querySelector('[role="dialog"]');
const dlgTitle = () => { const d = dlg(); const t = d && document.getElementById(d.getAttribute('aria-labelledby')); return t ? t.textContent.trim() : null; };
const dlgState = () => { const d = dlg(); return d ? { title: dlgTitle(), inside: d.contains(document.activeElement), focus: lbl(document.activeElement), buttons: [...d.querySelectorAll('button')].map((b) => lbl(b)) } : null; };
const nameInput = () => { const l = [...document.querySelectorAll('label')].find((x) => x.textContent.trim().startsWith('Name')); return l && l.control; };
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
        "(() => {{ const d = document.querySelector('[role=\"dialog\"]'); return !!d && d.contains(document.activeElement) && lbl(document.activeElement) === {label:?}; }})()"
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

    let delete_trigger = "const a = await until(() => [...document.querySelectorAll('article')].find((x) => x.textContent.includes('confirm-probe'))); const b = a && [...a.querySelectorAll('button')].find((x) => x.textContent.trim() === 'Delete'); if (!b) return false; b.focus(); return document.activeElement === b;";
    let title = "Delete MCP server?";

    if open_expect(app, checks, "MCP delete", delete_trigger, title).await {
        key(app, Key::Escape).await;
        checks.check(
            "MCP delete: Esc closes the dialog, keeps the server and returns focus to Delete",
            dialog_gone(app).await
                && mcp_has(app, &id).await
                && truthy(app, "document.activeElement && document.activeElement.textContent.trim() === 'Delete'").await,
            "dialog open, server missing or focus not restored",
        );
    }
    if open_expect(app, checks, "MCP delete (Cancel)", delete_trigger, title).await {
        let on_cancel = tab_to_dialog_button(app, "Cancel").await;
        key(app, Key::Enter).await;
        checks.check(
            "MCP delete: Enter on Cancel closes the dialog and keeps the server",
            on_cancel && dialog_gone(app).await && mcp_has(app, &id).await,
            "cancel did not close, or the server was deleted",
        );
    }
    if open_expect(app, checks, "MCP delete (Delete)", delete_trigger, title).await {
        let on_delete = tab_to_dialog_button(app, "Delete").await;
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
    let dirty = "const i = nameInput(); if (!i) return false; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(i, 'Unsaved draft'); i.dispatchEvent(new Event('input', { bubbles: true })); return !!(await until(() => document.querySelector('[aria-label=\"Unsaved changes\"]')));";
    let new_trigger = "const b = [...document.querySelectorAll('button')].find((x) => x.textContent.trim() === '+ New Provider'); if (!b) return false; b.focus(); return document.activeElement === b;";
    let discard = "Discard unsaved changes?";
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
                    && jd(app, "return !!(nameInput() && nameInput().value === 'Unsaved draft' && ![...document.querySelectorAll('h2')].some((h) => h.textContent.trim() === 'New Provider'));").await == true,
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
            let on_discard = tab_to_dialog_button(app, "Discard").await;
            key(app, Key::Enter).await;
            let closed = dialog_gone(app).await;
            let fresh = jd(app, "return !!(await until(() => [...document.querySelectorAll('h2')].some((h) => h.textContent.trim() === 'New Provider') && nameInput() && nameInput().value === ''));").await == true;
            checks.check(
                "Provider discard: Enter on Discard closes the dialog and opens the empty new-provider form",
                on_discard && closed && fresh,
                format!("onDiscard={on_discard} closed={closed} fresh={fresh}"),
            );
        }
    }

    let delete_form = "const b = await until(() => [...document.querySelectorAll('footer button')].find((x) => x.textContent.includes('Delete'))); if (!b) return false; b.focus(); return document.activeElement === b;";
    let delete_title = "Delete Provider?";
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
        let on_delete = tab_to_dialog_button(app, "Delete").await;
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
