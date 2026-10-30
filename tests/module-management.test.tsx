import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MemoryRouter } from 'react-router-dom';
import { ToastViewport, clearToasts } from '@dgos/ui';
import App from '../src/App';

interface Call {
  command: string;
  args: Record<string, unknown> | undefined;
}

const state = vi.hoisted(() => ({
  calls: [] as Array<{ command: string; args: Record<string, unknown> | undefined }>,
  modules: [] as Array<Record<string, any>>,
  crashes: [] as Array<Record<string, unknown>>,
  failures: {} as Record<string, { kind: string; code: number }>,
  commandErrors: {} as Record<string, { code: number; message: string }>,
  operations: new Map<string, Record<string, unknown>>(),
}));

// A controllable stand-in for the runtime host singleton: the tests push instances in and out.
const fakeHost = vi.hoisted(() => {
  let snapshot: readonly unknown[] = [];
  const listeners = new Set<() => void>();
  return {
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    getSnapshot: () => snapshot,
    ensureAttached: async () => null,
    set(next: readonly unknown[]) {
      snapshot = next;
      listeners.forEach((listener) => listener());
    },
  };
});

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), Channel: class {} }));
vi.mock('../src/modules/moduleRuntimeHost', () => ({ moduleRuntimeHost: fakeHost }));
vi.mock('../src/pages/ProviderManagement', () => ({ default: () => null }));
vi.mock('../src/pages/SkillsManagement', () => ({ default: () => null }));
vi.mock('../src/pages/Workbench', () => ({ default: () => null }));
vi.mock('../src/pages/McpManagement', () => ({ default: () => null }));

import { invoke } from '@tauri-apps/api/core';

const mockedInvoke = vi.mocked(invoke);

const OUTCOMES: Record<string, string> = {
  start: 'started',
  stop: 'stopped',
  update: 'updated',
  enable: 'enabled',
  disable: 'disabled',
  uninstall: 'uninstalled',
};
const LIFECYCLE = ['start_module', 'stop_module', 'update_module', 'enable_module', 'disable_module', 'uninstall_module'];

function makeModule(overrides: Record<string, unknown> = {}) {
  const moduleId = (overrides.module_id as string | undefined) ?? 'dgos.beta';
  return {
    module_id: moduleId,
    name: 'Beta',
    version: '1.0.0',
    integrity: 'sha256:fixture',
    official: false,
    enabled: true,
    permissions: [{ permission: 'providers.call', granted: true }],
    runtime: {
      module_id: moduleId,
      state: 'ready',
      runtime_id: 'rt-1',
      version: '1.0.0',
      integrity: 'sha256:fixture',
      in_flight: 0,
      pending_releases: 0,
      active_operation_id: null,
      executes_module_code: true,
    },
    recent_crashes: 0,
    ...overrides,
  };
}

function ok(data: unknown) {
  return { success: true, data, error: null };
}

function fail(code: number, message: string) {
  return { success: false, data: null, error: { code, message } };
}

function lifecycleCommands(): string[] {
  return state.calls.map((call: Call) => call.command).filter((command) => LIFECYCLE.includes(command));
}

function installInvoke() {
  mockedInvoke.mockImplementation(async (command: string, args?: unknown) => {
    const params = args as Record<string, unknown> | undefined;
    state.calls.push({ command, args: params });
    if (command === 'list_modules') return ok(structuredClone(state.modules));
    if (command === 'list_module_crashes') return ok(state.crashes);
    if (command === 'list_mcp_servers') return [];
    if (command === 'grant_module_permission' || command === 'revoke_module_permission') {
      const failure = state.commandErrors[command];
      if (failure) return fail(failure.code, failure.message);
      const module = state.modules.find((entry) => entry.module_id === params?.moduleId);
      const entry = module?.permissions.find((item: { permission: string }) => item.permission === params?.permission);
      if (entry) entry.granted = command === 'grant_module_permission';
      return ok(module);
    }
    if (LIFECYCLE.includes(command)) {
      const commandError = state.commandErrors[command];
      if (commandError) return fail(commandError.code, commandError.message);
      const kind = command.replace('_module', '');
      const failure = state.failures[command] ?? null;
      const operationId = `op-${state.operations.size + 1}`;
      state.operations.set(operationId, {
        operation_id: operationId,
        module_id: params?.moduleId,
        kind,
        state: failure ? 'failed' : 'succeeded',
        outcome: failure ? null : OUTCOMES[kind],
        failure: failure ? { ...failure, retryable: false } : null,
        submitted_at: 1,
        finished_at: 2,
      });
      if (!failure) {
        const module = state.modules.find((entry) => entry.module_id === params?.moduleId);
        if (module && kind === 'enable') module.enabled = true;
        if (module && kind === 'disable') module.enabled = false;
        if (kind === 'uninstall') state.modules = state.modules.filter((entry) => entry.module_id !== params?.moduleId);
      }
      return ok({ operation_id: operationId });
    }
    if (command === 'get_module_operation') return ok(state.operations.get(params?.operationId as string));
    return ok(null);
  });
}

