import { setLocaleForTesting } from '../src/i18n/runtime';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MemoryRouter } from 'react-router-dom';
import { ToastViewport, clearToasts, disposeAppearance } from '@dgos/ui';
import App from '../src/App';
// Resolve the real lazy page before assertions; cold Vitest transforms are fixture setup.
import '../src/pages/SettingsShell';
import { DEFAULTS } from '../src/ui/appearance/model';

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
  grantFailures: {} as Record<string, { code: number; message: string }>,
  audits: [] as Array<Record<string, unknown>>,
  auditPageSize: 2,
  afterUpdate: null as null | (() => void),
  updateOutcome: 'updated',
}));

// A controllable stand-in for the runtime host singleton: the tests push instances in and out.
const fakeHost = vi.hoisted(() => {
  let snapshot: readonly unknown[] = [];
  // A stable empty snapshot: `useSyncExternalStore` requires the same reference while nothing changed.
  const NO_CONFIRMATIONS: readonly unknown[] = [];
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
    // The host-owned MCP confirmation dialog (D093) is mounted next to the module slots; none is pending here.
    subscribeMcpConfirmations: () => () => {},
    getMcpConfirmations: () => NO_CONFIRMATIONS,
    resolveMcpConfirmation: async () => {},
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
vi.mock('../src/components/chat/ChatWorkbench', () => ({ default: () => null }));
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
    if (command === 'get_appearance_preferences') return ok({ kind: 'ready', snapshot: { ...structuredClone(DEFAULTS), revision: 1 }, repairWarnings: [] });
    if (command === 'list_modules') return ok(structuredClone(state.modules));
    if (command === 'read_workspace_app_catalog') return ok({ catalogRevision: 'test', entries: state.modules.map(module => ({ moduleId: module.module_id, displayName: module.name, iconResourceId: null, layout: 'document', viewCapability: 'single', enabled: module.enabled, permissionState: module.permissions.every((permission: {granted: boolean}) => permission.granted) ? 'granted' : 'pending', runtimeState: module.runtime.state, crashed: module.runtime.state === 'failed' })) });
    if (command === 'list_module_crashes') return ok(state.crashes);
    if (command === 'list_mcp_servers') return [];
    if (command === 'grant_module_permission' || command === 'revoke_module_permission') {
      const failure = state.commandErrors[command];
      if (failure) return fail(failure.code, failure.message);
      const permissionFailure = state.grantFailures[params?.permission as string];
      if (command === 'grant_module_permission' && permissionFailure) {
        return fail(permissionFailure.code, permissionFailure.message);
      }
      const module = state.modules.find((entry) => entry.module_id === params?.moduleId);
      const entry = module?.permissions.find((item: { permission: string }) => item.permission === params?.permission);
      if (entry) entry.granted = command === 'grant_module_permission';
      return ok(module);
    }
    if (LIFECYCLE.includes(command)) {
      if (command === 'update_module' && !state.failures[command] && !state.commandErrors[command]) state.afterUpdate?.();
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
        outcome: failure ? null : kind === 'update' ? state.updateOutcome : OUTCOMES[kind],
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
    if (command === 'list_module_permission_audits') {
      const commandError = state.commandErrors[command];
      if (commandError) return fail(commandError.code, commandError.message);
      // The cursor is opaque to the page; the fake encodes the start index.
      const start = params?.before === undefined ? 0 : Number(params.before);
      const end = Math.min(start + state.auditPageSize, state.audits.length);
      return ok({ entries: state.audits.slice(start, end), next_before: end < state.audits.length ? String(end) : null });
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
  setLocaleForTesting('en-US');
  state.calls = [];
  state.crashes = [];
  state.failures = {};
  state.commandErrors = {};
  state.operations = new Map();
  state.grantFailures = {};
  state.audits = [];
  state.auditPageSize = 2;
  state.afterUpdate = null;
  state.updateOutcome = 'updated';
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
  disposeAppearance();
  clearToasts();
  act(() => fakeHost.set([]));
  vi.clearAllMocks();
});

describe('Module management list', () => {

  it('is reachable from the sidebar and from the Modules tab of the Settings shell', async () => {
    renderAt('/settings');

    fireEvent.mouseDown(await screen.findByRole('tab', { name: 'Modules' }), { button: 0 });
    expect(await screen.findByRole('heading', { name: 'Module management' })).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Close settings' }));
    await screen.findByRole('link', { name: 'Settings' });
    fireEvent.click(screen.getByRole('link', { name: 'Settings' }));
    fireEvent.mouseDown(await screen.findByRole('tab', { name: 'Modules' }), { button: 0 });
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

  it('AC11 updates a module from the Settings modules tab and shows the new version', async () => {
    state.afterUpdate = () => {
      state.modules[1].version = '1.1.0';
    };
    renderAt('/settings');
    fireEvent.mouseDown(await screen.findByRole('tab', { name: 'Modules' }), { button: 0 });
    await screen.findByRole('heading', { name: 'Beta' });
    expect(card('dgos.beta').getByText('Version 1.0.0')).toBeTruthy();

    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Update' }));

    expect(await card('dgos.beta').findByText('Updated.')).toBeTruthy();
    expect(await card('dgos.beta').findByText('Version 1.1.0')).toBeTruthy();
    expect(lifecycleCommands()).toEqual(['update_module']);
    expect(state.calls.find((call: Call) => call.command === 'update_module')?.args).toEqual({ moduleId: 'dgos.beta' });
  });

  it('warns that an incompatible module needs a newer or older host', async () => {
    state.failures.start_module = { kind: 'module_incompatible', code: 1004022 };
    renderAt('/settings/modules');
    await screen.findByRole('heading', { name: 'Beta' });

    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Reload' }));

    expect(await screen.findByText(/newer or older host version/)).toBeTruthy();
    expect(screen.getByText(/code 1004022/)).toBeTruthy();
  });

  it('reports a failed action inline next to its module and once as a Toast with the registered text, never the host message', async () => {
    const logged = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    state.failures.update_module = { kind: 'update_failed', code: 1004013 };
    renderAt('/settings/modules');
    render(<ToastViewport />);
    await screen.findByRole('heading', { name: 'Beta' });

    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Update' }));

    const inline = await card('dgos.beta').findByRole('alert');
    expect(within(inline).getByText('Update failed, the previous version was restored')).toBeTruthy();
    expect(within(inline).getByText(/code 1004013/)).toBeTruthy();
    // 1004013 is registered (D110): the Toast shows the registered message and advice, never the host message.
    expect(await screen.findAllByText('Operation failed')).toHaveLength(1);
    expect(screen.getByText('Please try again later. Check the logs if the problem persists')).toBeTruthy();
    expect(screen.queryByText('操作失败（错误码 1004013）')).toBeNull();
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

  it('does not start the module when a grant fails, still attempts the other grants, and names the failure', async () => {
    state.modules[0].permissions = [
      { permission: 'providers.call', granted: false },
      { permission: 'mcp.call', granted: false },
      { permission: 'storage.read', granted: true },
    ];
    state.grantFailures['providers.call'] = { code: 1004004, message: 'Denied by the host' };
    renderAt('/settings/modules');
    await screen.findByRole('heading', { name: 'Alpha' });

    fireEvent.click(card('dgos.alpha').getByRole('button', { name: 'Enable' }));
    const dialog = within(await screen.findByRole('dialog', { name: 'Enable Alpha' }));
    fireEvent.click(dialog.getByRole('checkbox'));
    fireEvent.click(dialog.getByRole('button', { name: 'Confirm enable' }));

    const alert = within(await card('dgos.alpha').findByRole('alert'));
    expect(alert.getByText('providers.call')).toBeTruthy();
    expect(alert.getByText(/Denied by the host/)).toBeTruthy();
    expect(alert.getByText(/code 1004004/)).toBeTruthy();
    expect(alert.getByText(/Grant the remaining permissions with their Grant buttons/)).toBeTruthy();

    const sequence = state.calls
      .filter((call: Call) => call.command === 'grant_module_permission' || LIFECYCLE.includes(call.command))
      .map((call: Call) => (call.command === 'grant_module_permission' ? `grant:${call.args?.permission}` : call.command));
    expect(sequence).toEqual(['enable_module', 'grant:providers.call', 'grant:mcp.call']);
    expect(lifecycleCommands()).not.toContain('start_module');
    // The grant that worked stays; the failed one can be retried with its own button.
    expect(card('dgos.alpha').getByRole('button', { name: 'Revoke mcp.call for Alpha' })).toBeTruthy();

    delete state.grantFailures['providers.call'];
    fireEvent.click(card('dgos.alpha').getByRole('button', { name: 'Grant providers.call for Alpha' }));
    expect(await card('dgos.alpha').findByRole('button', { name: 'Revoke providers.call for Alpha' })).toBeTruthy();
  });

  it('starts the module with Reload after the failed grant is retried, even though stop finds no runtime', async () => {
    state.modules[0].permissions = [
      { permission: 'providers.call', granted: false },
      { permission: 'mcp.call', granted: false },
    ];
    state.grantFailures['providers.call'] = { code: 1004004, message: 'Denied by the host' };
    // The module was never started, so the host answers stop with not_running (1004016).
    state.failures.stop_module = { kind: 'not_running', code: 1004016 };
    renderAt('/settings/modules');
    await screen.findByRole('heading', { name: 'Alpha' });

    fireEvent.click(card('dgos.alpha').getByRole('button', { name: 'Enable' }));
    const dialog = within(await screen.findByRole('dialog', { name: 'Enable Alpha' }));
    fireEvent.click(dialog.getByRole('checkbox'));
    fireEvent.click(dialog.getByRole('button', { name: 'Confirm enable' }));

    const alert = within(await card('dgos.alpha').findByRole('alert'));
    expect(alert.getByText(/then press Reload to start the module/)).toBeTruthy();
    expect(alert.queryByText(/disabled and/)).toBeNull();
    expect(lifecycleCommands()).toEqual(['enable_module']);

    delete state.grantFailures['providers.call'];
    fireEvent.click(card('dgos.alpha').getByRole('button', { name: 'Grant providers.call for Alpha' }));
    expect(await card('dgos.alpha').findByRole('button', { name: 'Revoke providers.call for Alpha' })).toBeTruthy();
    expect(lifecycleCommands()).toEqual(['enable_module']);

    fireEvent.click(card('dgos.alpha').getByRole('button', { name: 'Reload' }));
    expect(await screen.findByText('Reloaded.')).toBeTruthy();
    expect(lifecycleCommands()).toEqual(['enable_module', 'stop_module', 'start_module']);
  });

  it('still reports a stop failure other than not_running and does not start', async () => {
    state.failures.stop_module = { kind: 'unload_failed', code: 1004007 };
    renderAt('/settings/modules');
    await screen.findByRole('heading', { name: 'Beta' });

    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Reload' }));

    expect(await screen.findByText(/code 1004007/)).toBeTruthy();
    expect(lifecycleCommands()).toEqual(['stop_module']);
  });

  it('leaves the module disabled when the enable confirmation is cancelled', async () => {
    renderAt('/settings/modules');
    await screen.findByRole('heading', { name: 'Alpha' });

    fireEvent.click(card('dgos.alpha').getByRole('button', { name: 'Enable' }));
    const dialog = within(await screen.findByRole('dialog', { name: 'Enable Alpha' }));
    fireEvent.click(dialog.getByRole('button', { name: 'Cancel' }));

    expect(screen.queryByRole('dialog', { name: 'Enable Alpha' })).toBeNull();
    expect(card('dgos.alpha').getByText('Disabled')).toBeTruthy();
    expect(card('dgos.alpha').getByRole('button', { name: 'Enable' })).toBeTruthy();
    expect(state.calls.filter((call: Call) => call.command === 'grant_module_permission' || LIFECYCLE.includes(call.command))).toEqual([]);
  });

  it.each([
    ['updated', 'Updated.'],
    ['updated_pending_release', 'Updated. The new version takes over once the old one is released.'],
  ])('lists newly declared and withdrawn permissions after an update (%s), with a Grant action for the new one', async (outcome, message) => {
    state.updateOutcome = outcome;
    state.modules[1].permissions = [
      { permission: 'providers.call', granted: true },
      { permission: 'storage.write', granted: true },
    ];
    state.afterUpdate = () => {
      state.modules[1].permissions = [
        { permission: 'providers.call', granted: true },
        { permission: 'mcp.call', granted: false },
      ];
    };
    renderAt('/settings/modules');
    await screen.findByRole('heading', { name: 'Beta' });

    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Update' }));

    expect(await card('dgos.beta').findByText('Permissions changed in this update.')).toBeTruthy();
    expect(await card('dgos.beta').findByText(message)).toBeTruthy();
    expect(card('dgos.beta').getByText(/Newly declared, not granted yet/)).toBeTruthy();
    expect(card('dgos.beta').getByText(/your grant was withdrawn/)).toBeTruthy();
    // storage.write is gone from the permission list, so the only place it appears is the withdrawn list.
    expect(card('dgos.beta').getAllByText('storage.write')).toHaveLength(1);

    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Grant new permission mcp.call for Beta' }));
    await waitFor(() => expect(card('dgos.beta').getByRole('button', { name: 'Revoke mcp.call for Beta' })).toBeTruthy());
    expect(card('dgos.beta').queryByText(/Newly declared, not granted yet/)).toBeNull();
    expect(card('dgos.beta').getByText(/your grant was withdrawn/)).toBeTruthy();
  });

  it('shows no permission change notice when an update leaves the declared permissions as they were', async () => {
    renderAt('/settings/modules');
    await screen.findByRole('heading', { name: 'Beta' });

    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Update' }));

    expect(await screen.findByText('Updated.')).toBeTruthy();
    expect(screen.queryByText('Permissions changed in this update.')).toBeNull();
  });

  it('shows the permission log newest first and pages older entries with the host cursor', async () => {
    state.audits = [
      { module_id: 'dgos.beta', permission: 'providers.call', action: 'revoked', created_at: 1_700_000_300 },
      { module_id: 'dgos.beta', permission: 'providers.call', action: 'granted', created_at: 1_700_000_200 },
      { module_id: 'dgos.beta', permission: 'storage.read', action: 'denied', created_at: 1_700_000_100 },
    ];
    renderAt('/settings/modules');
    await screen.findByRole('heading', { name: 'Beta' });

    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Permission log' }));

    const region = within(await screen.findByRole('region', { name: 'Permission log' }));
    let items = region.getAllByRole('listitem');
    expect(items).toHaveLength(2);
    expect(items[0].textContent).toContain('revoked');
    expect(items[1].textContent).toContain('granted');

    fireEvent.click(region.getByRole('button', { name: 'Load older' }));
    await waitFor(() => expect(region.getAllByRole('listitem')).toHaveLength(3));
    items = region.getAllByRole('listitem');
    expect(items[2].textContent).toContain('storage.read');
    expect(items[2].textContent).toContain('denied');
    expect(region.queryByRole('button', { name: 'Load older' })).toBeNull();

    const audits = state.calls.filter((call: Call) => call.command === 'list_module_permission_audits');
    expect(audits.map((call: Call) => call.args)).toEqual([{ moduleId: 'dgos.beta' }, { moduleId: 'dgos.beta', before: '2' }]);

    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Permission log' }));
    expect(screen.queryByRole('region', { name: 'Permission log' })).toBeNull();
  });

  it('shows an empty permission log and reports a failed permission log request like other notices', async () => {
    renderAt('/settings/modules');
    await screen.findByRole('heading', { name: 'Beta' });

    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Permission log' }));
    expect(await screen.findByText('No permission activity recorded.')).toBeTruthy();
    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Permission log' }));

    state.commandErrors.list_module_permission_audits = { code: 1004024, message: 'Invalid cursor' };
    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Permission log' }));
    expect(await screen.findByText('Invalid cursor')).toBeTruthy();
    expect(screen.getByText(/code 1004024/)).toBeTruthy();
    expect(screen.queryByRole('region', { name: 'Permission log' })).toBeNull();
  });
});

describe('Module crash recovery in management', () => {
  it('keeps crash recovery available after the runtime has been released', async () => {
    state.modules = [makeModule({ recent_crashes: 1, runtime: { ...makeModule().runtime, state: 'failed' } })];
    renderAt('/settings/modules');
    await screen.findByRole('heading', { name: 'Beta' });
    expect(card('dgos.beta').getByRole('alert')).toBeTruthy();
    expect(card('dgos.beta').getByRole('alert').textContent).toContain('dgos.beta');
    fireEvent.click(card('dgos.beta').getByRole('button', { name: 'Reload' }));
    await waitFor(() => expect(lifecycleCommands()).toEqual(['stop_module', 'start_module']));
    expect(card('dgos.beta').getByRole('alert').textContent).toContain('dgos.beta');
  });
  it('keeps installed catalog entries after their runtime unloads', async () => {
    renderAt('/');
    const link = await screen.findByRole('button', { name: 'Beta', exact: true });
    expect((link as HTMLButtonElement).disabled).toBe(false);
    act(() => fakeHost.set([instanceFor('dgos.beta', false)]));
    act(() => fakeHost.set([]));
    expect(screen.getByRole('button', { name: 'Beta', exact: true })).toBe(link);
  });
});
