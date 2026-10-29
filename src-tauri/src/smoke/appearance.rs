//! Real-WebView calibration for the FR-005 Phase 1 tokens (AC01/AC03/AC20 on the real renderer).
//!
//! The page is the production `dist/` (so the compiled Tailwind CSS and the appearance controller are
//! the shipped ones). Only `smoke_report` is registered, so `get_appearance_preferences` is rejected by
//! Tauri as an unknown command: that is the genuine read-failure path (AC21 read side), and the page must
//! come up with the defaults. The harness then sets `data-theme`/`data-density` itself, exactly the two
//! attributes the controller writes, and reads `getComputedStyle`.
//!
//! What this does not prove: persistence (no host command yet), the controller's own switching (covered by
//! tests/appearance.test.tsx), focus-ring visibility (needs eyes), VoiceOver, or any other platform's WebView.

use super::{eval, wait_load, Checks, LoadEvents, Pending, WATCHDOG};
use serde_json::{json, Value};
use std::io::Write;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::broadcast;

const PAIRS: &str = include_str!("../../../tests/fixtures/design-tokens/contrast-pairs.json");

/// Helpers every evaluated body starts with: a probe element, colour parsing and WCAG contrast.
const PRELUDE: &str = r#"
const PAIRS = __PAIRS__;
const root = document.documentElement;
const css = (el, prop) => getComputedStyle(el)[prop];
const probe = (style) => { const el = document.createElement('div'); el.setAttribute('data-smoke-probe', ''); el.style.cssText = style; document.body.appendChild(el); return el; };
const rgb = (text) => { const m = /rgba?\(([^)]+)\)/.exec(text) || /color\(srgb ([^)]+)\)/.exec(text); if (!m) throw new Error('unparsed colour ' + text); const p = m[1].split(/[ ,/]+/).filter(Boolean).slice(0, 3).map(Number); return text.startsWith('color(') ? p.map((v) => Math.round(v * 255)) : p; };
const lum = ([r, g, b]) => { const f = (c) => { c /= 255; return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4); }; return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b); };
const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)]; return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
const px = (text) => parseFloat(text);
"#;

fn script(body: &str) -> String {
    format!("{}{body}", PRELUDE.replace("__PAIRS__", PAIRS))
}

async fn js(app: &AppHandle, body: &str) -> Value {
    let reply = eval(app, "main", &script(body)).await;
    if reply["ok"] == true {
        reply["value"].clone()
    } else {
        json!({"__error": reply["error"].clone()})
    }
}

