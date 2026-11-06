#!/usr/bin/env node
import { spawn, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { access, mkdir, mkdtemp, readFile, writeFile, rm, copyFile, rename } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { cases, comparison } from '../tools/ui-preview/visual/cases.mjs';
import { installProbe, inspectControls, inspectSelect, inspectModal, inspectSettings, inspectChat } from '../tools/ui-preview/visual/probes.mjs';

// No browser download or automation framework: Node 22 WebSocket speaks Chrome DevTools Protocol.
// Start preview separately: npm run dev:ui. Record does not change the accepted baseline.
const help = `Usage: node scripts/ui-preview-visual.mjs [--record | --accept | --prove-detection]
  default             Compare all five real production surfaces against baseline (never update).
  --record            Write candidate PNGs + environment manifest; review all five images.
  --accept            Explicitly promote reviewed candidate; fail if environment has drifted.
  --prove-detection   Inject oversized real Modal geometry; save before/actual/diff + detection evidence.
Environment: UI_PREVIEW_CHROME=/absolute/path/to/Chrome (default: system Google Chrome).
Server: http://127.0.0.1:5197/frame.html. Node >=22. Required dev deps: pngjs@7.0.0 pixelmatch@7.1.0.
Exit: 0 pass/recorded/accepted/proven; 1 visual or geometry failure; 2 environment drift;
      3 missing baseline/candidate/dependency/browser/server, invalid arguments, or runner failure.
`;
const args = process.argv.slice(2);
if (args.includes('--help')) { console.log(help); process.exit(0); }
if (args.length > 1 || args.some(arg => !['--record', '--accept', '--prove-detection'].includes(arg))) { console.error(help); process.exit(3); }
const mode = args[0] ?? 'compare';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const assets = path.join(root, 'tools/ui-preview/visual');
const origin = 'http://127.0.0.1:5197';
const clock = 1793966400000;
const baseline = path.join(assets, 'baseline');
const candidate = path.join(assets, 'candidate');
const output = path.join(assets, 'results');
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const stable = value => JSON.stringify(value);
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
class RunnerError extends Error { constructor(message, code = 3) { super(message); this.code = code; } }
const json = async file => JSON.parse(await readFile(file, 'utf8'));
const saveJSON = (file, value) => writeFile(file, `${JSON.stringify(value, null, 2)}\n`);
async function fileHash(file) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest('hex');
}
async function exists(file) { try { await access(file); return true; } catch { return false; } }
async function waitFor(predicate, label, timeout = 15000) {
  const start = performance.now();
  while (performance.now() - start < timeout) {
    const result = await predicate();
    if (result) return result;
    await sleep(50);
  }
  throw new RunnerError(`Timed out waiting for ${label}`);
}

class CDP {
  constructor(socket) {
    this.socket = socket; this.next = 1; this.pending = new Map(); this.listeners = new Map();
    socket.addEventListener('message', event => {
      const message = JSON.parse(event.data);
      if (message.id) {
        const pending = this.pending.get(message.id);
        if (!pending) return;
        this.pending.delete(message.id); clearTimeout(pending.timer);
        if (message.error) pending.reject(new RunnerError(`CDP ${pending.method}: ${message.error.message}`));
        else pending.resolve(message.result);
      } else for (const handler of this.listeners.get(message.method) ?? []) handler(message.params);
    });
    socket.addEventListener('close', () => {
      for (const pending of this.pending.values()) { clearTimeout(pending.timer); pending.reject(new RunnerError('Chrome CDP connection closed')); }
      this.pending.clear();
    });
  }
  static async connect(url) {
    const socket = new WebSocket(url);
    await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, { once: true }); socket.addEventListener('error', reject, { once: true }); });
    return new CDP(socket);
  }
  on(method, handler) { this.listeners.set(method, [...(this.listeners.get(method) ?? []), handler]); }
  call(method, params = {}) {
    return new Promise((resolve, reject) => {
      const id = this.next++;
      const timer = setTimeout(() => { this.pending.delete(id); reject(new RunnerError(`CDP timeout: ${method}`)); }, 20000);
      this.pending.set(id, { resolve, reject, timer, method }); this.socket.send(JSON.stringify({ id, method, params }));
    });
  }
  async evaluate(expression) {
    const response = await this.call('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (response.exceptionDetails) throw new Error(response.exceptionDetails.exception?.description ?? response.exceptionDetails.text);
    return response.result.value;
  }
  run(fn, ...params) { return this.evaluate(`(${fn.toString()})(...${JSON.stringify(params)})`); }
  close() { this.socket.close(); }
}

