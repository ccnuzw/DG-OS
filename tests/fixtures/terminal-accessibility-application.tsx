// Browser-only application probe. No native host is mocked or contacted: the
// process is already exited, so DevelopmentTerminal never sends a PTY resize.
// Run with Vite at /tests/fixtures/terminal-accessibility.html?application=1.
import React from 'react';
import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { Terminal } from '@xterm/xterm';
import { DevelopmentTerminal } from '../../src/components/development/DevelopmentTerminal';
import type { DevelopmentProcess } from '../../src/stores/developmentRuntime';

export async function runApplicationProbe() {
  const host = document.createElement('div');
  Object.assign(host.style, { position: 'fixed', top: '0', left: '0', width: '900px', height: '500px' });
  const style = document.createElement('style');
  style.textContent = '.dev-terminal { height: 400px; --surface-code: #000000; --text-primary: #ffffff; }';
  document.head.appendChild(style); document.body.appendChild(host);
  const first = `${'P'.repeat(148)}\r\nMARK\r\nTICK:1\r\n`;
  const snapshots = [first, `${first}TICK:2\r\n`];
  const process: DevelopmentProcess = { id: 'accessibility-probe', project_id: 'probe', task_id: 'probe', kind: 'terminal', source: 'host_terminal', status: 'exited', output: first, truncated: false, exit_code: 0, reason: null, started_at: 1, finished_at: 2, program: '/bin/sh', args: [], cwd: '', fingerprint: '', start_fingerprint: '', verification_status: 'not_requested', verification_complete: false, mutation_observed: false, preview_url: null };
  const open = Terminal.prototype.open;
  const write = Terminal.prototype.write;
  const chunks: string[] = [];
  let instance: Terminal | undefined;
  Terminal.prototype.open = function (parent) { if (host.contains(parent)) instance = this; return open.call(this, parent); };
  Terminal.prototype.write = function (data, callback) { if (this === instance) chunks.push(typeof data === 'string' ? data : new TextDecoder().decode(data)); return write.call(this, data, callback); };
  const root = createRoot(host);
  const delay = (ms: number) => {
    const pending = Promise.withResolvers<void>();
    setTimeout(pending.resolve, ms);
    return pending.promise;
  };
  const frame = () => {
    const pending = Promise.withResolvers<void>();
    requestAnimationFrame(() => pending.resolve());
    return pending.promise;
  };
  const capture = (phase: string) => {
    if (!instance) throw new Error('DevelopmentTerminal did not open');
    const buffer = instance.buffer.active;
    const rows = Array.from({ length: instance.rows }, (_, row) => buffer.getLine(buffer.viewportY + row)?.translateToString(true) ?? '');
    const renderer = [...host.querySelectorAll('.xterm-rows > div')].map(row => row.textContent ?? '');
    const tree = [...host.querySelectorAll('.xterm-accessibility-tree > div')].map(row => row.textContent ?? '');
    return { phase, at: performance.now(), cols: instance.cols, rows: instance.rows, buffer: rows, renderer, tree, ticks: Object.fromEntries(Object.entries({ buffer: rows, renderer, tree }).map(([layer, lines]) => [layer, lines.flatMap(line => [...line.matchAll(/^TICK:(\d+)\s*$/gm)].map(match => Number(match[1])))])) };
  };
  const samples: { phase: string; at: number; cols: number; rows: number; buffer: string[]; renderer: string[]; tree: string[]; ticks: Record<string, number[]> }[] = [];
  let reflowAt = Infinity;
  const observer = new MutationObserver(() => { if (performance.now() >= reflowAt) samples.push(capture('tree-mutation')); });
  try {
    flushSync(() => root.render(<DevelopmentTerminal process={process} />));
    observer.observe(host.querySelector('.xterm-accessibility-tree')!, { childList: true, characterData: true, subtree: true });
    for (let i = 0; i < 150; i++) { if (capture('setup').ticks.tree.includes(1)) break; await delay(20); }
    if (!instance || !capture('setup').ticks.tree.includes(1)) throw new Error('Initial application tree missing TICK:1');
    instance.resize(80, 24);
    await delay(1100);
    instance.refresh(0, instance.rows - 1);
    await frame(); await frame();
    samples.push(capture('initial'));
    flushSync(() => root.render(<DevelopmentTerminal process={{ ...process, output: snapshots[1] }} />));
    const parsed = Promise.withResolvers<void>();
    write.call(instance, '', parsed.resolve);
    await parsed.promise;
    await frame(); await frame();
    host.style.top = `${innerHeight + 1000}px`;
    await frame(); await frame(); await delay(50);
    reflowAt = performance.now();
    instance.resize(57, 24);
    instance.refresh(0, instance.rows - 1);
    samples.push(capture('public-refresh-requested-offscreen'));
    for (let i = 0; i < 12; i++) { await delay(100); samples.push(capture('offscreen-sample')); }
    const duplicateSamples = samples.filter(sample => sample.at >= reflowAt && sample.ticks.tree.some(tick => sample.ticks.tree.filter(value => value === tick).length > sample.ticks.buffer.filter(value => value === tick).length));
    return { scope: 'real DevelopmentTerminal, synthetic exited process snapshots, no native host or real screen reader', snapshots, applicationChunks: chunks, chunksEqualLatestSnapshot: chunks.join('') === snapshots[1], samples, duplicateSamples, treeIntroducedDuplicateTicks: duplicateSamples.length > 0 };
  } finally {
    observer.disconnect();
    root.unmount(); Terminal.prototype.open = open; Terminal.prototype.write = write; host.remove(); style.remove();
  }
}