async fn scenario(app: AppHandle, ready: bool) {
    let mut checks = Checks::default();
    if !ready {
        checks.check("main window loaded /settings", false, "page never loaded");
        finish(checks);
    }

    // The gate lifts once the (failed) read settles; wait for it rather than sleeping.
    let gate = js(
        &app,
        "for (let i = 0; i < 100; i++) { if (!root.hasAttribute('data-appearance-pending')) return true; await new Promise(r => setTimeout(r, 50)); } return false;",
    )
    .await;
    checks.check(
        "first-frame gate lifted after the read failed",
        gate == true,
        gate.to_string(),
    );

    let info = js(
        &app,
        "return { ua: navigator.userAgent, theme: root.getAttribute('data-theme'), density: root.getAttribute('data-density'), \
         children: document.getElementById('root').childElementCount, rootPx: css(root, 'fontSize'), dpr: devicePixelRatio, w: innerWidth, h: innerHeight };",
    )
    .await;
    println!("INFO webview {info}");
    checks.check(
        "unknown host command -> defaults applied (light, 100)",
        info["theme"] == "light" && info["density"] == "100",
        info.to_string(),
    );
    checks.check(
        "React mounted under the gate",
        info["children"].as_u64().unwrap_or(0) > 0,
        info.to_string(),
    );

    // CSS features the compiled Tailwind output relies on.
    let features = js(
        &app,
        "const s = (q) => CSS.supports(q); return { colorMix: s('color: color-mix(in srgb, red 50%, blue)'), colorMixOklab: s('color: color-mix(in oklab, red, blue)'), \
         layers: (() => { const st = document.createElement('style'); st.textContent = '@layer a, b;'; document.head.appendChild(st); const ok = st.sheet.cssRules.length === 1 && st.sheet.cssRules[0].constructor.name === 'CSSLayerStatementRule'; st.remove(); return ok; })(), \
         property: typeof CSS.registerProperty === 'function', focusVisible: s('selector(:focus-visible)'), maxFn: s('font-size: max(1rem, 11px)'), calcVarRem: s('padding: calc(4rem / 14)') };",
    )
    .await;
    println!("INFO css features {features}");
    for key in [
        "colorMix",
        "layers",
        "property",
        "focusVisible",
        "maxFn",
        "calcVarRem",
    ] {
        checks.check(
            &format!("WebView supports {key}"),
            features[key] == true,
            features.to_string(),
        );
    }
    println!(
        "INFO colorMixOklab (used only inside @supports) = {}",
        features["colorMixOklab"]
    );

    // AC01 on the real renderer: body colours equal the token of each theme; the theme actually changes them.
    let themes = js(
        &app,
        "const out = {}; for (const theme of ['light', 'dark']) { root.setAttribute('data-theme', theme); \
         const bg = probe('background-color: var(--color-bg-primary)'); const fg = probe('color: var(--color-text-primary)'); \
         out[theme] = { body: css(document.body, 'backgroundColor'), token: css(bg, 'backgroundColor'), text: css(document.body, 'color'), textToken: css(fg, 'color'), scheme: css(root, 'colorScheme') }; } \
         root.setAttribute('data-theme', 'light'); document.querySelectorAll('[data-smoke-probe]').forEach(e => e.remove()); return out;",
    )
    .await;
    println!("INFO themes {themes}");
    for theme in ["light", "dark"] {
        let t = &themes[theme];
        checks.check(
            &format!("{theme}: body background = --color-bg-primary"),
            t["body"] == t["token"] && t["body"].is_string(),
            t.to_string(),
        );
        checks.check(
            &format!("{theme}: body text = --color-text-primary"),
            t["text"] == t["textToken"] && t["text"].is_string(),
            t.to_string(),
        );
        checks.check(
            &format!("{theme}: color-scheme follows"),
            t["scheme"] == theme,
            t.to_string(),
        );
    }
    checks.check(
        "dark background differs from light",
        themes["dark"]["body"] != themes["light"]["body"],
        themes.to_string(),
    );

    // AC20 on rendered colours: every declared pair, both themes, from what the engine actually paints.
    let contrast = js(
        &app,
        "const bad = []; let n = 0; for (const theme of ['light', 'dark']) { root.setAttribute('data-theme', theme); \
         for (const p of PAIRS.pairs) { const el = probe(`color: var(--color-${p.fg}); background-color: var(--color-${p.bg})`); \
         const r = ratio(rgb(css(el, 'color')), rgb(css(el, 'backgroundColor'))); n++; \
         if (r < (p.kind === 'text' ? 4.5 : 3)) bad.push(`${theme} ${p.fg}/${p.bg}=${r.toFixed(2)}`); } } \
         root.setAttribute('data-theme', 'light'); document.querySelectorAll('[data-smoke-probe]').forEach(e => e.remove()); return { n, bad };",
    )
    .await;
    checks.check(
        &format!(
            "rendered contrast: {} pairs x 2 themes meet WCAG thresholds",
            contrast["n"].as_u64().unwrap_or(0) / 2
        ),
        contrast["bad"] == json!([]) && contrast["n"].as_u64().unwrap_or(0) > 0,
        contrast.to_string(),
    );

    // AC03 and the 14px-root arithmetic (D064, D066): px values at every density.
    let density = js(
        &app,
        "const out = {}; for (const d of [75, 100, 125, 150, 175]) { root.setAttribute('data-density', String(d)); \
         const sp = probe('padding: var(--spacing-2)'); const u = probe('padding: var(--spacing-unit)'); \
         const sizes = {}; for (const s of ['xs', 'sm', 'base', 'lg', 'xl', '2xl', '3xl', '4xl']) { const e = probe(`font-size: var(--font-size-${s})`); sizes[s] = px(css(e, 'fontSize')); } \
         const rad = probe('border-radius: var(--radius-md); width: 10px; height: 10px'); \
         out[d] = { root: px(css(root, 'fontSize')), spacing2: px(css(sp, 'paddingTop')), unit: px(css(u, 'paddingTop')), radiusMd: px(css(rad, 'borderTopLeftRadius')), body: px(css(document.body, 'fontSize')), sizes }; } \
         root.setAttribute('data-density', '100'); document.querySelectorAll('[data-smoke-probe]').forEach(e => e.remove()); return out;",
    )
    .await;
    println!("INFO density {density}");
    let near = |a: &Value, b: f64| a.as_f64().map(|v| (v - b).abs() < 0.05).unwrap_or(false);
    let d100 = &density["100"];
    checks.check(
        "100%: root 14px, spacing unit 4px, p-2 = 8px, radius-md = 8px, body text 14px",
        near(&d100["root"], 14.0)
            && near(&d100["unit"], 4.0)
            && near(&d100["spacing2"], 8.0)
            && near(&d100["radiusMd"], 8.0)
            && near(&d100["body"], 14.0),
        d100.to_string(),
    );
    let want = [
        ("xs", 12.0),
        ("sm", 13.0),
        ("base", 14.0),
        ("lg", 16.0),
        ("xl", 18.0),
        ("2xl", 20.0),
        ("3xl", 24.0),
        ("4xl", 30.0),
    ];
    checks.check(
        "100%: type scale = 12/13/14/16/18/20/24/30 px",
        want.iter().all(|(k, px)| near(&d100["sizes"][*k], *px)),
        d100["sizes"].to_string(),
    );
    for (level, scale) in [("75", 0.75), ("125", 1.25), ("150", 1.5), ("175", 1.75)] {
        let d = &density[level];
        checks.check(
            &format!("{level}%: root and spacing scale by {scale} (interface zoom)"),
            near(&d["root"], 14.0 * scale)
                && near(&d["spacing2"], 8.0 * scale)
                && near(&d["unit"], 4.0 * scale),
            d.to_string(),
        );
    }
    // D064 floor: nothing below 11px at 75%, and still non-decreasing in the scale.
    let floor_ok = ["75", "100", "125", "150", "175"].iter().all(|level| {
        let sizes: Vec<f64> = want
            .iter()
            .map(|(k, _)| density[*level]["sizes"][*k].as_f64().unwrap_or(0.0))
            .collect();
        sizes.iter().all(|s| *s >= 10.99) && sizes.windows(2).all(|w| w[0] <= w[1] + 0.001)
    });
    checks.check(
        "every size token >= 11px at every density, scale stays ordered",
        floor_ok,
        density.to_string(),
    );
    println!("INFO 75% sizes {}", density["75"]["sizes"]);

    // 800x600 is the minimum window; report horizontal overflow per density on the real page. Reading
    // scrollWidth forces layout; requestAnimationFrame is not used because a hidden window never fires it.
    // Asserted only where the legacy (un-migrated) shell is expected to fit; the rest is information (Phase 7/8).
    let overflow = js(
        &app,
        "const out = {}; for (const d of [75, 100, 125, 150, 175]) { root.setAttribute('data-density', String(d)); \
         out[d] = { scrollW: root.scrollWidth, clientW: root.clientWidth, over: root.scrollWidth > root.clientWidth }; } root.setAttribute('data-density', '100'); return out;",
    )
    .await;
    println!("INFO overflow (window as created) {overflow}");
    checks.check(
        "no horizontal overflow of the shell at 100% in the harness window",
        overflow["100"]["over"] == false,
        overflow["100"].to_string(),
    );

    // The gate itself: with the attribute back on, nothing paints and the body is not the light token.
    let gate_css = js(
        &app,
        "root.setAttribute('data-appearance-pending', ''); const v = css(document.body, 'visibility'); const bg = css(document.body, 'backgroundColor'); \
         root.removeAttribute('data-appearance-pending'); return { v, bg };",
    )
    .await;
    checks.check(
        "pending gate hides the body and leaves its background transparent",
        gate_css["v"] == "hidden" && gate_css["bg"] == "rgba(0, 0, 0, 0)",
        gate_css.to_string(),
    );

    finish(checks);
}