async function browserBinary() {
  const choices = [process.env.UI_PREVIEW_CHROME, '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', '/usr/bin/google-chrome', '/usr/bin/chromium'].filter(Boolean);
  for (const choice of choices) if (await exists(choice)) return path.resolve(choice);
  throw new RunnerError('Chrome unavailable. Set UI_PREVIEW_CHROME to an installed Chrome executable.');
}
async function launch(binary) {
  const profile = await mkdtemp(path.join(os.tmpdir(), 'dgos-ui-visual-'));
  const flags = ['--headless=new', '--remote-debugging-port=0', '--remote-debugging-address=127.0.0.1', `--user-data-dir=${profile}`, '--no-first-run', '--no-default-browser-check', '--disable-background-networking', '--disable-component-update', '--disable-sync', '--disable-extensions', '--hide-scrollbars', '--force-device-scale-factor=1', '--force-color-profile=srgb', '--lang=en-US', '--window-size=1280,900', 'about:blank'];
  const process = spawn(binary, flags, { stdio: ['ignore', 'ignore', 'pipe'] });
  let stderr = '', launchError;
  process.stderr.on('data', chunk => { stderr = (stderr + chunk.toString()).slice(-12000); });
  process.on('error', error => { launchError = error; });
  try {
    const activePort = path.join(profile, 'DevToolsActivePort');
    await waitFor(async () => {
      if (launchError) throw launchError;
      if (process.exitCode !== null) throw new RunnerError(`Chrome exited: ${stderr}`);
      return exists(activePort);
    }, 'Chrome debugging port');
    const [port] = (await readFile(activePort, 'utf8')).trim().split('\n');
    const targets = await fetch(`http://127.0.0.1:${port}/json/list`).then(response => response.json());
    const target = targets.find(entry => entry.type === 'page');
    if (!target) throw new RunnerError('Chrome did not expose a page target');
    const cdp = await CDP.connect(target.webSocketDebuggerUrl);
    return { cdp, flags: flags.filter(flag => !flag.startsWith('--user-data-dir=')), async close() { cdp.close(); process.kill(); await new Promise(resolve => { if (process.exitCode !== null) resolve(); else { process.once('exit', resolve); setTimeout(() => { process.kill('SIGKILL'); resolve(); }, 3000).unref(); } }); await rm(profile, { recursive: true, force: true }); } };
  } catch (error) { process.kill(); await rm(profile, { recursive: true, force: true }); throw error; }
}

