import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as React from 'react';
import {
  ModuleRuntimeHost,
  type HostInstruction,
  type InstructionChannel,
  type LoadInstruction,
  type ModuleRuntimeHostDeps,
  type Receipt,
  type UnloadInstruction,
} from '../src/modules/moduleRuntimeHost';
import { evaluateFixtureSource, hooksControl, moduleProbe, readFixture, resetModuleProbe } from './helpers/moduleFixtures';

// The real import(blobUrl) cannot run in jsdom/Node. `importModule` is replaced by a double that
// evaluates the fixture source (see evaluateFixtureSource); the real path is covered by the WebView smoke.

type AttachStep = 'ok' | 'fail' | 'throw';
type ReportMode = 'ok' | 'fail' | 'throw' | 'duplicate';

interface RecordedReceipt {
  attachmentId: string;
  ticket: string;
  receipt: Receipt;
}

let ticketSeq = 0;

function loadInstruction(fixture: string, overrides: Partial<LoadInstruction> = {}): LoadInstruction {
  return {
    kind: 'load',
    ticket: `t${++ticketSeq}`,
    runtimeId: 'rt-1',
    moduleId: 'dgos.fixture',
    version: '1.0.0',
    source: readFixture(fixture),
    hookTimeoutMs: 500,
    ...overrides,
  };
}

function unloadInstruction(runtimeId = 'rt-1', overrides: Partial<UnloadInstruction> = {}): UnloadInstruction {
  return { kind: 'unload', ticket: `t${++ticketSeq}`, runtimeId, hookTimeoutMs: 500, ...overrides };
}

const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 25));

function createHarness(attachScript: AttachStep[] = []) {
  const sources = new Map<string, string>();
  const created: string[] = [];
  const revoked: string[] = [];
  const containers: HTMLElement[] = [];
  const receipts: RecordedReceipt[] = [];
  const timers = new Set<unknown>();
  const crashReports: Array<Record<string, unknown>> = [];
  const channel: InstructionChannel = { onmessage: () => undefined };
  const state = {
    attachCalls: 0,
    channels: 0,
    reportMode: 'ok' as ReportMode,
    gate: null as Promise<void> | null,
    crashScript: [] as Array<'ok' | 'fail' | 'throw'>,
  };

  const invoke = vi.fn(async (command: string, args?: Record<string, unknown>): Promise<unknown> => {
    if (command === 'attach_module_host') {
      state.attachCalls += 1;
      const step = attachScript.shift() ?? 'ok';
      if (step === 'throw') throw new Error('attach exploded');
      if (step === 'fail') return { success: false, data: null, error: { code: 1, message: 'denied' } };
      return { success: true, data: { attachmentId: `att-${state.attachCalls}` }, error: null };
    }
    if (command === 'report_module_crash') {
      crashReports.push({ ...args });
      const step = state.crashScript.shift() ?? 'ok';
      if (step === 'throw') throw new Error('crash report exploded');
      if (step === 'fail') return { success: false, data: null, error: { code: 3, message: 'denied' } };
      return { success: true, data: { recorded: true }, error: null };
    }
    if (command === 'report_module_host') {
      receipts.push({
        attachmentId: args?.attachmentId as string,
        ticket: args?.ticket as string,
        receipt: args?.receipt as Receipt,
      });
      if (state.reportMode === 'throw') throw new Error('report exploded');
      if (state.reportMode === 'fail') return { success: false, data: null, error: { code: 2, message: 'nope' } };
      const disposition = state.reportMode === 'duplicate' ? 'duplicate' : 'applied';
      return { success: true, data: { disposition }, error: null };
    }
    throw new Error(`unexpected command ${command}`);
  });

  const deps: ModuleRuntimeHostDeps = {
    invoke: invoke as unknown as ModuleRuntimeHostDeps['invoke'],
    createChannel: () => {
      state.channels += 1;
      return channel;
    },
    createObjectUrl: (source) => {
      const url = `blob:test/${created.length + 1}`;
      sources.set(url, source);
      created.push(url);
      return url;
    },
    revokeObjectUrl: (url) => {
      revoked.push(url);
    },
    importModule: async (url) => {
      await state.gate;
      const source = sources.get(url);
      if (source === undefined) throw new Error('unknown blob url');
      return evaluateFixtureSource(source);
    },
    document: {
      createElement: (tag: string) => {
        const element = document.createElement(tag);
        containers.push(element);
        return element;
      },
    },
    setTimeout: (callback, ms) => {
      const handle = globalThis.setTimeout(() => {
        timers.delete(handle);
        callback();
      }, ms);
      timers.add(handle);
      return handle;
    },
    clearTimeout: (handle) => {
      timers.delete(handle);
      globalThis.clearTimeout(handle as number);
    },
  };

  const host = new ModuleRuntimeHost(deps);
  return {
    host,
    invoke,
    channel,
    state,
    created,
    revoked,
    containers,
    receipts,
    crashReports,
    timers,
    send: (instruction: HostInstruction) => channel.onmessage(instruction),
    waitForReceipts: (count: number) => vi.waitFor(() => expect(receipts).toHaveLength(count)),
    holdImports: () => {
      let release: () => void = () => undefined;
      state.gate = new Promise<void>((resolve) => {
        release = resolve;
      });
      return release;
    },
  };
}