fn finish(checks: Checks) -> ! {
    println!(
        "SMOKE RESULT: {} passed, {} failed",
        checks.passed,
        checks.failed.len()
    );
    for line in &checks.failed {
        println!("  FAILED {line}");
    }
    let _ = std::io::stdout().flush();
    std::process::exit(if checks.failed.is_empty() { 0 } else { 1 });
}

pub fn run_appearance() {
    std::thread::spawn(|| {
        std::thread::sleep(WATCHDOG);
        println!("SMOKE RESULT: watchdog expired");
        let _ = std::io::stdout().flush();
        std::process::exit(2);
    });

    let (load_sender, _) = broadcast::channel(64);
    let mut context = tauri::generate_context!();
    // The window is created below so the harness controls size and first URL.
    context.config_mut().app.windows.clear();

    let result = tauri::Builder::default()
        .manage(Pending::default())
        .manage(LoadEvents(load_sender))
        .on_page_load(|webview, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                let _ = webview
                    .app_handle()
                    .state::<LoadEvents>()
                    .0
                    .send((webview.label().to_owned(), payload.url().path().to_owned()));
            }
        })
        .setup(move |app| {
            let events = app.state::<LoadEvents>().0.subscribe();
            // 800x600: the minimum window size in tauri.conf.json.
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("settings".into()))
                .inner_size(800.0, 600.0)
                .visible(false)
                .build()?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let ready = wait_load(events, "main", "/settings").await;
                scenario(handle, ready).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![super::smoke_report])
        .run(context);
    if let Err(error) = result {
        println!("SMOKE RESULT: tauri failed to run: {error}");
    }
    std::process::exit(3);
}
