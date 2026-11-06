// Dependency-only reproducer, not a product workaround or a screen-reader test.
// Loaded by terminal-accessibility.html and the focused native smoke branch.
// Uses only xterm 6's public API and preserves the raw sequence in every layer.
// Mechanism: https://github.com/xtermjs/xterm.js/blob/6.0.0/src/browser/AccessibilityManager.ts#L367-L385
// Public refresh remains renderer-gated while offscreen:
// https://github.com/xtermjs/xterm.js/blob/6.0.0/src/browser/services/RenderService.ts#L148-L152
// Supported API contract: https://github.com/xtermjs/xterm.js/blob/6.0.0/typings/xterm.d.ts#L1277-L1283
// The stale row range survives until its fixed 1000ms trailing refresh:
// https://github.com/xtermjs/xterm.js/blob/6.0.0/src/browser/TimeBasedDebouncer.ts#L37-L64
// Recreating the terminal or toggling screenReaderMode destroys accessibility
// selection/live-region state; neither is used here as a supposed refresh API.
globalThis.runTerminalAccessibilityProbe = async function ({ Terminal, snapshots }) {
  if (!Array.isArray(snapshots) || snapshots.length !== 2 || !snapshots[1].startsWith(snapshots[0])) {
    throw new Error('Two prefix-preserving output snapshots are required');
  }
  const host = document.createElement('div');
  Object.assign(host.style, { position: 'fixed', left: '0', top: '0', width: '900px', height: '500px', zIndex: '2147483647' });
  document.body.appendChild(host);
  const terminal = new Terminal({ cols: 80, rows: 24, screenReaderMode: true, scrollback: 2000, convertEol: false, cursorBlink: false });
  const changes = [];
  const samples = [];
  const chunks = [];
  let previous = '';
  let reflowAt = Infinity;
  const ticks = rows => rows.flatMap(row => [...row.matchAll(/^TICK:(\d+)\s*$/gm)].map(match => Number(match[1])));
  const capture = phase => {
    const buffer = terminal.buffer.active;
    const lines = Array.from({ length: terminal.rows }, (_, row) => buffer.getLine(buffer.viewportY + row)?.translateToString(true) ?? '');
    const renderer = [...host.querySelectorAll('.xterm-rows > div')].map(row => row.textContent ?? '');
    const tree = [...host.querySelectorAll('.xterm-accessibility-tree > div')].map(row => ({ text: row.textContent ?? '', posinset: row.getAttribute('aria-posinset'), setsize: row.getAttribute('aria-setsize') }));
    return { phase, at: performance.now(), cols: terminal.cols, rows: terminal.rows, viewportY: buffer.viewportY, offscreen: host.getBoundingClientRect().top > innerHeight, buffer: lines, renderer, tree, ticks: { buffer: ticks(lines), renderer: ticks(renderer), tree: ticks(tree.map(row => row.text)) } };
  };
  const frame = () => new Promise(resolve => requestAnimationFrame(resolve));
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  const append = async snapshot => {
    const prefix = snapshot.startsWith(previous);
    const chunk = prefix ? snapshot.slice(previous.length) : snapshot;
    if (!prefix) terminal.reset();
    chunks.push({ previous, snapshot, prefix, chunk });
    previous = snapshot;
    await new Promise(resolve => terminal.write(chunk, resolve));
  };
  let observer;
  try {
    terminal.open(host);
    observer = new MutationObserver(() => changes.push(capture('tree-mutation')));
    observer.observe(host.querySelector('.xterm-accessibility-tree'), { childList: true, characterData: true, subtree: true });
    await append(snapshots[0]);
    // Establish an initial tree; this is setup, not a wait-until-fixed assertion.
    const deadline = performance.now() + 3000;
    while (performance.now() < deadline && !capture('setup').ticks.tree.includes(1)) await delay(20);
    const initial = capture('initial');
    if (!initial.ticks.tree.includes(1)) throw new Error('Initial accessible TICK:1 was not rendered');
    samples.push(initial);
    await delay(1100);
    terminal.refresh(0, terminal.rows - 1);
    await frame(); await frame();
    await append(snapshots[1]);
    await frame(); await frame();
    // Queue a partial accessibility refresh, then let IntersectionObserver pause
    // rendering before changing column count. Reflow moves old TICK row indices.
    host.style.top = `${innerHeight + 1000}px`;
    await frame(); await frame(); await delay(50);
    samples.push(capture('offscreen-before-reflow'));
    reflowAt = performance.now();
    terminal.resize(57, 24);
    samples.push(capture('offscreen-after-reflow'));
    terminal.refresh(0, terminal.rows - 1);
    samples.push(capture('public-refresh-requested-offscreen'));
    // Fixed sampling spans the dependency's 1000ms throttle. Never scroll into
    // view or use eventual convergence as a passing substitute for correctness.
    for (let i = 0; i < 12; i++) { await delay(100); samples.push(capture('offscreen-sample')); }
    const mismatches = [...changes, ...samples].filter(sample => sample.at >= reflowAt && JSON.stringify(sample.ticks.buffer) !== JSON.stringify(sample.ticks.tree));
    const duplicateSamples = mismatches.filter(sample => sample.ticks.tree.some(tick => sample.ticks.tree.filter(value => value === tick).length > sample.ticks.buffer.filter(value => value === tick).length));
    const rowMismatches = [...changes, ...samples].filter(sample => sample.at >= reflowAt && sample.buffer.some((line, row) => line !== (sample.tree[row]?.text === '\u00a0' ? '' : sample.tree[row]?.text)));
    return {
      library: '@xterm/xterm', userAgent: navigator.userAgent,
      scope: 'dependency instance; same prefix-append algorithm as DevelopmentTerminal; not an observation of its production instance',
      applicationAppendObserved: false, screenReaderObserved: false,
      snapshots, chunks, concatenatedChunks: chunks.map(write => write.chunk).join(''),
      chunksEqualLatestSnapshot: chunks.map(write => write.chunk).join('') === snapshots.at(-1),
      samples, treeChanges: changes, mismatches,
      duplicateSamples, treeIntroducedDuplicateTicks: duplicateSamples.length > 0,
      rowMismatches,
      treeMatchesBufferThroughoutReflow: mismatches.length === 0,
    };
  } finally {
    observer?.disconnect(); terminal.dispose(); host.remove();
  }
};
