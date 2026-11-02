// D113: `host.<capability>.<method>(..., { signal })` as a dynamic module sees it.
//
// Evidence level: jsdom. Modules are loaded through the real `ModuleRuntimeHost` (`readContract` builds `host`),
// `import(blobUrl)` is replaced by evaluating the source and `invoke` is a double that plays the host. NOT proven
// here: that the real host cancels (Rust tests and the WebView smoke `webview_smoke_call` cover that) and the real
// Tauri IPC transport.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  HostCallError,
  MODULE_CALL_CANCELLED_CODE,
  ModuleRuntimeHost,
  type HostInstruction,
  type InstructionChannel,
  type LoadInstruction,
  type Receipt,
} from '../src/modules/moduleRuntimeHost';
import { evaluateFixtureSource, resetModuleProbe } from './helpers/moduleFixtures';

const MODULE = 'cancel-probe';
const SOURCE = `export default function createModule(host) {
  (globalThis.__dgosCancelHosts ??= {})[host.moduleId] = host;
  return { Component() { return null; } };
}`;

interface Call {
  command: string;
  args: Record<string, unknown>;
}

type Reply = { success: boolean; data: unknown; error: { code: number; message: string } | null };

const ok = (data: unknown): Reply => ({ success: true, data, error: null });
const cancelled = (): Reply => ({ success: false, data: null, error: { code: 1004028, message: 'Module call cancelled' } });

type CapabilityHost = {
  providers: { call: (...args: unknown[]) => Promise<unknown> };
  storage: Record<'get' | 'set' | 'delete' | 'list', (...args: unknown[]) => Promise<unknown>>;
  network: { request: (...args: unknown[]) => Promise<unknown> };
  mcp: { callTool: (...args: unknown[]) => Promise<unknown> };
  skills: { prepare: (...args: unknown[]) => Promise<unknown> };
};

/** A real runtime host with a scripted Tauri side. `capability` answers every capability command. */
async function createRuntime(
  capability: (call: Call) => Promise<Reply> | Reply,
  cancel: (call: Call) => Promise<Reply> | Reply = () => ok(true),
) {
  const sources = new Map<string, string>();
  const receipts: Receipt[] = [];
  const calls: Call[] = [];
  const channel: InstructionChannel = { onmessage: () => undefined };
  const invoke = vi.fn(async (command: string, args: Record<string, unknown> = {}): Promise<unknown> => {
    if (command === 'attach_module_host') return ok({ attachmentId: 'att-1' });
    if (command === 'report_module_host') {
      receipts.push(args.receipt as Receipt);
      return ok({ disposition: 'applied' });
    }
    const call = { command, args: { ...args } };
    calls.push(call);
    return command === 'cancel_module_call' ? cancel(call) : capability(call);
  });
  const host = new ModuleRuntimeHost({
    invoke: invoke as never,
    createChannel: () => channel,
    createObjectUrl: (source) => {
      const url = `blob:test/${sources.size + 1}`;
      sources.set(url, source);
      return url;
    },
    revokeObjectUrl: () => undefined,
    importModule: async (url) => evaluateFixtureSource(sources.get(url) ?? ''),
  });
  await host.ensureAttached();
  channel.onmessage({ kind: 'route', runtimeId: 'rt-1', route: 'route-1' } as HostInstruction);
  const instruction: LoadInstruction = {
    kind: 'load',
    ticket: 't1',
    runtimeId: 'rt-1',
    moduleId: MODULE,
    version: '1.0.0',
    source: SOURCE,
    hookTimeoutMs: 500,
  };
  channel.onmessage(instruction);
  await vi.waitFor(() => expect(receipts).toEqual([{ kind: 'loaded' }]));
  const hosts: unknown = Reflect.get(globalThis, '__dgosCancelHosts');
  const moduleHost = (hosts as Record<string, CapabilityHost>)[MODULE];
  const sent = (command: string) => calls.filter((call) => call.command === command);
  return { moduleHost, calls, sent };
}

/** A capability reply that stays pending until `release` is called. */
function gated(reply: Reply) {
  const { promise: gate, resolve: release } = Promise.withResolvers<void>();
  return { wait: async () => { await gate; return reply; }, release };
}

beforeEach(() => {
  resetModuleProbe();
  vi.spyOn(console, 'error').mockImplementation(() => undefined);
  vi.spyOn(console, 'warn').mockImplementation(() => undefined);
});

