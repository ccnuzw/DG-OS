import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { BrowserRouter } from 'react-router-dom';
import App from '../src/App';
import { isHostSettingsPath } from '../src/modules/hostRoutes';
import { moduleRuntimeHost } from '../src/modules/moduleRuntimeHost';
import type * as ModuleRuntimeHostModule from '../src/modules/moduleRuntimeHost';
import { hooksControl, readFixture } from './helpers/moduleFixtures';
import { resetLocaleForTesting, setLocaleForTesting } from '../src/i18n/runtime';
import { Modal, ModalHost } from '@dgos/ui';
import { DEFAULTS } from '../src/ui/appearance/model';

const bridge = vi.hoisted(() => ({
  attachCalls: 0,
  channels: [] as Array<{ onmessage: (message: unknown) => void }>,
  reports: [] as Array<{ attachmentId: unknown; ticket: unknown; receipt: unknown }>,
}));

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }));

vi.mock('@tauri-apps/api/core', () => {
  class Channel {
    onmessage: (message: unknown) => void = () => undefined;
    constructor() {
      bridge.channels.push(this);
    }
  }
  async function invoke(command: string, args?: Record<string, unknown>): Promise<unknown> {
    if (command === 'attach_module_host') {
      bridge.attachCalls += 1;
      return { success: true, data: { attachmentId: 'a1' }, error: null };
    }
    if (command === 'report_module_host') {
      bridge.reports.push({ attachmentId: args?.attachmentId, ticket: args?.ticket, receipt: args?.receipt });
      return { success: true, data: { disposition: 'applied' }, error: null };
    }
    if (command === 'list_mcp_servers') return [];
    if (command === 'read_workspace_app_catalog') return { success: true, data: { catalogRevision: 'fixture', entries: [{ moduleId: 'dgos.route', displayName: 'Route app', iconResourceId: null, layout: 'document', viewCapability: 'single', enabled: true, permissionState: 'granted', runtimeState: 'ready', crashed: false }] } };
    if (command === 'get_appearance_preferences') return { success: true, data: { kind: 'ready', snapshot: { ...structuredClone(DEFAULTS), revision: 1 }, repairWarnings: [] } };
    return null;
  }
  return { Channel, invoke };
});

// The singleton keeps its real attach/report path (through the mocked Tauri core above). Only the
// Blob import is replaced, because jsdom/Node cannot import blob: URLs; the fixture source is
// evaluated instead (see evaluateFixtureSource). The real import(blobUrl) is covered by the WebView smoke.
// vi.mock factories are hoisted above the static imports, so the factory bodies below load their
// dependencies with `await import()`; a static binding would still be uninitialised when they run.
vi.mock('../src/modules/moduleRuntimeHost', async (importOriginal) => {
  const actual = await importOriginal<typeof ModuleRuntimeHostModule>();
  const { evaluateFixtureSource } = await import('./helpers/moduleFixtures');
  const sources = new Map<string, string>();
  let sequence = 0;
  const moduleRuntimeHost = new actual.ModuleRuntimeHost({
    createObjectUrl: (source) => {
      const url = `blob:route-test/${++sequence}`;
      sources.set(url, source);
      return url;
    },
    revokeObjectUrl: (url) => {
      sources.delete(url);
    },
    importModule: async (url) => {
      const source = sources.get(url);
      if (source === undefined) throw new Error('unknown blob url');
      return evaluateFixtureSource(source);
    },
  });
  return { ...actual, moduleRuntimeHost };
});

vi.mock('../src/pages/ProviderManagement', async () => {
  const { createElement } = await import('react');
  return { default: () => createElement('div', null, 'Providers Page') };
});
vi.mock('../src/pages/SkillsManagement', async () => {
  const { createElement } = await import('react');
  return { default: () => createElement('div', null, 'Skills Page') };
});
vi.mock('../src/components/chat/ChatWorkbench', async () => {
  const { createElement } = await import('react');
  return { default: () => createElement('div', null, 'Workbench Page') };
});
vi.mock('../src/pages/McpManagement', async () => {
  const { createElement } = await import('react');
  return { default: () => createElement('div', null, 'Mcp Page') };
});

function renderApp() {
  return render(
    <BrowserRouter future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>
      <App />
    </BrowserRouter>,
  );
}

function navLink(href: string) {
  const link = screen.getAllByRole('link').find((item) => item.getAttribute('href') === href);
  if (!link) throw new Error(`Navigation link not found: ${href}`);
  return link;
}