async function attachedHarness(attachScript: AttachStep[] = []) {
  const harness = createHarness(attachScript);
  await harness.host.ensureAttached();
  return harness;
}

beforeEach(() => {
  resetModuleProbe();
  vi.spyOn(console, 'error').mockImplementation(() => undefined);
  vi.spyOn(console, 'warn').mockImplementation(() => undefined);
});
afterEach(() => {
  vi.restoreAllMocks();
});

describe('attach', () => {
  it('attaches once under concurrent and repeated ensureAttached', async () => {
    const h = createHarness();
    await Promise.all([h.host.ensureAttached(), h.host.ensureAttached(), h.host.ensureAttached()]);
    await h.host.ensureAttached();
    expect(h.state.attachCalls).toBe(1);
    expect(h.state.channels).toBe(1);
    expect(h.invoke).toHaveBeenCalledWith('attach_module_host', { onInstruction: h.channel });
  });

  it('resets after a failed attach so a later call retries, and reports with the returned attachmentId', async () => {
    const h = createHarness(['fail', 'throw', 'ok']);
    await expect(h.host.ensureAttached()).resolves.toBeUndefined();
    await expect(h.host.ensureAttached()).resolves.toBeUndefined();
    expect(h.state.attachCalls).toBe(2);
    await h.host.ensureAttached();
    await h.host.ensureAttached();
    expect(h.state.attachCalls).toBe(3);
    expect(console.warn).toHaveBeenCalled();

    const instruction = loadInstruction('counter.js');
    h.send(instruction);
    await h.waitForReceipts(1);
    expect(h.receipts[0]).toEqual({ attachmentId: 'att-3', ticket: instruction.ticket, receipt: { kind: 'loaded' } });
  });
});

