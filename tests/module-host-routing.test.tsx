import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { BrowserRouter } from 'react-router-dom';
import App from '../src/App';
import { isHostSettingsPath } from '../src/modules/hostRoutes';
import { moduleRuntimeHost } from '../src/modules/moduleRuntimeHost';
import type * as ModuleRuntimeHostModule from '../src/modules/moduleRuntimeHost';
import { readFixture } from './helpers/moduleFixtures';

const bridge = vi.hoisted(() => ({
  attachCalls: 0,
  channels: [] as Array<{ onmessage: (message: unknown) => void }>,
  reports: [] as Array<{ attachmentId: unknown; ticket: unknown; receipt: unknown }>,
}));

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
vi.mock('../src/pages/Workbench', async () => {
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

function navLink(name: string) {
  return screen.getByRole('link', { name });
}

beforeAll(() => {
  Object.assign(window, { __TAURI_INTERNALS__: {} });
});
afterAll(() => {
  Reflect.deleteProperty(window, '__TAURI_INTERNALS__');
});
beforeEach(() => {
  window.history.replaceState(null, '', '/');
  bridge.reports.length = 0;
});
afterEach(async () => {
  cleanup();
  vi.restoreAllMocks();
  // The host singleton outlives each test: unload whatever a test left loaded.
  const channel = bridge.channels[0];
  for (const view of moduleRuntimeHost.getSnapshot()) {
    channel.onmessage({ kind: 'unload', ticket: `cleanup-${view.runtimeId}`, runtimeId: view.runtimeId, hookTimeoutMs: 100 });
  }
  await waitFor(() => expect(moduleRuntimeHost.getSnapshot()).toHaveLength(0));
});

describe('SPA navigation', () => {
  it('opens Settings through pushState without a page load', async () => {
    Object.assign(window, { __spaMarker: 'alive' });
    renderApp();
    const pushState = vi.spyOn(window.history, 'pushState');

    fireEvent.click(navLink('Settings'));

    await screen.findByText('Settings Page');
    expect(pushState).toHaveBeenCalledWith(expect.anything(), '', '/settings');
    expect(window.location.pathname).toBe('/settings');
    expect(screen.getByRole('link', { name: 'Module management' })).toBeTruthy();
    expect(Reflect.get(window, '__spaMarker')).toBe('alive');
    Reflect.deleteProperty(window, '__spaMarker');
  });

  it('attaches the module host exactly once across SPA navigation', async () => {
    renderApp();
    fireEvent.click(navLink('Providers'));
    await screen.findByText('Providers Page');
    fireEvent.click(navLink('Settings'));
    await screen.findByText('Settings Page');
    fireEvent.click(navLink('Workbench'));
    await screen.findByText('Workbench Page');
    fireEvent.click(navLink('Settings'));
    await screen.findByText('Settings Page');

    expect(bridge.attachCalls).toBe(1);
    expect(bridge.channels).toHaveLength(1);
  });

  it('keeps a loaded module alive across route changes and re-attaches the same container', async () => {
    renderApp();
    fireEvent.click(navLink('Settings'));
    await screen.findByText('Settings Page');
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

    // The module DOM only lives on its own panel; the sidebar entry leads there.
    expect(screen.queryByTestId('module-slot')).toBeNull();
    fireEvent.click(await screen.findByRole('link', { name: 'dgos.route' }));
    const button = await screen.findByTestId('fixture-counter');
    expect(window.location.pathname).toBe('/settings/modules/dgos.route');
    expect(screen.getByTestId('module-slot').getAttribute('data-runtime-id')).toBe('rt-route-1');
    await waitFor(() =>
      expect(bridge.reports).toContainEqual({ attachmentId: 'a1', ticket: 'route-load', receipt: { kind: 'loaded' } }),
    );

    fireEvent.click(button);
    await waitFor(() => expect(button.textContent).toBe('count:1'));

    fireEvent.click(navLink('Workbench'));
    await screen.findByText('Workbench Page');
    expect(screen.queryByTestId('module-slot')).toBeNull();
    const [view] = moduleRuntimeHost.getSnapshot();
    expect(moduleRuntimeHost.getSnapshot()).toHaveLength(1);
    expect(view.runtimeId).toBe('rt-route-1');
    expect(view.container.parentNode).toBeNull();

    fireEvent.click(screen.getByRole('link', { name: 'dgos.route' }));
    const returned = await screen.findByTestId('fixture-counter');
    expect(returned).toBe(button);
    expect(returned.textContent).toBe('count:1');
    expect(screen.getByTestId('module-slot').contains(view.container)).toBe(true);

    channel.onmessage({ kind: 'unload', ticket: 'route-unload', runtimeId: 'rt-route-1', hookTimeoutMs: 500 });
    await waitFor(() => expect(moduleRuntimeHost.getSnapshot()).toHaveLength(0));
    expect(screen.queryByTestId('fixture-counter')).toBeNull();
    expect(screen.queryByTestId('module-slot')).toBeNull();
    expect(screen.queryByRole('link', { name: 'dgos.route' })).toBeNull();
    expect(screen.getByText('This module is not running')).toBeTruthy();
  });
});

describe('Settings route parity with the Rust gate (D037)', () => {
  it.each(['/settings', '/settings/', '/settings/sub', '/Settings', '/settings2', '/%73ettings', '/settings%2Fx', '/', '/workbench'])(
    'shows Settings for %s only when isHostSettingsPath accepts the raw pathname',
    (pathname) => {
      vi.spyOn(console, 'warn').mockImplementation(() => undefined);
      window.history.pushState(null, '', pathname);
      const expected = isHostSettingsPath(window.location.pathname);

      renderApp();

      expect(screen.queryByText('Settings Page') !== null).toBe(expected);
      expect(screen.queryByRole('link', { name: 'Module management' }) !== null).toBe(expected);
    },
  );

  it('answers the Rust gate table', () => {
    expect(
      ['/settings', '/settings/', '/settings/sub', '/Settings', '/settings2', '/%73ettings', '/settings%2Fx', '/', '/workbench'].map(isHostSettingsPath),
    ).toEqual([true, true, true, false, false, false, false, false, false]);
  });
});