function renderAt(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]} future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>
      <App />
    </MemoryRouter>,
  );
}

function instanceFor(moduleId: string, crashed: boolean) {
  return { runtimeId: `rt-${moduleId}`, moduleId, version: '1.0.0', container: document.createElement('div'), crashed };
}

function card(moduleId: string) {
  return within(screen.getByTestId(`module-${moduleId}`));
}

beforeEach(() => {
  state.calls = [];
  state.crashes = [];
  state.failures = {};
  state.commandErrors = {};
  state.operations = new Map();
  state.modules = [
    makeModule({
      module_id: 'dgos.alpha',
      name: 'Alpha',
      official: true,
      enabled: false,
      permissions: [
        { permission: 'providers.call', granted: false },
        { permission: 'storage.read', granted: true },
      ],
      runtime: { ...makeModule().runtime, module_id: 'dgos.alpha', state: 'not_loaded', runtime_id: null },
    }),
    makeModule({ recent_crashes: 2 }),
  ];
  installInvoke();
});

afterEach(() => {
  cleanup();
  clearToasts();
  act(() => fakeHost.set([]));
  vi.clearAllMocks();
});

describe('Module management list', () => {
  it('renders every module with version, badges, runtime state, permissions and crash count', async () => {
    renderAt('/settings/modules');

    expect(await screen.findByRole('heading', { name: 'Alpha' })).toBeTruthy();
    const alpha = card('dgos.alpha');
    expect(alpha.getByText('Official')).toBeTruthy();
    expect(alpha.getByText('Disabled')).toBeTruthy();
    expect(alpha.getByText('Version 1.0.0')).toBeTruthy();
    expect(alpha.getByText('Runtime: not_loaded')).toBeTruthy();
    expect(alpha.getByText('providers.call').parentElement?.textContent).toContain('Not granted');
    expect(alpha.getByText('storage.read').parentElement?.textContent).toContain('Granted');

    const beta = card('dgos.beta');
    expect(beta.queryByText('Official')).toBeNull();
    expect(beta.getByText('Enabled')).toBeTruthy();
    expect(beta.getByText('Runtime: ready')).toBeTruthy();
    expect(beta.getByText(/Recent crashes: 2/)).toBeTruthy();
  });

  it('is reachable from the sidebar and from the Settings page', async () => {
    renderAt('/settings');

    expect(await screen.findByRole('link', { name: 'Module management' })).toBeTruthy();
    fireEvent.click(screen.getByRole('link', { name: 'Modules' }));
    expect(await screen.findByRole('heading', { name: 'Module management' })).toBeTruthy();
  });

  it('asks for a confirmation before enabling, then enables, grants and starts in order', async () => {
    renderAt('/settings/modules');
    await screen.findByRole('heading', { name: 'Alpha' });

    fireEvent.click(card('dgos.alpha').getByRole('button', { name: 'Enable' }));
    const dialog = within(await screen.findByRole('dialog', { name: 'Enable Alpha' }));
    expect(dialog.getByText('providers.call')).toBeTruthy();
    expect(dialog.getByText('storage.read')).toBeTruthy();
    expect(lifecycleCommands()).toEqual([]);

    const confirm = dialog.getByRole('button', { name: 'Confirm enable' });
    expect((confirm as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(dialog.getByRole('checkbox'));
    expect((confirm as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(confirm);

    await waitFor(() => expect(card('dgos.alpha').getByRole('button', { name: 'Disable' })).toBeTruthy());
    const sequence = state.calls
      .map((call: Call) => call.command)
      .filter((command) => command === 'grant_module_permission' || LIFECYCLE.includes(command));
    expect(sequence).toEqual(['enable_module', 'grant_module_permission', 'start_module']);
    expect(state.calls.find((call: Call) => call.command === 'grant_module_permission')?.args).toEqual({
      moduleId: 'dgos.alpha',
      permission: 'providers.call',
    });
  });

  it('hides Uninstall for official modules and confirms it for the others', async () => {
    renderAt('/settings/modules');
    await screen.findByRole('heading', { name: 'Alpha' });

    expect(card('dgos.alpha').queryByRole('button', { name: 'Uninstall' })).toBeNull();

    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Uninstall' }));
    expect(lifecycleCommands()).toEqual([]);
    const dialog = within(await screen.findByRole('dialog', { name: 'Uninstall Beta' }));
    fireEvent.click(dialog.getByRole('button', { name: 'Confirm uninstall' }));

    await waitFor(() => expect(screen.queryByTestId('module-dgos.beta')).toBeNull());
    expect(lifecycleCommands()).toEqual(['uninstall_module']);
  });

  it('says the previous version was restored when an update fails', async () => {
    state.failures.update_module = { kind: 'update_failed', code: 1004013 };
    renderAt('/settings/modules');
    await screen.findByRole('heading', { name: 'Beta' });

    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Update' }));

    expect(await screen.findByText('Update failed, the previous version was restored')).toBeTruthy();
    expect(screen.getByText(/code 1004013/)).toBeTruthy();
  });

  it('warns that an incompatible module needs a newer or older host', async () => {
    state.failures.start_module = { kind: 'module_incompatible', code: 1004022 };
    renderAt('/settings/modules');
    await screen.findByRole('heading', { name: 'Beta' });

    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Reload' }));

    expect(await screen.findByText(/newer or older host version/)).toBeTruthy();
    expect(screen.getByText(/code 1004022/)).toBeTruthy();
  });

  it('reports a failed action inline next to its module and once as a Toast that carries only the code', async () => {
    const logged = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    state.failures.update_module = { kind: 'update_failed', code: 1004013 };
    renderAt('/settings/modules');
    render(<ToastViewport />);
    await screen.findByRole('heading', { name: 'Beta' });

    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Update' }));

    const inline = await card('dgos.beta').findByRole('alert');
    expect(within(inline).getByText('Update failed, the previous version was restored')).toBeTruthy();
    expect(within(inline).getByText(/code 1004013/)).toBeTruthy();
    // 1004xxx has no registry entry (FR-005 §13.2): the Toast is the generic text with the code, never the host message.
    expect(await screen.findAllByText('操作失败（错误码 1004013）')).toHaveLength(1);
    expect(logged).toHaveBeenCalledWith('[dgos] error', { code: 1004013, where: 'ModuleManagement' });
    logged.mockRestore();
  });

  it('reloads with stop followed by start', async () => {
    renderAt('/settings/modules');
    await screen.findByRole('heading', { name: 'Beta' });

    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Reload' }));

    expect(await screen.findByText('Reloaded.')).toBeTruthy();
    expect(lifecycleCommands()).toEqual(['stop_module', 'start_module']);
  });

  it('shows the host message and code for a failed permission change', async () => {
    state.commandErrors.revoke_module_permission = { code: 1004004, message: 'Denied by the host' };
    renderAt('/settings/modules');
    await screen.findByRole('heading', { name: 'Beta' });

    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Revoke providers.call for Beta' }));

    expect(await screen.findByText('Denied by the host')).toBeTruthy();
    expect(screen.getByText(/code 1004004/)).toBeTruthy();
  });

  it('lists the crash log with time, source, error name and stack', async () => {
    state.crashes = [
      {
        crash_id: 'c1',
        module_id: 'dgos.beta',
        module_version: '1.0.0',
        source: 'render',
        error_name: 'TypeError',
        stack: 'render@module:1:2',
        created_at: 1_700_000_000,
      },
    ];
    renderAt('/settings/modules');
    await screen.findByRole('heading', { name: 'Beta' });

    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Crash log' }));

    expect(await screen.findByText('TypeError')).toBeTruthy();
    expect(screen.getByText('render@module:1:2')).toBeTruthy();
    expect(screen.getByText(/· render ·/)).toBeTruthy();
    expect(state.calls.find((call: Call) => call.command === 'list_module_crashes')?.args).toEqual({ moduleId: 'dgos.beta' });
  });
});

describe('Module panel', () => {
  it('says the module is not running when there is no live instance', async () => {
    renderAt('/settings/modules/dgos.beta');

    expect(await screen.findByText('This module is not running')).toBeTruthy();
    expect(screen.getByRole('link', { name: 'Back to module management' })).toBeTruthy();
  });

  it('shows a crash banner whose Reload module stops and starts the module', async () => {
    renderAt('/settings/modules/dgos.beta');
    act(() => fakeHost.set([instanceFor('dgos.beta', true)]));

    expect(await screen.findByText('Module dgos.beta crashed')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'View details' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Reload module' }));

    await waitFor(() => expect(lifecycleCommands()).toEqual(['stop_module', 'start_module']));
  });

  it('keeps the crash banner when the host marks the runtime failed and the instance is already released', async () => {
    state.modules = [
      makeModule({ recent_crashes: 1, runtime: { ...makeModule().runtime, state: 'failed' } }),
    ];
    renderAt('/settings/modules/dgos.beta');

    // No live instance in the page, but the host recorded a crash and failed the runtime.
    expect(await screen.findByText('Module dgos.beta crashed')).toBeTruthy();
    expect(screen.getByText('This module is not running')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Reload module' }));
    await waitFor(() => expect(lifecycleCommands()).toEqual(['stop_module', 'start_module']));
  });

  it('shows no crash banner for a failed runtime that never recorded a crash', async () => {
    state.modules = [makeModule({ recent_crashes: 0, runtime: { ...makeModule().runtime, state: 'failed' } })];
    renderAt('/settings/modules/dgos.beta');

    expect(await screen.findByText('This module is not running')).toBeTruthy();
    expect(screen.queryByText('Module dgos.beta crashed')).toBeNull();
  });

  it('uninstalls a crashed module after confirmation and leaves the panel', async () => {
    renderAt('/settings/modules/dgos.beta');
    act(() => fakeHost.set([instanceFor('dgos.beta', true)]));
    await screen.findByText('Module dgos.beta crashed');

    await waitFor(() => expect((screen.getByRole('button', { name: 'Uninstall' }) as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(screen.getByRole('button', { name: 'Uninstall' }));
    fireEvent.click(within(await screen.findByRole('dialog', { name: 'Uninstall dgos.beta' })).getByRole('button', { name: 'Confirm uninstall' }));

    expect(await screen.findByRole('heading', { name: 'Module management' })).toBeTruthy();
    expect(lifecycleCommands()).toEqual(['uninstall_module']);
  });

  it('disables Uninstall in the crash banner of an official module', async () => {
    renderAt('/settings/modules/dgos.alpha');
    act(() => fakeHost.set([instanceFor('dgos.alpha', true)]));
    await screen.findByText('Module dgos.alpha crashed');

    await waitFor(() => expect(screen.getByRole('heading', { name: 'Alpha' })).toBeTruthy());
    expect((screen.getByRole('button', { name: 'Uninstall' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('warns that a module with three recorded crashes may be faulty', async () => {
    state.modules = [makeModule({ recent_crashes: 3, enabled: false })];
    renderAt('/settings/modules/dgos.beta');

    expect(await screen.findByText('This module may be faulty; it was disabled. Consider uninstalling it.')).toBeTruthy();
  });
});

describe('Sidebar module entries', () => {
  it('adds a link for a ready instance and removes it after unload', async () => {
    renderAt('/settings');
    await screen.findByText('Settings Page');
    expect(screen.queryByRole('link', { name: 'dgos.beta' })).toBeNull();

    act(() => fakeHost.set([instanceFor('dgos.beta', false)]));
    const link = await screen.findByRole('link', { name: 'dgos.beta' });
    expect(link.getAttribute('href')).toBe('/settings/modules/dgos.beta');

    act(() => fakeHost.set([]));
    await waitFor(() => expect(screen.queryByRole('link', { name: 'dgos.beta' })).toBeNull());
  });
});