describe('load', () => {
  it('mounts the component with the shared React and reports loaded once', async () => {
    const h = await attachedHarness();
    const instruction = loadInstruction('counter.js');
    h.send(instruction);
    await h.waitForReceipts(1);
    await settle();

    expect(h.receipts).toEqual([{ attachmentId: 'att-1', ticket: instruction.ticket, receipt: { kind: 'loaded' } }]);
    const [view] = h.host.getSnapshot();
    expect(h.host.getSnapshot()).toHaveLength(1);
    expect(view).toMatchObject({ runtimeId: 'rt-1', moduleId: 'dgos.fixture', version: '1.0.0' });
    expect(view.container.getAttribute('data-module-runtime')).toBe('rt-1');
    expect(view.container.getAttribute('data-module-id')).toBe('dgos.fixture');
    expect(view.container.isConnected).toBe(false);

    const button = view.container.querySelector('[data-testid="fixture-counter"]');
    expect(button?.textContent).toBe('count:0');
    (button as HTMLElement).click();
    await vi.waitFor(() => expect(button?.textContent).toBe('count:1'));
    expect(view.container.querySelector('[data-testid="fixture-counter"]')).toBe(button);

    expect(moduleProbe().evaluated).toBe(1);
    expect(moduleProbe().events).toEqual(['load:dgos.fixture']);
    expect(h.created).toHaveLength(1);
    expect(h.revoked).toEqual(h.created);
    expect(h.timers.size).toBe(0);
  });

  it('evaluates the module once per load and keeps instances independent', async () => {
    const h = await attachedHarness();
    h.send(loadInstruction('counter.js', { runtimeId: 'rt-a' }));
    h.send(loadInstruction('counter.js', { runtimeId: 'rt-b' }));
    await h.waitForReceipts(2);

    expect(moduleProbe().evaluated).toBe(2);
    const views = h.host.getSnapshot();
    expect(views.map((view) => view.runtimeId).sort()).toEqual(['rt-a', 'rt-b']);
    expect(views[0].container).not.toBe(views[1].container);

    const first = views.find((view) => view.runtimeId === 'rt-a')!.container.querySelector('button')!;
    const second = views.find((view) => view.runtimeId === 'rt-b')!.container.querySelector('button')!;
    first.click();
    await vi.waitFor(() => expect(first.textContent).toBe('count:1'));
    expect(second.textContent).toBe('count:0');
  });

  it.each([
    ['bad-contract.js', 'contract', false, 1],
    ['syntax-error.js', 'import', false, 0],
    ['render-throws.js', 'render', true, 1],
    ['onload-throws.js', 'hook', true, 1],
  ] as const)('%s fails with stage %s and leaves nothing behind', async (fixture, stage, createsContainer, evaluated) => {
    const h = await attachedHarness();
    const listener = vi.fn();
    h.host.subscribe(listener);
    const instruction = loadInstruction(fixture);
    h.send(instruction);
    await h.waitForReceipts(1);
    await settle();

    expect(h.receipts).toEqual([
      { attachmentId: 'att-1', ticket: instruction.ticket, receipt: { kind: 'loadFailed', stage } },
    ]);
    expect(h.host.getSnapshot()).toEqual([]);
    expect(listener).not.toHaveBeenCalled();
    expect(h.containers).toHaveLength(createsContainer ? 1 : 0);
    for (const container of h.containers) {
      expect(container.childNodes).toHaveLength(0);
      expect(container.isConnected).toBe(false);
    }
    expect(h.created).toHaveLength(1);
    expect(h.revoked).toEqual(h.created);
    expect(moduleProbe().evaluated).toBe(evaluated);
    expect(moduleProbe().events).not.toContain('unload:dgos.fixture');
  });

  it('times out a hook that never settles without hanging the host', async () => {
    const h = await attachedHarness();
    const started = performance.now();
    const instruction = loadInstruction('onload-hangs.js', { hookTimeoutMs: 20 });
    h.send(instruction);
    await h.waitForReceipts(1);

    expect(performance.now() - started).toBeLessThan(500);
    expect(h.receipts[0].receipt).toEqual({ kind: 'loadFailed', stage: 'timeout' });
    expect(h.timers.size).toBe(0);
    expect(h.host.getSnapshot()).toEqual([]);
    expect(h.containers).toHaveLength(1);
    expect(h.containers[0].childNodes).toHaveLength(0);
    expect(h.revoked).toEqual(h.created);
    await settle();
    expect(h.receipts).toHaveLength(1);
  });
});

