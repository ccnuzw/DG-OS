import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { Modal } from '@dgos/ui';
import ModuleMcpConfirm from '../src/modules/ModuleMcpConfirm';
import {
  ModuleRuntimeHost,
  type HostInstruction,
  type InstructionChannel,
  type McpConfirmInstruction,
  type ModuleRuntimeHostDeps,
} from '../src/modules/moduleRuntimeHost';
import { evaluateFixtureSource, readFixture } from './helpers/moduleFixtures';

// D093 item 6: the host shows each module `mcp.call` for approval, outside the module boundary. The Tauri side
// (token minting, deadline, 1003019) is covered by the Rust tests; this file covers the page side only.

const ARGUMENTS = { text: 'exact-value', nested: { n: [1, 2, 3] } };

function confirmInstruction(overrides: Partial<McpConfirmInstruction> = {}): McpConfirmInstruction {
  return {
    kind: 'confirmMcp',
    confirmationId: 'c1',
    moduleName: 'Example Module',
    serverId: 'server-7',
    toolName: 'echo',
    arguments: ARGUMENTS,
    ...overrides,
  };
}

function createHarness(answer: 'ok' | 'denied' | 'throw' = 'ok') {
  const sources = new Map<string, string>();
  const channel: InstructionChannel = { onmessage: () => undefined };
  const invoke = vi.fn(async (command: string, args?: Record<string, unknown>): Promise<unknown> => {
    if (command === 'attach_module_host') return { success: true, data: { attachmentId: 'att-1' }, error: null };
    if (command === 'report_module_host') return { success: true, data: { disposition: 'applied' }, error: null };
    if (command === 'resolve_module_mcp_confirmation') {
      if (answer === 'throw') throw new Error('boom');
      if (answer === 'denied') return { success: false, data: null, error: { code: 1004004, message: 'denied' } };
      return { success: true, data: null, error: null };
    }
    if (command === 'module_call_mcp_tool') return { success: true, data: { is_error: false }, error: null };
    throw new Error(`unexpected command ${command} ${JSON.stringify(args)}`);
  });
  const deps: ModuleRuntimeHostDeps = {
    invoke: invoke as unknown as ModuleRuntimeHostDeps['invoke'],
    createChannel: () => channel,
    createObjectUrl: (source) => {
      const url = `blob:test/${sources.size + 1}`;
      sources.set(url, source);
      return url;
    },
    revokeObjectUrl: () => undefined,
    importModule: async (url) => evaluateFixtureSource(sources.get(url) ?? ''),
  };
  const host = new ModuleRuntimeHost(deps);
  const send = (instruction: HostInstruction) => act(async () => channel.onmessage(instruction));
  const resolveCalls = () => invoke.mock.calls.filter(([command]) => command === 'resolve_module_mcp_confirmation');
  return { host, invoke, send, resolveCalls };
}