beforeAll(() => {
  Object.assign(window, { __TAURI_INTERNALS__: {} });
});
afterAll(() => {
  Reflect.deleteProperty(window, '__TAURI_INTERNALS__');
});
beforeEach(() => {
  setLocaleForTesting('zh-CN');
  window.history.replaceState(null, '', '/');
  bridge.reports.length = 0;
});
afterEach(async () => {
  cleanup();
  vi.restoreAllMocks();
  // The host singleton outlives each test: unload whatever a test left loaded.
  Modal.closeAll();
  const channel = bridge.channels[0];
  for (const view of moduleRuntimeHost.getSnapshot()) {
    channel.onmessage({ kind: 'unload', ticket: `cleanup-${view.runtimeId}`, runtimeId: view.runtimeId, hookTimeoutMs: 100 });
  }
  await waitFor(() => expect(moduleRuntimeHost.getSnapshot()).toHaveLength(0));
  resetLocaleForTesting();
});

describe('SPA navigation', () => {
  it('opens Settings through pushState without a page load', async () => {
    Object.assign(window, { __spaMarker: 'alive' });
    renderApp();
    const pushState = vi.spyOn(window.history, 'pushState');

    fireEvent.click(navLink('/settings'));

    await screen.findByRole('tablist', { name: '系统设置' });
    expect(pushState).toHaveBeenCalledWith(expect.anything(), '', '/settings');
    expect(window.location.pathname).toBe('/settings/appearance');
    expect(screen.getByRole('tab', { name: '外观', selected: true })).toBeTruthy();
    expect(Reflect.get(window, '__spaMarker')).toBe('alive');
    Reflect.deleteProperty(window, '__spaMarker');
  });

  it('attaches the module host exactly once across SPA navigation', async () => {
    renderApp();
    const before = bridge.attachCalls;
    fireEvent.click(navLink('/workbench'));
    await screen.findByText('Workbench Page');
    fireEvent.click(navLink('/settings'));
    await screen.findByRole('tablist', { name: '系统设置' });
    fireEvent.click(screen.getByRole('button', { name: '关闭设置' }));
    await screen.findByRole('link', { name: '技能中心' });
    fireEvent.click(navLink('/skills'));
    await screen.findByText('Skills Page');
    expect(bridge.attachCalls).toBe(before);
  });

  it('keeps a loaded module alive across route changes and re-attaches the same container', async () => {
    renderApp();
    await waitFor(() => expect(bridge.channels).toHaveLength(1));
    const channel = bridge.channels[0];

    channel.onmessage({
      kind: 'load',
      ticket: 'route-load',
      runtimeId: 'rt-route-1',
      moduleId: 'dgos.route',
      version: '1.0.0',
      source: readFixture('counter.js'),
      hookTimeoutMs: 500,
    });

    // Unpinned apps are opened through the live launcher directory.
    expect(screen.queryByTestId('module-slot')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: '应用', exact: true }));
    fireEvent.click(await within(screen.getByRole('dialog', { name: '应用' })).findByRole('button', { name: 'Route app', exact: true }));
    const button = await screen.findByTestId('fixture-counter');
    expect(window.location.pathname).toBe('/apps/dgos.route');
    expect(screen.getByTestId('module-slot').getAttribute('data-runtime-id')).toBe('rt-route-1');
    await waitFor(() =>
      expect(bridge.reports).toContainEqual({ attachmentId: 'a1', ticket: 'route-load', receipt: { kind: 'loaded' } }),
    );

    fireEvent.click(button);
    await waitFor(() => expect(button.textContent).toBe('count:1'));

    fireEvent.click(navLink('/workbench'));
    await screen.findByText('Workbench Page');
    expect(screen.getByTestId('module-slot').closest('[hidden]')).not.toBeNull();
    const [view] = moduleRuntimeHost.getSnapshot();
    expect(moduleRuntimeHost.getSnapshot()).toHaveLength(1);
    expect(view.runtimeId).toBe('rt-route-1');
    expect(view.container.isConnected).toBe(true);

    fireEvent.click(screen.getByRole('tab', { name: /Route app/ }));
    const returned = await screen.findByTestId('fixture-counter');
    expect(returned).toBe(button);
    expect(returned.textContent).toBe('count:1');
    expect(screen.getByTestId('module-slot').contains(view.container)).toBe(true);

    fireEvent.click(navLink('/settings'));
    await screen.findByRole('tablist', { name: '系统设置' });
    expect(button.isConnected).toBe(true);
    expect(button.closest('[hidden]')).toBeNull();
    expect(button.closest('[inert]')).not.toBeNull();
    expect(moduleRuntimeHost.getSnapshot()[0].container).toBe(view.container);
    fireEvent.click(screen.getByRole('button', { name: '关闭设置' }));
    await waitFor(() => expect(window.location.pathname).toBe('/apps/dgos.route'));
    expect(await screen.findByTestId('fixture-counter')).toBe(button);
    expect(button.closest('[inert]')).toBeNull();
    expect(button.textContent).toBe('count:1');

    channel.onmessage({ kind: 'unload', ticket: 'route-unload', runtimeId: 'rt-route-1', hookTimeoutMs: 500 });
    await waitFor(() => expect(moduleRuntimeHost.getSnapshot()).toHaveLength(0));
    expect(screen.queryByTestId('fixture-counter')).toBeNull();
    expect(screen.queryByTestId('module-slot')).toBeNull();
    expect(screen.getByRole('tab', { name: /Route app/ })).toBeTruthy();
    expect(screen.getByText('应用当前不可用，请在应用管理中检查启用、授权和运行状态。')).toBeTruthy();
  });

  it('updates a hidden module dirty marker and checks the document again when closing', async () => {
    const control = { dirty: false, checks: 0, disposed: 0 };
    hooksControl().seenReact = control;
    renderApp();
    render(<ModalHost />);
    await waitFor(() => expect(bridge.channels).toHaveLength(1));
    bridge.channels[0].onmessage({
      kind: 'load', ticket: 'hidden-dirty', runtimeId: 'rt-hidden', moduleId: 'dgos.route',
      version: '1.0.0', hookTimeoutMs: 500, views: { layout: 'document', capability: 'multiple' },
      source: `export default function createModule(host) {
        const state = globalThis.__dgosHooks.seenReact;
        return { Component: () => null, openView: () => ({
          Component: () => host.React.createElement('div', null, 'Hidden document'),
          beforeClose: () => { state.checks++; return { dirty: state.dirty, canSave: false }; },
          dispose: () => { state.disposed++; }
        }) };
      }`,
    });
    fireEvent.click(screen.getByRole('button', { name: '应用', exact: true }));
    fireEvent.click(await within(screen.getByRole('dialog', { name: '应用' })).findByRole('button', { name: 'Route app', exact: true }));
    const document = await screen.findByText('Hidden document');
    fireEvent.click(navLink('/workbench'));
    expect(document.closest('[hidden]')).not.toBeNull();
    control.dirty = true;
    const tab = screen.getByRole('tab', { name: /Route app/ });
    await waitFor(() => expect(tab.querySelector('.workspace-dirty-dot')).not.toBeNull());
    const checks = control.checks;
    fireEvent.click(screen.getByRole('button', { name: '关闭 Route app' }));
    const dialog = await screen.findByRole('dialog');
    expect(control.checks).toBeGreaterThan(checks);
    fireEvent.click(within(dialog).getByRole('button', { name: '取消', exact: true }));
    expect(document.isConnected).toBe(true);
    expect(control.disposed).toBe(0);
    delete hooksControl().seenReact;
  });

  it('keeps a dirty document on cancel or failed save, and closes only after successful save', async () => {
    const control = { saved: false, attempts: 0, disposed: 0 };
    hooksControl().seenReact = control;
    renderApp();
    await waitFor(() => expect(bridge.channels).toHaveLength(1));
    bridge.channels[0].onmessage({
      kind: 'load', ticket: 'save-load', runtimeId: 'rt-save', moduleId: 'dgos.route',
      version: '1.0.0', hookTimeoutMs: 500,
      views: { layout: 'document', capability: 'multiple' },
      source: `export default function createModule(host) {
        const control = globalThis.__dgosHooks.seenReact;
        return { Component: () => null, openView: () => ({
          Component: () => host.React.createElement('div', null, 'Document draft'),
          beforeClose: () => ({ dirty: true, canSave: true }),
          save: () => { control.attempts++; return control.saved; },
          dispose: () => { control.disposed++; }
        }) };
      }`,
    });
    fireEvent.click(screen.getByRole('button', { name: '应用', exact: true }));
    fireEvent.click(await within(screen.getByRole('dialog', { name: '应用' })).findByRole('button', { name: 'Route app', exact: true }));
    const draft = await screen.findByText('Document draft');
    fireEvent.click(screen.getByRole('button', { name: '关闭 Route app' }));
    let dialog = await screen.findByRole('dialog');
    fireEvent.click(within(dialog).getByRole('button', { name: '取消', exact: true }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(draft.isConnected).toBe(true);
    expect(control.disposed).toBe(0);
    fireEvent.click(screen.getByRole('button', { name: '关闭 Route app' }));
    dialog = await screen.findByRole('dialog');
    fireEvent.click(within(dialog).getByRole('button', { name: '保存并关闭' }));
    await waitFor(() => expect(control.attempts).toBe(1));
    expect(draft.isConnected).toBe(true);
    expect(screen.getByRole('dialog')).toBe(dialog);
    control.saved = true;
    fireEvent.click(within(dialog).getByRole('button', { name: '保存并关闭' }));
    await waitFor(() => expect(screen.queryByRole('tab', { name: /Route app/ })).toBeNull());
    expect(draft.isConnected).toBe(false);
    expect(control.disposed).toBe(1);
    expect(moduleRuntimeHost.getSnapshot()[0].runtimeId).toBe('rt-save');
    delete hooksControl().seenReact;
  });
  it('keeps a newer confirmation after a cancelled save and releases it when the runtime unloads', async () => {
    const pending = Promise.withResolvers<boolean>();
    const control = { save: pending.promise, attempts: 0, disposed: 0 };
    hooksControl().seenReact = control;
    renderApp();
    await waitFor(() => expect(bridge.channels).toHaveLength(1));
    bridge.channels[0].onmessage({
      kind: 'load', ticket: 'cancelled-save', runtimeId: 'rt-cancelled-save', moduleId: 'dgos.route',
      version: '1.0.0', hookTimeoutMs: 500, views: { layout: 'document', capability: 'multiple' },
      source: `export default function createModule(host) {
        const control = globalThis.__dgosHooks.seenReact;
        return { Component: () => null, openView: () => ({
          Component: () => host.React.createElement('div', null, 'Pending save draft'),
          beforeClose: () => ({ dirty: true, canSave: true }),
          save: () => { control.attempts++; return control.save; },
          dispose: () => { control.disposed++; }
        }) };
      }`,
    });
    fireEvent.click(screen.getByRole('button', { name: '应用', exact: true }));
    fireEvent.click(await within(screen.getByRole('dialog', { name: '应用' })).findByRole('button', { name: 'Route app', exact: true }));
    const draft = await screen.findByText('Pending save draft');
    fireEvent.click(screen.getByRole('button', { name: '关闭 Route app' }));
    let dialog = await screen.findByRole('dialog');
    fireEvent.click(within(dialog).getByRole('button', { name: '保存并关闭' }));
    await waitFor(() => expect(control.attempts).toBe(1));
    fireEvent.click(within(dialog).getByRole('button', { name: '取消', exact: true }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    fireEvent.click(screen.getByRole('button', { name: '关闭 Route app' }));
    dialog = await screen.findByRole('dialog');
    await act(async () => { pending.resolve(true); });
    expect(screen.getByRole('dialog')).toBe(dialog);
    expect(draft.isConnected).toBe(true);
    expect(control.disposed).toBe(0);
    bridge.channels[0].onmessage({ kind: 'unload', ticket: 'unload-during-consent', runtimeId: 'rt-cancelled-save', hookTimeoutMs: 500 });
    await waitFor(() => expect(moduleRuntimeHost.getSnapshot()).toHaveLength(0));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(screen.getByRole('tab', { name: /Route app/ })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '关闭 Route app' }));
    await waitFor(() => expect(screen.queryByRole('tab', { name: /Route app/ })).toBeNull());
    expect(control.disposed).toBe(1);
    delete hooksControl().seenReact;
  });

});

describe('Settings route parity with the Rust gate (D037)', () => {
  it.each(['/settings', '/settings/', '/settings/sub', '/Settings', '/settings2', '/%73ettings', '/settings%2Fx', '/', '/workbench'])(
    'shows Settings for %s only when isHostSettingsPath accepts the raw pathname',
    async (pathname) => {
      vi.spyOn(console, 'warn').mockImplementation(() => undefined);
      window.history.pushState(null, '', pathname);
      const expected = isHostSettingsPath(window.location.pathname);

      renderApp();

      await screen.findByRole('navigation', { name: '工作区导航', hidden: true });
      await waitFor(() => expect(screen.queryByRole('tablist', { name: '系统设置' }) !== null).toBe(expected));
    },
  );

  it('answers the Rust gate table', () => {
    expect(
      ['/settings', '/settings/', '/settings/sub', '/Settings', '/settings2', '/%73ettings', '/settings%2Fx', '/', '/workbench'].map(isHostSettingsPath),
    ).toEqual([true, true, true, false, false, false, false, false, false]);
  });
});