describe('unload', () => {
  it('runs onUnload, disposes the root, revokes the URL and is idempotent', async () => {
    const h = await attachedHarness();
    h.send(loadInstruction('counter.js'));
    await h.waitForReceipts(1);
    const [view] = h.host.getSnapshot();
    document.body.appendChild(view.container);
    expect(view.container.childNodes.length).toBeGreaterThan(0);

    const first = unloadInstruction();
    h.send(first);
    await h.waitForReceipts(2);
    expect(h.receipts[1]).toEqual({ attachmentId: 'att-1', ticket: first.ticket, receipt: { kind: 'unloaded', hookFailed: false } });
    expect(moduleProbe().events).toEqual(['load:dgos.fixture', 'unload:dgos.fixture']);
    expect(h.host.getSnapshot()).toEqual([]);
    expect(view.container.childNodes).toHaveLength(0);
    expect(document.body.contains(view.container)).toBe(false);
    expect(h.revoked).toEqual(h.created);

    const second = unloadInstruction();
    h.send(second);
    await h.waitForReceipts(3);
    expect(h.receipts[2]).toEqual({ attachmentId: 'att-1', ticket: second.ticket, receipt: { kind: 'unloaded', hookFailed: false } });
    expect(moduleProbe().events).toHaveLength(2);
    expect(h.revoked).toEqual(h.created);
  });

  it('cancels a load whose unload arrived while the import was pending, now and for later loads', async () => {
    const h = await attachedHarness();
    const release = h.holdImports();
    const pending = loadInstruction('counter.js', { runtimeId: 'rt-9' });
    const unload = unloadInstruction('rt-9');
    h.send(pending);
    h.send(unload);
    await h.waitForReceipts(1);
    expect(h.receipts[0]).toMatchObject({ ticket: unload.ticket, receipt: { kind: 'unloaded', hookFailed: false } });

    release();
    await h.waitForReceipts(2);
    expect(h.receipts[1]).toMatchObject({ ticket: pending.ticket, receipt: { kind: 'loadFailed', stage: 'cancelled' } });
    await settle();
    expect(h.receipts).toHaveLength(2);
    expect(h.host.getSnapshot()).toEqual([]);
    expect(h.containers).toHaveLength(0);
    expect(moduleProbe().events).toEqual([]);
    expect(h.revoked).toEqual(h.created);

    const later = loadInstruction('counter.js', { runtimeId: 'rt-9' });
    h.send(later);
    await h.waitForReceipts(3);
    expect(h.receipts[2]).toMatchObject({ ticket: later.ticket, receipt: { kind: 'loadFailed', stage: 'cancelled' } });
    expect(h.host.getSnapshot()).toEqual([]);
    expect(h.containers).toHaveLength(0);
    expect(h.created).toHaveLength(1);
    expect(moduleProbe().evaluated).toBe(1);
  });

  it('disposes an instance whose unload arrives while onLoad is still pending', async () => {
    const h = await attachedHarness();
    const pending = loadInstruction('onload-hangs.js', { hookTimeoutMs: 100 });
    h.send(pending);
    await vi.waitFor(() => expect(h.containers).toHaveLength(1));
    const unload = unloadInstruction('rt-1', { hookTimeoutMs: 100 });
    h.send(unload);
    // The unloaded receipt is sent at once; the cancelled load receipt only after the hook
    // timeout. A poll can land after both, so wait for "at least one" and assert the order below.
    await vi.waitFor(() => expect(h.receipts.length).toBeGreaterThanOrEqual(1));
    expect(h.receipts[0]).toMatchObject({ ticket: unload.ticket, receipt: { kind: 'unloaded', hookFailed: false } });
    expect(h.containers[0].childNodes).toHaveLength(0);

    await h.waitForReceipts(2);
    expect(h.receipts[1]).toMatchObject({ ticket: pending.ticket, receipt: { kind: 'loadFailed', stage: 'cancelled' } });
    expect(h.host.getSnapshot()).toEqual([]);
    expect(h.timers.size).toBe(0);
  });

  it('reports hookFailed when onUnload throws and still disposes', async () => {
    const h = await attachedHarness();
    h.send(loadInstruction('onunload-throws.js'));
    await h.waitForReceipts(1);
    const [view] = h.host.getSnapshot();
    document.body.appendChild(view.container);

    h.send(unloadInstruction());
    await h.waitForReceipts(2);
    expect(h.receipts[1].receipt).toEqual({ kind: 'unloaded', hookFailed: true });
    expect(moduleProbe().events).toEqual(['load:dgos.fixture', 'unload:dgos.fixture']);
    expect(h.host.getSnapshot()).toEqual([]);
    expect(view.container.childNodes).toHaveLength(0);
    expect(document.body.contains(view.container)).toBe(false);
    expect(h.revoked).toEqual(h.created);
  });

  it('reports unloaded for a runtime that was never loaded', async () => {
    const h = await attachedHarness();
    const unload = unloadInstruction('never-loaded');
    h.send(unload);
    await h.waitForReceipts(1);
    expect(h.receipts[0]).toMatchObject({ ticket: unload.ticket, receipt: { kind: 'unloaded', hookFailed: false } });
  });
});