beforeEach(() => {
  vi.spyOn(console, 'error').mockImplementation(() => undefined);
  vi.spyOn(console, 'warn').mockImplementation(() => undefined);
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('pending confirmations in the runtime host', () => {
  it('keeps confirmMcp host-side, in order, ignoring duplicates and malformed instructions', async () => {
    const { host, send } = createHarness();
    await host.ensureAttached();
    const seen = vi.fn();
    const unsubscribe = host.subscribeMcpConfirmations(seen);
    await send(confirmInstruction());
    await send(confirmInstruction({ confirmationId: 'c2', toolName: 'second' }));
    await send(confirmInstruction()); // duplicate id
    await send({ ...confirmInstruction({ confirmationId: 'c3' }), toolName: 7 } as unknown as HostInstruction);
    expect(host.getMcpConfirmations().map((pending) => pending.confirmationId)).toEqual(['c1', 'c2']);
    expect(host.getMcpConfirmations()[0]).toMatchObject({
      moduleName: 'Example Module',
      serverId: 'server-7',
      toolName: 'echo',
      arguments: ARGUMENTS,
    });
    expect(seen).toHaveBeenCalledTimes(2);
    // The instance snapshot is untouched.
    expect(host.getSnapshot()).toEqual([]);
    unsubscribe();
  });

  it('removes a confirmation on confirmMcpClosed and ignores unknown ids', async () => {
    const { host, send, resolveCalls } = createHarness();
    await host.ensureAttached();
    await send(confirmInstruction());
    await send({ kind: 'confirmMcpClosed', confirmationId: 'other' });
    expect(host.getMcpConfirmations()).toHaveLength(1);
    await send({ kind: 'confirmMcpClosed', confirmationId: 'c1' });
    expect(host.getMcpConfirmations()).toHaveLength(0);
    // Closed by the host: nothing is answered.
    expect(resolveCalls()).toHaveLength(0);
  });

  it('answers once with only the id and the decision, and survives a failing answer', async () => {
    for (const mode of ['ok', 'denied', 'throw'] as const) {
      const { host, send, resolveCalls } = createHarness(mode);
      await host.ensureAttached();
      await send(confirmInstruction());
      await Promise.all([host.resolveMcpConfirmation('c1', true), host.resolveMcpConfirmation('c1', true)]);
      expect(resolveCalls()).toEqual([['resolve_module_mcp_confirmation', { confirmationId: 'c1', approved: true }]]);
      expect(host.getMcpConfirmations()).toHaveLength(0);
      // An unknown id sends nothing.
      await host.resolveMcpConfirmation('never', false);
      expect(resolveCalls()).toHaveLength(1);
    }
  });
});

describe('ModuleMcpConfirm dialog', () => {
  it('renders nothing while nothing is pending', () => {
    const { host } = createHarness();
    render(<ModuleMcpConfirm host={host} />);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('shows module, server, tool and the exact arguments JSON', async () => {
    const { host, send } = createHarness();
    await host.ensureAttached();
    render(<ModuleMcpConfirm host={host} />);
    await send(confirmInstruction());
    const dialog = await screen.findByRole('dialog');
    expect(dialog.textContent).toContain('Example Module');
    expect(dialog.textContent).toContain('server-7');
    expect(dialog.textContent).toContain('echo');
    const shown = within(dialog).getByLabelText('调用参数');
    expect(shown.textContent).toBe(JSON.stringify(ARGUMENTS, null, 2));
    expect(JSON.parse(shown.textContent ?? '')).toEqual(ARGUMENTS);
  });

  it('Approve and Reject invoke the command with { confirmationId, approved } and then close', async () => {
    const { host, send, resolveCalls } = createHarness();
    await host.ensureAttached();
    render(<ModuleMcpConfirm host={host} />);

    await send(confirmInstruction());
    fireEvent.click(await screen.findByRole('button', { name: '批准' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(resolveCalls()).toEqual([['resolve_module_mcp_confirmation', { confirmationId: 'c1', approved: true }]]);

    await send(confirmInstruction({ confirmationId: 'c2' }));
    fireEvent.click(await screen.findByRole('button', { name: '拒绝' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(resolveCalls()[1]).toEqual(['resolve_module_mcp_confirmation', { confirmationId: 'c2', approved: false }]);
    expect(resolveCalls()).toHaveLength(2);
  });

  it('Esc rejects exactly once', async () => {
    const { host, send, resolveCalls } = createHarness();
    await host.ensureAttached();
    render(<ModuleMcpConfirm host={host} />);
    await send(confirmInstruction());
    await screen.findByRole('dialog');
    fireEvent.keyDown(document.activeElement ?? document.body, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(resolveCalls()).toEqual([['resolve_module_mcp_confirmation', { confirmationId: 'c1', approved: false }]]);
  });

  it('a closed instruction removes the dialog without answering, and the next request is shown', async () => {
    const { host, send, resolveCalls } = createHarness();
    await host.ensureAttached();
    render(<ModuleMcpConfirm host={host} />);
    await send(confirmInstruction());
    await send(confirmInstruction({ confirmationId: 'c2', toolName: 'second' }));
    const dialog = await screen.findByRole('dialog');
    expect(dialog.textContent).toContain('echo');
    expect(dialog.textContent).toContain('1 个请求等待确认');

    await send({ kind: 'confirmMcpClosed', confirmationId: 'c1' });
    await waitFor(() => expect(screen.getByRole('dialog').textContent).toContain('second'));
    await send({ kind: 'confirmMcpClosed', confirmationId: 'c2' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(resolveCalls()).toHaveLength(0);
  });

  it('renders arguments as text, never as markup', async () => {
    const { host, send } = createHarness();
    await host.ensureAttached();
    render(<ModuleMcpConfirm host={host} />);
    await send(confirmInstruction({ moduleName: '<img src=x>', arguments: { html: '<b>bold</b>' } }));
    const dialog = await screen.findByRole('dialog');
    expect(dialog.querySelector('img')).toBeNull();
    expect(dialog.querySelector('b')).toBeNull();
    expect(dialog.textContent).toContain('<b>bold</b>');
  });
});

describe('module code cannot reach confirmations', () => {
  it('the host object of a module has no confirmation or resolve surface, and a module call carries no token', async () => {
    const { host, invoke, send } = createHarness();
    await host.ensureAttached();
    await send({
      kind: 'load',
      ticket: 't1',
      runtimeId: 'rt-1',
      moduleId: 'dgos.mcp-caller',
      version: '1.0.0',
      source: readFixture('mcp-caller.js'),
      hookTimeoutMs: 500,
    });
    await send({ kind: 'route', runtimeId: 'rt-1', route: 'route-1' });
    const scope = globalThis as typeof globalThis & {
      __dgosMcpCallers?: Record<string, { host: Record<string, unknown>; call: (...args: unknown[]) => Promise<unknown> }>;
    };
    const module = scope.__dgosMcpCallers?.['dgos.mcp-caller'];
    expect(module).toBeDefined();
    const moduleHost = module!.host;

    // Everything the module can reach, searched for anything confirmation related.
    const seen = new Set<unknown>();
    const names: string[] = [];
    const walk = (value: unknown, path: string) => {
      if (typeof value !== 'object' && typeof value !== 'function') return;
      if (value === null || seen.has(value) || path.split('.').length > 3) return;
      seen.add(value);
      for (const key of Object.keys(value as object)) {
        if (key === 'React' || key === 'ui') continue;
        names.push(`${path}.${key}`);
        walk((value as Record<string, unknown>)[key], `${path}.${key}`);
      }
    };
    walk(moduleHost, 'host');
    expect(names.filter((name) => /confirm|resolve|token/i.test(name))).toEqual([]);
    expect(Object.keys(moduleHost.mcp as object)).toEqual(['callTool']);

    // A pending confirmation is not visible to the module either.
    await send(confirmInstruction({ arguments: { secret: 'only-for-the-dialog' } }));
    expect(JSON.stringify(Object.keys(moduleHost))).not.toContain('onfirm');

    await module!.call('server-7', 'echo', { text: 'x' });
    const call = invoke.mock.calls.find(([command]) => command === 'module_call_mcp_tool');
    expect(call?.[1]).toEqual({
      serverId: 'server-7',
      toolName: 'echo',
      arguments: { text: 'x' },
      attachmentId: 'att-1',
      route: 'route-1',
      callId: expect.any(String),
    });
    expect(JSON.stringify(call?.[1])).not.toMatch(/token|confirmation/i);
  });
});

afterEach(() => {
  act(() => Modal.closeAll());
});