async function environment(cdp, binary, flags) {
  const version = await cdp.call('Browser.getVersion');
  const fonts = await cdp.evaluate(`(() => {
    const samples = ['AaBb0123456789 !@# The quick brown fox', '中文预览设置审批输入框，。'];
    const families = ['-apple-system', 'BlinkMacSystemFont', 'sans-serif', 'monospace', '"SF Mono"', 'Monaco'];
    return families.map(family => {
      const canvas = document.createElement('canvas'); canvas.width = 900; canvas.height = 110;
      const ctx = canvas.getContext('2d'); ctx.fillStyle = '#fff'; ctx.fillRect(0,0,900,110); ctx.fillStyle = '#000'; ctx.font = '20px ' + family;
      samples.forEach((text,index) => ctx.fillText(text, 5, 35 + 40 * index));
      return {family, widths:samples.map(text=>ctx.measureText(text).width), raster:canvas.toDataURL()};
    });
  })()`);
  return {
    schema: 1, browser: { product: version.product, revision: version.revision, jsVersion: version.jsVersion, sha256: await fileHash(binary) },
    os: { platform: os.platform(), arch: os.arch(), release: os.release(), version: os.version(), productVersion: os.platform() === 'darwin' ? execFileSync('sw_vers', ['-productVersion'], { encoding: 'utf8' }).trim() : os.release() },
    node: process.version, flags, deviceScaleFactor: 1, timezone: 'UTC', locale: 'en-US', colorProfile: 'srgb',
    fonts: fonts.map(({ raster, ...font }) => ({ ...font, rasterSha256: digest(raster) })),
    clock, comparison, cases,
    imageTools: { pngjsSha256: await fileHash(fileURLToPath(import.meta.resolve('pngjs'))), pixelmatchSha256: await fileHash(fileURLToPath(import.meta.resolve('pixelmatch'))) },
  };
}
function assertEnvironment(expected, actual) {
  if (stable(expected) !== stable(actual)) throw new RunnerError('ENVIRONMENT_DRIFT: browser binary/version, OS, font raster, Node, viewport matrix or capture settings differ. Do not re-accept across environments; record and review a new candidate.', 2);
}
async function key(cdp, name, modifiers = 0) {
  const codes = { Tab: 9, Enter: 13, Escape: 27, End: 35, Home: 36, ArrowDown: 40 };
  await cdp.call('Input.dispatchKeyEvent', { type: 'keyDown', key: name, code: name, windowsVirtualKeyCode: codes[name], modifiers, ...(name === 'Enter' ? { text: '\r', unmodifiedText: '\r' } : {}) });
  await cdp.call('Input.dispatchKeyEvent', { type: 'keyUp', key: name, code: name, windowsVirtualKeyCode: codes[name], modifiers });
  await settle(cdp);
}
async function settle(cdp) { await cdp.evaluate('new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))'); }
async function focus(cdp, selector) { await cdp.evaluate(`document.querySelector(${JSON.stringify(selector)})?.focus()`); await settle(cdp); }
async function screenshot(cdp, file) {
  const { data } = await cdp.call('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false, fromSurface: true });
  await writeFile(file, Buffer.from(data, 'base64'));
}
async function openCase(cdp, scene) {
  await cdp.call('Emulation.setDeviceMetricsOverride', { width: scene.width, height: scene.height, deviceScaleFactor: 1, mobile: false });
  await cdp.call('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }, { name: 'prefers-color-scheme', value: scene.theme }] });
  const params = new URLSearchParams(Object.fromEntries(Object.entries(scene).filter(([key]) => !['id', 'width', 'height'].includes(key)).map(([key, value]) => [key, String(value)])));
  await cdp.call('Page.navigate', { url: 'about:blank' });
  await waitFor(() => cdp.evaluate('location.href === "about:blank" && document.readyState === "complete"'), 'clean navigation');
  const destination = `${origin}/frame.html?${params}`;
  const navigation = await cdp.call('Page.navigate', { url: destination });
  if (navigation.errorText) throw new RunnerError(`Preview navigation failed: ${navigation.errorText}`);
  await waitFor(async () => { try { return await cdp.evaluate(`location.href === ${JSON.stringify(destination)} && window.__preview?.scenario === ${JSON.stringify(scene.scenario)} && window.__preview?.ready === true`); } catch { return false; } }, `${scene.id} ready`);
  await cdp.evaluate(`(async () => {
    await document.fonts.ready;
    const style = document.createElement('style');
    style.textContent = '*,*::before,*::after { animation:none !important; transition:none !important; caret-color:transparent !important; scroll-behavior:auto !important; }';
    document.head.append(style);
    if (document.fonts.status !== 'loaded') throw new Error('Fonts have not loaded');
    if (devicePixelRatio !== 1) throw new Error('Expected DPR 1');
    if (window.__preview.clock !== ${clock} || Date.now() !== ${clock}) throw new Error('Preview data clock is not frozen');
  })()`);
  await settle(cdp);
  await cdp.run(installProbe);
}
async function prepare(cdp, scene) {
  switch (scene.scenario) {
    case 'controls': {
      const inputs = await cdp.run(inspectControls, scene);
      await focus(cdp, '#preview-select-long');
      await key(cdp, 'ArrowDown');
      await waitFor(() => cdp.evaluate('!!document.querySelector(\'[role="listbox"]\')'), 'Select menu');
      const select = await cdp.run(inspectSelect);
      await key(cdp, 'End');
      const lastFocused = await cdp.evaluate('document.activeElement?.getAttribute("role") === "option"');
      if (!lastFocused) throw new Error('Select End key did not focus an option');
      return { ...inputs, select };
    }
    case 'modal': {
      for (let i = 0; i < 12; i++) { await key(cdp, 'Tab'); await cdp.run(inspectModal); }
      return cdp.run(inspectModal);
    }
    case 'settings': {
      await focus(cdp, '.workspace-settings-category');
      await key(cdp, 'End');
      const evidence = await cdp.run(inspectSettings);
      return evidence;
    }
    case 'chat': {
      await focus(cdp, '.chat-composer textarea');
      await cdp.call('Input.insertText', { text: 'Preview keyboard input 中文' });
      await key(cdp, 'Enter', 8);
      return cdp.run(inspectChat, false);
    }
    case 'chat-approval': {
      await focus(cdp, '.chat-approval-decisions button');
      await key(cdp, 'Tab');
      return cdp.run(inspectChat, true);
    }
    default: throw new RunnerError(`No probe for ${scene.scenario}`);
  }
}
async function afterCapture(cdp, scene) {
  if (scene.scenario === 'controls') {
    await key(cdp, 'Escape');
    if (!await cdp.evaluate('!document.querySelector(\'[role="listbox"]\') && document.activeElement?.id === "preview-select-long"')) throw new Error('Select Escape did not close and restore trigger focus');
  }
  if (scene.scenario === 'chat-approval') {
    await key(cdp, 'Enter');
    try { await waitFor(() => cdp.evaluate('!document.querySelector(".chat-approval")'), 'keyboard approval decision'); }
    catch (error) {
      const diagnostic = await cdp.evaluate(`({ active: document.activeElement?.outerHTML, preview: window.__preview, approval: document.querySelector('.chat-approval')?.outerHTML })`);
      await saveJSON(path.join(output, 'approval-timeout.json'), diagnostic);
      throw new Error(`${error.message}; diagnostics: ${JSON.stringify(diagnostic)}`);
    }
  }
  const diagnostic = await cdp.evaluate('({blocked:window.__preview.blocked, errors:window.__preview.errors})');
  if (Array.isArray(diagnostic.blocked) && diagnostic.blocked.length) throw new Error(`Unexpected IPC fixture calls: ${JSON.stringify(diagnostic.blocked)}`);
}

async function dependencies() {
  try { const [{ PNG }, { default: pixelmatch }] = await Promise.all([import('pngjs'), import('pixelmatch')]); return { PNG, pixelmatch }; }
  catch { throw new RunnerError('Install exact dev dependencies pngjs@7.0.0 and pixelmatch@7.1.0 before visual capture.'); }
}
async function diffImages(expectedFile, actualFile, diffFile, { PNG, pixelmatch }) {
  const expected = PNG.sync.read(await readFile(expectedFile));
  const actual = PNG.sync.read(await readFile(actualFile));
  const width = Math.max(expected.width, actual.width), height = Math.max(expected.height, actual.height);
  const pad = image => { if (image.width === width && image.height === height) return image; const padded = new PNG({ width, height }); PNG.bitblt(image, padded, 0, 0, image.width, image.height, 0, 0); return padded; };
  const a = pad(expected), b = pad(actual), diff = new PNG({ width, height });
  const different = pixelmatch(a.data, b.data, diff.data, width, height, { threshold: comparison.threshold, includeAA: comparison.includeAA });
  await writeFile(diffFile, PNG.sync.write(diff));
  const dimensionsMatch = expected.width === actual.width && expected.height === actual.height;
  return { different, total: width * height, ratio: different / (width * height), dimensionsMatch, pass: dimensionsMatch && different / (width * height) <= comparison.maxDifferentRatio };
}
async function prove(cdp, deps) {
  const scene = cases.find(entry => entry.scenario === 'modal');
  await openCase(cdp, scene); await prepare(cdp, scene);
  const before = path.join(output, 'proof-before.png'), actual = path.join(output, 'proof-actual.png'), diff = path.join(output, 'proof-diff.png');
  await screenshot(cdp, before);
  await cdp.evaluate(`(() => { const dialog=document.querySelector('[role="dialog"]'); dialog.style.setProperty('min-width', '1000px', 'important'); dialog.style.setProperty('width','1000px','important'); })()`);
  await settle(cdp);
  let detected;
  try { await cdp.run(inspectModal); } catch (error) { detected = error.message; }
  await screenshot(cdp, actual);
  const pixels = await diffImages(before, actual, diff, deps);
  const report = { scenario: scene.id, injection: 'Real production Modal min-width/width:1000px in 520px viewport', geometryDetected: !!detected, assertion: detected, pixels, pass: !!detected && !pixels.pass };
  await saveJSON(path.join(output, 'proof.json'), report);
  if (!report.pass) throw new RunnerError('Detection proof failed: geometry AND pixel comparator must reject the visible regression', 1);
  console.log(`PASS regression proof: ${detected}; ${pixels.different} pixels differ. Inspect ${output}/proof-*.png`);
}

async function main() {
  if (Number(process.versions.node.split('.')[0]) < 22 || typeof WebSocket !== 'function') throw new RunnerError('Node >=22 with native WebSocket is required');
  const deps = await dependencies();
  const binary = await browserBinary();
  const browser = await launch(binary);
  try {
    const cdp = browser.cdp;
    await cdp.call('Page.enable'); await cdp.call('Runtime.enable');
    await cdp.call('Emulation.setTimezoneOverride', { timezoneId: 'UTC' });
    await cdp.call('Emulation.setLocaleOverride', { locale: 'en-US' });
    await cdp.call('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
    await cdp.call('Page.addScriptToEvaluateOnNewDocument', { source: `document.addEventListener('DOMContentLoaded', () => { const style=document.createElement('style'); style.textContent='*,*::before,*::after { animation:none !important; transition:none !important; caret-color:transparent !important; scroll-behavior:auto !important; }'; document.head.append(style); }, {once:true});` });
    const env = await environment(cdp, binary, browser.flags);
    await mkdir(output, { recursive: true });
    await saveJSON(path.join(output, 'environment.json'), env);
    if (mode === '--accept') {
      if (!await exists(path.join(candidate, 'manifest.json'))) throw new RunnerError('No candidate manifest. Run --record and review all five candidate PNGs first.');
      const manifest = await json(path.join(candidate, 'manifest.json'));
      assertEnvironment(manifest.environment, env);
      if (manifest.status !== 'passed' || manifest.images?.length !== cases.length) throw new RunnerError('Candidate is incomplete or failed assertions');
      for (const image of manifest.images) if (await fileHash(path.join(candidate, image.file)) !== image.sha256) throw new RunnerError(`Candidate modified since recording: ${image.file}`);
      const staging = await mkdtemp(path.join(assets, '.accept-'));
      try {
        for (const image of manifest.images) await copyFile(path.join(candidate, image.file), path.join(staging, image.file));
        await saveJSON(path.join(staging, 'manifest.json'), manifest);
        // Explicit --accept is the sole path that replaces the accepted image set.
        await rm(baseline, { recursive: true, force: true }); await rename(staging, baseline);
      } finally { await rm(staging, { recursive: true, force: true }); }
      console.log(`Accepted ${manifest.images.length} reviewed candidate images; environment manifest preserved.`); return;
    }
    try {
      const response = await fetch(`${origin}/frame.html`);
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
    } catch (error) { throw new RunnerError(`Preview server unavailable at ${origin}/frame.html: ${error.message}. Start npm run dev:ui separately.`); }
    if (mode === '--prove-detection') { await prove(cdp, deps); return; }
    let accepted;
    if (mode === 'compare') {
      if (!await exists(path.join(baseline, 'manifest.json'))) throw new RunnerError('No accepted baseline. Run --record, manually review all five candidate PNGs, then explicitly run --accept.');
      accepted = await json(path.join(baseline, 'manifest.json')); assertEnvironment(accepted.environment, env);
      if (accepted.status !== 'passed' || accepted.images?.length !== cases.length) throw new RunnerError('Accepted baseline manifest is incomplete');
      for (const image of accepted.images) if (await fileHash(path.join(baseline, image.file)) !== image.sha256) throw new RunnerError(`Accepted baseline modified outside --accept: ${image.file}`);
    } else {
      await mkdir(candidate, { recursive: true });
      // Invalidate the previous candidate before replacing its images.
      await saveJSON(path.join(candidate, 'manifest.json'), { environment: env, status: 'recording', images: [] });
    }
    const reports = [], images = [];
    for (const scene of cases) {
      const actual = path.join(output, `${scene.id}.actual.png`), diff = path.join(output, `${scene.id}.diff.png`);
      await rm(diff, { force: true });
      let geometry, failure;
      try { await openCase(cdp, scene); geometry = await prepare(cdp, scene); }
      catch (error) { failure = error.message; }
      await screenshot(cdp, actual);
      let pixels;
      if (mode === 'compare') pixels = await diffImages(path.join(baseline, `${scene.id}.png`), actual, diff, deps);
      if (!failure) { try { await afterCapture(cdp, scene); } catch (error) { failure = error.message; } }
      const passed = !failure && (mode === '--record' || pixels.pass);
      if (mode === '--record') { const file = `${scene.id}.png`; await copyFile(actual, path.join(candidate, file)); images.push({ file, sha256: await fileHash(actual) }); }
      reports.push({ id: scene.id, passed, geometry, failure, pixels, actual, ...(pixels ? { diff } : {}) });
      console.log(`${passed ? 'PASS' : 'FAIL'} ${scene.id}${failure ? `: ${failure}` : pixels ? `: ${pixels.different} pixels differ` : ''}`);
    }
    const passed = reports.every(report => report.passed);
    await saveJSON(path.join(output, 'report.json'), { environment: env, passed, reports });
    if (mode === '--record') await saveJSON(path.join(candidate, 'manifest.json'), { environment: env, status: passed ? 'passed' : 'failed', images });
    if (!passed) throw new RunnerError(`Visual/geometry failures retained in ${output}. No baseline was updated.`, 1);
    if (mode === '--record') console.log(`Candidate only: manually review all five PNGs in ${candidate}; run --accept explicitly after review.`);
  } finally { await browser.close(); }
}
main().catch(error => { console.error(`${error.code === 2 ? 'ENVIRONMENT_DRIFT' : 'ERROR'}: ${error.message}`); process.exitCode = error.code ?? 3; });