describe('report failures', () => {
  it.each(['throw', 'fail'] as const)('a %s report neither throws nor retries nor leaves a half instance', async (mode) => {
    const h = await attachedHarness();
    h.state.reportMode = mode;
    h.send(loadInstruction('counter.js'));
    await h.waitForReceipts(1);
    await settle();
    expect(h.receipts).toHaveLength(1);
    expect(console.warn).toHaveBeenCalled();
    expect(h.host.getSnapshot()).toHaveLength(1);

    h.state.reportMode = 'ok';
    h.send(unloadInstruction());
    await h.waitForReceipts(2);
    expect(h.host.getSnapshot()).toEqual([]);
    expect(h.revoked).toEqual(h.created);
  });

  it('a failing report for a failed load leaves no instance and is not retried', async () => {
    const h = await attachedHarness();
    h.state.reportMode = 'throw';
    h.send(loadInstruction('render-throws.js'));
    await h.waitForReceipts(1);
    await settle();
    expect(h.receipts).toHaveLength(1);
    expect(h.host.getSnapshot()).toEqual([]);
    expect(h.containers[0].childNodes).toHaveLength(0);
  });

  it('a non-applied disposition is only logged', async () => {
    const h = await attachedHarness();
    h.state.reportMode = 'duplicate';
    h.send(loadInstruction('counter.js'));
    await h.waitForReceipts(1);
    await settle();
    expect(console.warn).toHaveBeenCalledWith(expect.any(String), 'duplicate');
    expect(h.host.getSnapshot()).toHaveLength(1);
    expect(h.receipts).toHaveLength(1);
  });
});

describe('snapshot store', () => {
  it('keeps identity between changes and notifies subscribers on change', async () => {
    const h = await attachedHarness();
    const initial = h.host.getSnapshot();
    expect(h.host.getSnapshot()).toBe(initial);

    const listener = vi.fn();
    const unsubscribe = h.host.subscribe(listener);
    h.send(loadInstruction('counter.js'));
    await h.waitForReceipts(1);
    expect(listener).toHaveBeenCalledTimes(1);
    const loaded = h.host.getSnapshot();
    expect(loaded).not.toBe(initial);
    expect(loaded).toHaveLength(1);
    expect(h.host.getSnapshot()).toBe(loaded);

    h.send(unloadInstruction());
    await h.waitForReceipts(2);
    expect(listener).toHaveBeenCalledTimes(2);
    expect(h.host.getSnapshot()).toEqual([]);

    unsubscribe();
    h.send(loadInstruction('counter.js', { runtimeId: 'rt-2' }));
    await h.waitForReceipts(3);
    expect(listener).toHaveBeenCalledTimes(2);
  });
});

interface UpdateDriver {
  send: (instruction: HostInstruction) => void;
  waitForReceipts: (count: number) => Promise<unknown>;
}