afterEach(() => {
  vi.restoreAllMocks();
  Reflect.deleteProperty(globalThis, '__dgosCancelHosts');
  document.body.replaceChildren();
});

describe('host.<capability>.<method>(..., { signal }) (D113)', () => {
  it('aborting sends cancel_module_call with the same attachment, route and callId, and rejects with 1004028', async () => {
    const pending = gated(cancelled());
    const runtime = await createRuntime(pending.wait, () => {
      pending.release();
      return ok(true);
    });
    const controller = new AbortController();
    const promise = runtime.moduleHost.providers.call('p1', 'm1', { messages: [] }, { signal: controller.signal });
    const settled = promise.then(
      () => 'resolved',
      (error: unknown) => error,
    );
    await vi.waitFor(() => expect(runtime.sent('module_call_provider')).toHaveLength(1));
    expect(runtime.sent('cancel_module_call')).toHaveLength(0);

    controller.abort();
    const error = await settled;
    expect(error).toBeInstanceOf(HostCallError);
    expect((error as HostCallError).code).toBe(1004028);
    expect(MODULE_CALL_CANCELLED_CODE).toBe(1004028);

    const [call] = runtime.sent('module_call_provider');
    const [cancel] = runtime.sent('cancel_module_call');
    expect(runtime.sent('cancel_module_call')).toHaveLength(1);
    expect(cancel.args).toEqual({ attachmentId: 'att-1', route: 'route-1', callId: call.args.callId });
    expect(call.args).toMatchObject({ attachmentId: 'att-1', route: 'route-1', providerId: 'p1', modelId: 'm1' });
    expect(typeof call.args.callId).toBe('string');
  });

  it('an already aborted signal sends nothing and rejects with 1004028', async () => {
    const runtime = await createRuntime(() => ok({ content: 'never' }));
    const controller = new AbortController();
    controller.abort();
    const error = await runtime.moduleHost.providers
      .call('p1', 'm1', { messages: [] }, { signal: controller.signal })
      .then(() => null, (reason: unknown) => reason);
    expect(error).toBeInstanceOf(HostCallError);
    expect((error as HostCallError).code).toBe(1004028);
    expect(runtime.calls).toEqual([]);
  });

  it('without a signal nothing changes: no cancel command is ever sent', async () => {
    const runtime = await createRuntime(() => ok({ content: 'pong' }));
    await expect(runtime.moduleHost.providers.call('p1', 'm1', { messages: [] })).resolves.toEqual({ content: 'pong' });
    await expect(runtime.moduleHost.providers.call('p1', 'm1', { messages: [] }, {})).resolves.toEqual({ content: 'pong' });
    expect(runtime.sent('cancel_module_call')).toHaveLength(0);
    expect(runtime.sent('module_call_provider')).toHaveLength(2);
  });

  it('aborting after the call settled sends nothing', async () => {
    const runtime = await createRuntime(() => ok({ content: 'pong' }));
    const controller = new AbortController();
    await runtime.moduleHost.providers.call('p1', 'm1', { messages: [] }, { signal: controller.signal });
    controller.abort();
    const { promise: elapsed, resolve } = Promise.withResolvers<void>();
    setTimeout(resolve, 30);
    await elapsed;
    expect(runtime.sent('cancel_module_call')).toHaveLength(0);
  });

  it('a call id is never reused: two calls with one signal cancel only the one still pending', async () => {
    const first = gated(cancelled());
    let served = 0;
    const runtime = await createRuntime(
      (call) => (call.command === 'module_call_provider' && served++ === 0 ? first.wait() : ok({ content: 'done' })),
      () => {
        first.release();
        return ok(true);
      },
    );
    const controller = new AbortController();
    const options = { signal: controller.signal };
    const pending = runtime.moduleHost.providers.call('p', 'm', { messages: [] }, options).then(() => null, (e: unknown) => e);
    await vi.waitFor(() => expect(runtime.sent('module_call_provider')).toHaveLength(1));
    controller.abort();
    expect(((await pending) as HostCallError).code).toBe(1004028);
    const [one] = runtime.sent('module_call_provider');
    const [cancel] = runtime.sent('cancel_module_call');
    expect(cancel.args.callId).toBe(one.args.callId);
    // The aborted signal now rejects a later call before anything is sent.
    await expect(runtime.moduleHost.providers.call('p', 'm', { messages: [] }, options)).rejects.toMatchObject({ code: 1004028 });
    expect(runtime.sent('module_call_provider')).toHaveLength(1);
  });

  it('retries a cancel that the host answered false while the call is still pending', async () => {
    const pending = gated(cancelled());
    let attempts = 0;
    const runtime = await createRuntime(pending.wait, () => {
      attempts += 1;
      if (attempts < 3) return ok(false);
      pending.release();
      return ok(true);
    });
    const controller = new AbortController();
    const settled = runtime.moduleHost.storage
      .get('k', { signal: controller.signal })
      .then(() => null, (error: unknown) => error);
    await vi.waitFor(() => expect(runtime.sent('module_storage_get')).toHaveLength(1));
    controller.abort();
    expect(((await settled) as HostCallError).code).toBe(1004028);
    expect(runtime.sent('cancel_module_call')).toHaveLength(3);
    const ids = new Set(runtime.sent('cancel_module_call').map((call) => call.args.callId));
    expect(ids).toEqual(new Set([runtime.sent('module_storage_get')[0].args.callId]));
  });

  it('a refused cancel is not an error of its own: the call settles on its own terms', async () => {
    const pending = gated(ok('value'));
    const runtime = await createRuntime(pending.wait, () => ({
      success: false,
      data: null,
      error: { code: 1004004, message: 'Host permission denied' },
    }));
    const controller = new AbortController();
    const result = runtime.moduleHost.storage.get('k', { signal: controller.signal });
    await vi.waitFor(() => expect(runtime.sent('module_storage_get')).toHaveLength(1));
    controller.abort();
    await vi.waitFor(() => expect(runtime.sent('cancel_module_call')).toHaveLength(1));
    pending.release();
    await expect(result).resolves.toBe('value');
    expect(runtime.sent('cancel_module_call')).toHaveLength(1);
  });

  const capabilityCalls: Array<[string, string, (host: CapabilityHost, options: { signal: AbortSignal }) => Promise<unknown>]> = [
    ['providers.call', 'module_call_provider', (h, o) => h.providers.call('p', 'm', { messages: [] }, o)],
    ['storage.get', 'module_storage_get', (h, o) => h.storage.get('k', o)],
    ['storage.set', 'module_storage_set', (h, o) => h.storage.set('k', 1, o)],
    ['storage.delete', 'module_storage_delete', (h, o) => h.storage.delete('k', o)],
    ['storage.list', 'module_storage_list', (h, o) => h.storage.list(o)],
    ['network.request', 'module_network_request', (h, o) => h.network.request({ url: 'http://127.0.0.1:1/' }, o)],
    ['mcp.callTool', 'module_call_mcp_tool', (h, o) => h.mcp.callTool('s', 't', {}, o)],
    ['skills.prepare', 'module_prepare_skill', (h, o) => h.skills.prepare('skill', o)],
  ];

  it.each(capabilityCalls)('%s takes the optional trailing { signal } and cancels its own callId', async (_name, command, invoke) => {
    const pending = gated(cancelled());
    const runtime = await createRuntime(pending.wait, () => {
      pending.release();
      return ok(true);
    });
    const controller = new AbortController();
    const settled = invoke(runtime.moduleHost, { signal: controller.signal }).then(() => null, (error: unknown) => error);
    await vi.waitFor(() => expect(runtime.sent(command)).toHaveLength(1));
    controller.abort();
    expect(((await settled) as HostCallError).code).toBe(1004028);
    expect(runtime.sent('cancel_module_call')[0].args).toEqual({
      attachmentId: 'att-1',
      route: 'route-1',
      callId: runtime.sent(command)[0].args.callId,
    });
  });

  it('the host object stays frozen with the same members and apiVersion 1', async () => {
    const runtime = await createRuntime(() => ok(null));
    const host = runtime.moduleHost as unknown as Record<string, unknown>;
    expect(Object.isFrozen(host)).toBe(true);
    expect(host.apiVersion).toBe(1);
    expect(Object.keys(host).sort()).toEqual(['React', 'apiVersion', 'mcp', 'moduleId', 'network', 'providers', 'skills', 'storage', 'ui', 'version']);
    for (const namespace of ['providers', 'storage', 'network', 'mcp', 'skills']) {
      expect(Object.isFrozen(host[namespace])).toBe(true);
    }
  });
});