describe('hot update state handover', () => {
  async function loadOld(h: UpdateDriver) {
    h.send(loadInstruction('hooks-update.js', { runtimeId: 'rt-old', version: '1.0.0' }));
    await h.waitForReceipts(1);
  }

  function update(h: UpdateDriver, overrides: Partial<LoadInstruction> = {}) {
    h.send(
      loadInstruction('hooks-update.js', { runtimeId: 'rt-new', version: '2.0.0', replaces: 'rt-old', ...overrides }),
    );
  }

  it('hands the JSON-roundtripped state from the old instance to the new one, in order', async () => {
    const h = await attachedHarness();
    await loadOld(h);
    update(h);
    await h.waitForReceipts(2);

    expect(h.receipts[1].receipt).toEqual({ kind: 'loaded' });
    expect(moduleProbe().events).toEqual(['load:1.0.0', 'before:1.0.0', 'load:2.0.0', 'after:2.0.0']);
    const restored = hooksControl().restored as Record<string, unknown>;
    expect(restored).toEqual({ count: 3, list: [1, 2], when: '1970-01-01T00:00:00.000Z' });
    expect('skipped' in restored).toBe(false);
    // The old instance is not unloaded by the load; the host sends its own unload later.
    expect(h.host.getSnapshot().map((view) => view.runtimeId)).toEqual(['rt-old', 'rt-new']);
    expect(moduleProbe().events).not.toContain('unload:1.0.0');
    // The state stays in page memory: it is in no command argument and no receipt.
    expect(JSON.stringify(h.invoke.mock.calls)).not.toContain('1970-01-01');
    expect(h.timers.size).toBe(0);
  });

  it('accepts a state of exactly 256 KiB', async () => {
    const h = await attachedHarness();
    await loadOld(h);
    hooksControl().before = 'max';
    update(h);
    await h.waitForReceipts(2);
    expect(h.receipts[1].receipt).toEqual({ kind: 'loaded' });
    expect(hooksControl().restored).toBe('x'.repeat(256 * 1024 - 2));
    expect(moduleProbe().events).toContain('after:2.0.0');
  });

  it.each(['throw', 'reject', 'hang', 'function', 'huge', 'circular'] as const)(
    'onBeforeUpdate mode %s means no state: the update loads without onAfterUpdate',
    async (mode) => {
      const h = await attachedHarness();
      await loadOld(h);
      hooksControl().before = mode;
      update(h, { hookTimeoutMs: mode === 'hang' ? 20 : 500 });
      await h.waitForReceipts(2);

      expect(h.receipts[1].receipt).toEqual({ kind: 'loaded' });
      expect(moduleProbe().events).toEqual(['load:1.0.0', 'before:1.0.0', 'load:2.0.0']);
      expect(hooksControl().restored).toBeUndefined();
      expect(h.host.getSnapshot().map((view) => view.runtimeId)).toEqual(['rt-old', 'rt-new']);
      expect(h.timers.size).toBe(0);
    },
  );

  it.each([undefined, null, 'rt-missing'])('replaces %s runs no update hooks', async (replaces) => {
    const h = await attachedHarness();
    await loadOld(h);
    update(h, { replaces });
    await h.waitForReceipts(2);
    expect(h.receipts[1].receipt).toEqual({ kind: 'loaded' });
    expect(moduleProbe().events).toEqual(['load:1.0.0', 'load:2.0.0']);
  });

  it.each(['throw', 'reject', 'hang'] as const)(
    'onAfterUpdate mode %s fails the new load with stage hook and leaves the old instance untouched',
    async (mode) => {
      const h = await attachedHarness();
      await loadOld(h);
      const [oldView] = h.host.getSnapshot();
      hooksControl().after = mode;
      update(h, { hookTimeoutMs: mode === 'hang' ? 20 : 500 });
      await h.waitForReceipts(2);

      expect(h.receipts[1].receipt).toEqual({ kind: 'loadFailed', stage: 'hook' });
      expect(moduleProbe().events).toEqual(['load:1.0.0', 'before:1.0.0', 'load:2.0.0', 'after:2.0.0']);
      expect(h.host.getSnapshot()).toHaveLength(1);
      expect(h.host.getSnapshot()[0]).toBe(oldView);
      expect(oldView.container.textContent).toBe('version:1.0.0');
      expect(h.containers).toHaveLength(2);
      expect(h.containers[1].childNodes).toHaveLength(0);
      expect(h.revoked).toEqual(h.created);
      expect(h.timers.size).toBe(0);

      const unload = unloadInstruction('rt-old');
      h.send(unload);
      await h.waitForReceipts(3);
      expect(h.receipts[2].receipt).toEqual({ kind: 'unloaded', hookFailed: false });
      expect(moduleProbe().events).toContain('unload:1.0.0');
    },
  );
});

describe('post-mount crash reporting', () => {
  it('reports a render error caught after mount exactly once, with a truncated stack and no message', async () => {
    const h = await attachedHarness();
    h.send(loadInstruction('crash-after-mount.js'));
    await h.waitForReceipts(1);
    h.send({ kind: 'route', runtimeId: 'rt-1', route: 'route-1' });

    const listener = vi.fn();
    h.host.subscribe(listener);
    const [before] = h.host.getSnapshot();
    expect(before.crashed).toBe(false);
    before.container.querySelector<HTMLElement>('[data-testid="fixture-crash"]')!.click();
    await vi.waitFor(() => expect(h.crashReports).toHaveLength(1));

    const report = h.crashReports[0];
    expect(Object.keys(report).sort()).toEqual(['attachmentId', 'callId', 'errorName', 'route', 'source', 'stack']);
    expect(report).toMatchObject({ attachmentId: 'att-1', route: 'route-1', source: 'render', errorName: 'FixtureCrash' });
    expect(typeof report.callId).toBe('string');
    expect((report.callId as string).length).toBeGreaterThan(0);
    const fullStack = 'FixtureCrash\n' + '    at frame (module:1:1)\n'.repeat(1000);
    expect(report.stack).toBe(fullStack.slice(0, 8 * 1024));
    expect(JSON.stringify(report)).not.toContain('secret');

    const [after] = h.host.getSnapshot();
    expect(after.crashed).toBe(true);
    expect(after.container).toBe(before.container);
    expect(h.host.getSnapshot()).toHaveLength(1);
    expect(listener).toHaveBeenCalledTimes(1);
    expect(after.container.querySelector('[role="alert"]')?.textContent).toBe('This module failed to render.');

    await settle();
    expect(h.crashReports).toHaveLength(1);
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it('marks the instance crashed but reports nothing when it has no route', async () => {
    const h = await attachedHarness();
    h.send(loadInstruction('crash-after-mount.js'));
    await h.waitForReceipts(1);

    h.host.getSnapshot()[0].container.querySelector<HTMLElement>('[data-testid="fixture-crash"]')!.click();
    await vi.waitFor(() => expect(h.host.getSnapshot()[0].crashed).toBe(true));
    await settle();
    expect(h.crashReports).toEqual([]);
    expect(h.invoke.mock.calls.map(([command]) => command)).not.toContain('report_module_crash');
  });

  async function crashWith(script: Array<'ok' | 'fail' | 'throw'>) {
    const h = await attachedHarness();
    h.send(loadInstruction('crash-after-mount.js'));
    await h.waitForReceipts(1);
    h.send({ kind: 'route', runtimeId: 'rt-1', route: 'route-1' });
    h.state.crashScript = [...script];
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    h.host.getSnapshot()[0].container.querySelector<HTMLElement>('[data-testid="fixture-crash"]')!.click();
    return h;
  }

  it('retries a failed crash report with the same callId and stops after three attempts', async () => {
    const h = await crashWith(['throw', 'fail', 'fail', 'ok']);
    try {
      await vi.advanceTimersByTimeAsync(0);
      expect(h.crashReports).toHaveLength(1);
      await vi.advanceTimersByTimeAsync(250);
      expect(h.crashReports).toHaveLength(2);
      await vi.advanceTimersByTimeAsync(500);
      expect(h.crashReports).toHaveLength(3);
      await vi.advanceTimersByTimeAsync(60_000);
      expect(h.crashReports).toHaveLength(3);
      expect(new Set(h.crashReports.map((report) => report.callId)).size).toBe(1);
      expect(h.host.getSnapshot()[0].crashed).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });

  it('stops retrying once a retry is accepted', async () => {
    const h = await crashWith(['fail', 'ok', 'ok']);
    try {
      await vi.advanceTimersByTimeAsync(0);
      expect(h.crashReports).toHaveLength(1);
      await vi.advanceTimersByTimeAsync(250);
      expect(h.crashReports).toHaveLength(2);
      await vi.advanceTimersByTimeAsync(60_000);
      expect(h.crashReports).toHaveLength(2);
      expect(h.crashReports[1].callId).toBe(h.crashReports[0].callId);
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not report a load-time render failure as a crash', async () => {
    const h = await attachedHarness();
    h.send({ kind: 'route', runtimeId: 'rt-1', route: 'route-1' });
    h.send(loadInstruction('render-throws.js'));
    await h.waitForReceipts(1);
    await settle();
    expect(h.receipts[0].receipt).toEqual({ kind: 'loadFailed', stage: 'render' });
    expect(h.crashReports).toEqual([]);
  });
});

describe('shared React (D055)', () => {
  it("hands the module the host's own React object", async () => {
    const h = await attachedHarness();
    h.send(loadInstruction('hooks-react-identity.js'));
    await h.waitForReceipts(1);
    expect(h.receipts[0].receipt).toEqual({ kind: 'loaded' });
    expect(hooksControl().seenReact).toBe(React);
  });

  it("fails an entry with a bare import 'react' at the import stage", async () => {
    const h = await attachedHarness();
    h.send(loadInstruction('hooks-bare-import.js'));
    await h.waitForReceipts(1);
    expect(h.receipts[0].receipt).toEqual({ kind: 'loadFailed', stage: 'import' });
    expect(h.host.getSnapshot()).toEqual([]);
    expect(moduleProbe().evaluated).toBe(0);
    expect(h.revoked).toEqual(h.created);
  });
});
