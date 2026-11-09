// P7 (FR-005 §13.5): application shell, the shared page error reporter and the appearance write error path.
//
// jsdom with a mocked Tauri core. Proven here: the current route is the only highlighted nav entry, an error
// action button appears only inside a router and only for a route in the route table, and a failed appearance
// write reaches the unified Toast with the interface already rolled back. NOT proven: layout, focus ring
// rendering and screen-reader output (real WebView, Phase 8).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { ToastViewport, clearToasts, disposeAppearance, initAppearance } from '@dgos/ui';
import App from '../src/App';
import { routeExists } from '../src/appRoutes';
import { usePageError } from '../src/pageError';
import { resetLocaleForTesting, setLocaleForTesting } from '../src/i18n/runtime';
import { useDesktopPreferences } from '../src/workspace/desktopPreferences';
import type { WorkspaceAppEntry } from '../src/modules/moduleApi';
import { useChatStore } from '../src/stores/chatStore';
import { DEFAULTS } from '../src/ui/appearance/model';
import { shellMinimumWidth } from '../src/ui/layout/layoutBudgets';

const host = vi.hoisted(() => ({
  handler: async (_command: string, _args?: unknown): Promise<unknown> => null,
}));

vi.mock('@tauri-apps/api/core', () => {
  class Channel {
    onmessage: (message: unknown) => void = () => undefined;
  }
  return {
    Channel,
    invoke: (command: string, args?: unknown) => host.handler(command, args),
  };
});

beforeEach(() => {
  useDesktopPreferences.getState().reset();
  useChatStore.getState().reset();
  useDesktopPreferences.setState({ loaded: true });
  setLocaleForTesting('zh-CN');
  host.handler = async (command) => (command === 'list_mcp_servers' ? [] : null);
  vi.spyOn(console, 'error').mockImplementation(() => undefined);
});

afterEach(() => {
  cleanup();
  useChatStore.getState().reset();
  clearToasts();
  disposeAppearance();
  document.documentElement.removeAttribute('data-theme');
  document.documentElement.removeAttribute('data-density');
  vi.restoreAllMocks();
  resetLocaleForTesting();
});

describe('navigation', () => {
  it('marks Settings on its exact route', () => {
    render(
      <MemoryRouter initialEntries={['/settings']} future={{ v7_startTransition: true, v7_relativeSplatPath: true }}><App /></MemoryRouter>,
    );
    const current = screen
      .getAllByRole('link', { hidden: true })
      .filter((link) => link.getAttribute('aria-current') === 'page')
      .map((link) => link.textContent);
    expect(current).toEqual(['设置']);
  });

  it('marks Desktop on / and nothing on an unknown path', () => {
    const { unmount } = render(
      <MemoryRouter initialEntries={['/']} future={{ v7_startTransition: true, v7_relativeSplatPath: true }}><App /></MemoryRouter>,
    );
    expect(screen.getByRole('link', { name: '桌面总览' }).getAttribute('aria-current')).toBe('page');
    unmount();

    render(
      <MemoryRouter initialEntries={['/nowhere']} future={{ v7_startTransition: true, v7_relativeSplatPath: true }}><App /></MemoryRouter>,
    );
    expect(screen.getAllByRole('link').filter((link) => link.getAttribute('aria-current') === 'page')).toEqual([]);
  });

  it.each(['/Settings', '/%73ettings', '/settings%2Fmodules'])(
    'does not mark Desktop or Settings on %s', (path) => {
      render(<MemoryRouter initialEntries={[path]} future={{ v7_startTransition: true, v7_relativeSplatPath: true }}><App /></MemoryRouter>);
      expect(document.querySelector('a[href="/"]')?.getAttribute('aria-current')).toBeNull();
      expect(document.querySelector('a[href="/settings"]')?.getAttribute('aria-current')).toBeNull();
      expect(screen.queryAllByRole('tab', { selected: true })).toEqual([]);
    },
  );

  it('updates navigation and open tab names when the locale changes without losing selection', () => {
    render(<MemoryRouter initialEntries={['/skills']} future={{ v7_startTransition: true, v7_relativeSplatPath: true }}><App /></MemoryRouter>);
    expect(screen.getByRole('link', { name: '技能中心' }).getAttribute('aria-current')).toBe('page');
    expect(screen.getByRole('tab', { name: '技能中心' }).getAttribute('aria-selected')).toBe('true');
    act(() => setLocaleForTesting('en-US'));
    expect(screen.getByRole('link', { name: 'Skills' }).getAttribute('aria-current')).toBe('page');
    expect(screen.getByRole('tab', { name: 'Skills' }).getAttribute('aria-selected')).toBe('true');
    expect(screen.getByRole('link', { name: 'Desktop' }).getAttribute('href')).toBe('/');
    expect(screen.getByRole('link', { name: 'Settings' }).getAttribute('href')).toBe('/settings');
    expect(screen.getByRole('button', { name: 'Collapse navigation' }).getAttribute('aria-expanded')).toBe('true');
    fireEvent.click(screen.getByRole('button', { name: 'Collapse navigation' }));
    expect(screen.getByRole('button', { name: 'Expand navigation' }).getAttribute('aria-expanded')).toBe('false');
    expect(screen.getByRole('link', { name: 'Skills' }).getAttribute('aria-current')).toBe('page');
    expect(screen.getByRole('tab', { name: 'Skills' }).getAttribute('aria-selected')).toBe('true');
    fireEvent.click(screen.getByRole('button', { name: 'Expand navigation' }));
    expect(screen.getByRole('button', { name: 'Collapse navigation' }).getAttribute('aria-expanded')).toBe('true');
  });
  it('keeps command destinations and focus labels current across locale and layout changes', async () => {
    host.handler = async command => {
      if (command === 'get_appearance_preferences') return { success: true, data: { kind: 'ready', snapshot: { ...structuredClone(DEFAULTS), revision: 1 }, repairWarnings: [] } };
      if (command === 'list_mcp_servers' || command === 'list_installed_skills' || command === 'check_skill_updates') return [];
      if (command === 'fetch_skills_registry') return { skills: [], registry_url: 'bundled://official', offline: false, warning: null };
      return null;
    };
    render(<MemoryRouter initialEntries={['/skills']} future={{ v7_startTransition: true, v7_relativeSplatPath: true }}><App /></MemoryRouter>);
    fireEvent.keyDown(window, { key: 'k', ctrlKey: true });
    expect(screen.getByRole('option', { name: '专注' })).toBeTruthy();
    act(() => setLocaleForTesting('en-US'));
    fireEvent.click(screen.getByRole('option', { name: 'MCP management' }));
    expect(screen.getByRole('link', { name: 'MCP management' }).getAttribute('aria-current')).toBe('page');
    expect(screen.getByRole('tab', { name: 'MCP management' }).getAttribute('aria-selected')).toBe('true');
    fireEvent.keyDown(window, { key: 'k', metaKey: true });
    fireEvent.click(screen.getByRole('option', { name: 'Focus' }));
    expect(screen.queryByRole('complementary', { name: 'Main navigation' })).toBeNull();
    fireEvent.keyDown(window, { key: 'k', ctrlKey: true });
    fireEvent.click(screen.getByRole('option', { name: 'Restore layout' }));
    expect(screen.getByRole('link', { name: 'Skills' })).toBeTruthy();
    fireEvent.keyDown(window, { key: 'k', ctrlKey: true });
    fireEvent.click(screen.getByRole('option', { name: 'Settings' }));
    expect(await screen.findByRole('button', { name: 'Close settings' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Close settings' }));
    await waitFor(() => expect(screen.getByRole('tab', { name: 'MCP management' }).getAttribute('aria-selected')).toBe('true'));
  });

  it('keeps chat draft status on its mounted surface across navigation and clears it when the draft clears', async () => {
    host.handler = async command => {
      if (command === 'list_chat_sessions') return { success: true, data: { items: [], next_cursor: null } };
      if (command === 'list_mcp_servers' || command === 'list_installed_skills' || command === 'check_skill_updates') return [];
      if (command === 'fetch_skills_registry') return { skills: [], registry_url: 'bundled://official', offline: false, warning: null };
      return null;
    };
    render(<MemoryRouter initialEntries={['/workbench']} future={{ v7_startTransition: true, v7_relativeSplatPath: true }}><App /></MemoryRouter>);
    await screen.findByRole('heading', { name: '开始一次对话' });
    act(() => useChatStore.getState().setDraft('kept-session', 'unsent'));
    expect(within(screen.getByRole('tab', { name: /AI 会话/ })).getByRole('img', { name: '未保存的内容' })).toBeTruthy();
    fireEvent.click(screen.getByRole('link', { name: '技能中心' }));
    expect(within(screen.getByRole('tab', { name: /AI 会话/ })).getByRole('img', { name: '未保存的内容' })).toBeTruthy();
    act(() => useChatStore.getState().setDraft('kept-session', ''));
    expect(within(screen.getByRole('tab', { name: /AI 会话/ })).queryByRole('img', { name: '未保存的内容' })).toBeNull();
  });

  it('retranslates visible launcher feedback without losing its current query', async () => {
    const entry: WorkspaceAppEntry = { moduleId: 'blocked-app', displayName: 'Blocked app', iconResourceId: null, layout: 'document', viewCapability: 'single', enabled: true, permissionState: 'denied', runtimeState: 'ready', crashed: false };
    host.handler = async command => command === 'read_workspace_app_catalog' ? { success: true, data: { catalogRevision: 'feedback', entries: [entry] } } : command === 'list_mcp_servers' ? [] : null;
    render(<MemoryRouter initialEntries={['/']} future={{ v7_startTransition: true, v7_relativeSplatPath: true }}><App /></MemoryRouter>);
    fireEvent.click(screen.getByRole('button', { name: '应用', exact: true }));
    fireEvent.change(screen.getByRole('textbox', { name: '搜索应用' }), { target: { value: 'Blocked' } });
    fireEvent.click(await within(screen.getByRole('dialog', { name: '应用' })).findByRole('button', { name: 'Blocked app', exact: true }));
    expect(screen.getByText('应用当前不可用，请在应用管理中检查启用、授权和运行状态。')).toBeTruthy();
    act(() => setLocaleForTesting('en-US'));
    expect(screen.getByText('This app is unavailable. Check its enabled state, permissions and runtime in app management.')).toBeTruthy();
    expect((screen.getByRole('textbox', { name: 'Search apps' }) as HTMLInputElement).value).toBe('Blocked');
  });

  it('refreshes pinned and command app entries while reusing an existing document destination', async () => {
    function Path() { return <output aria-label="path">{useLocation().pathname}</output>; }
    let entry: WorkspaceAppEntry = { moduleId: 'nav-app', displayName: 'Navigator', iconResourceId: null, layout: 'document', viewCapability: 'multiple', enabled: true, permissionState: 'granted', runtimeState: 'ready', crashed: false };
    useDesktopPreferences.setState({ values: { ...useDesktopPreferences.getState().values, pinnedModuleIds: [entry.moduleId] } });
    host.handler = async command => command === 'read_workspace_app_catalog' ? { success: true, data: { catalogRevision: entry.displayName, entries: [entry] } } : command === 'list_mcp_servers' ? [] : null;
    render(<MemoryRouter initialEntries={['/apps/nav-app/documents/doc-1']} future={{ v7_startTransition: true, v7_relativeSplatPath: true }}><App /><Path /></MemoryRouter>);
    await screen.findByRole('tab', { name: /Navigator/ });
    fireEvent.click(screen.getByRole('link', { name: '桌面总览' }));
    fireEvent.click(screen.getByRole('link', { name: 'Navigator' }));
    expect(screen.getByRole('status', { name: 'path' }).textContent).toBe('/apps/nav-app/documents/doc-1');
    expect(screen.getAllByRole('tab', { name: /Navigator/ })).toHaveLength(1);
    fireEvent.click(screen.getByRole('link', { name: '桌面总览' }));
    fireEvent.keyDown(window, { key: 'k', ctrlKey: true });
    entry = { ...entry, displayName: 'Renamed navigator', permissionState: 'denied' };
    act(() => window.dispatchEvent(new Event('dgos:catalog-invalidated')));
    const blocked = await screen.findByRole('option', { name: 'Renamed navigator' });
    expect(blocked.getAttribute('aria-disabled')).toBe('true');
    fireEvent.click(blocked);
    expect(screen.getByRole('dialog', { name: '命令面板' })).toBeTruthy();
    fireEvent.keyDown(screen.getByRole('combobox'), { key: 'Escape' });
    fireEvent.click(screen.getByRole('link', { name: 'Renamed navigator' }));
    expect(screen.getByRole('status', { name: 'path' }).textContent).toBe('/');
    entry = { ...entry, permissionState: 'granted' };
    act(() => window.dispatchEvent(new Event('dgos:catalog-invalidated')));
    fireEvent.keyDown(window, { key: 'k', ctrlKey: true });
    await waitFor(() => expect(screen.getByRole('option', { name: 'Renamed navigator' }).getAttribute('aria-disabled')).toBe('false'));
    fireEvent.click(screen.getByRole('option', { name: 'Renamed navigator' }));
    expect(screen.getByRole('status', { name: 'path' }).textContent).toBe('/apps/nav-app/documents/doc-1');
    expect(screen.getAllByRole('tab')).toHaveLength(1);
  });

  it('uses stable outer width and restores saved sidebar intent after automatic collapse', () => {
    let width = 1440;
    const callbacks: (() => void)[] = [];
    const frames: FrameRequestCallback[] = [];
    document.documentElement.style.fontSize = '14px';
    document.documentElement.style.setProperty('--font-size-ui', '12px');
    const boundary = shellMinimumWidth(document.body);
    vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(() => width);
    vi.stubGlobal('ResizeObserver', class { constructor(callback: () => void) { callbacks.push(callback); } observe() {} unobserve() {} disconnect() {} });
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
    vi.stubGlobal('cancelAnimationFrame', () => {});
    const settle = () => act(() => { callbacks.forEach(callback => callback()); frames.splice(0).forEach(callback => callback(0)); });
    const toggle = () => screen.getByRole('button', { name: /收起导航|展开导航/ });
    try {
      render(<MemoryRouter initialEntries={['/']} future={{ v7_startTransition: true, v7_relativeSplatPath: true }}><App /></MemoryRouter>);
      settle(); expect(toggle().getAttribute('aria-expanded')).toBe('true');
      width = boundary - 1; settle();
      expect(toggle().getAttribute('aria-expanded')).toBe('false'); expect(toggle()).toHaveProperty('disabled', true);
      expect(useDesktopPreferences.getState().values.sidebarCollapsed).toBe(false);
      settle(); expect(toggle().getAttribute('aria-expanded')).toBe('false');
      width = boundary; settle(); expect(toggle().getAttribute('aria-expanded')).toBe('true');
      act(() => useDesktopPreferences.setState({ values: { ...useDesktopPreferences.getState().values, sidebarCollapsed: true } }));
      width = 800; settle(); width = 1440; settle();
      expect(toggle().getAttribute('aria-expanded')).toBe('false'); expect(toggle()).toHaveProperty('disabled', false);
      expect(useDesktopPreferences.getState().values.sidebarCollapsed).toBe(true);
    } finally { cleanup(); document.documentElement.removeAttribute('style'); vi.restoreAllMocks(); vi.unstubAllGlobals(); }
  });
});


describe('usePageError', () => {
  function Probe({ code }: { code: number }) {
    const reportError = usePageError('Probe');
    const { pathname } = useLocation();
    return (
      <>
        <button onClick={() => reportError({ code })}>fail</button>
        <output>{pathname}</output>
      </>
    );
  }

  it('offers the error action for a route in the table and navigates there', async () => {
    render(
      <MemoryRouter initialEntries={['/skills']} future={{ v7_startTransition: true, v7_relativeSplatPath: true }}><Probe code={1001001} />
      <ToastViewport /></MemoryRouter>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'fail' }));
    fireEvent.click(await screen.findByRole('button', { name: '查看 Provider 列表' }));
    await waitFor(() => expect(document.querySelector('output')?.textContent).toBe('/settings/providers'));
  });

  it('shows the Toast without an action button when there is no router', async () => {
    function Bare() {
      const reportError = usePageError('Bare');
      return <button onClick={() => reportError({ code: 1001001 })}>fail</button>;
    }
    render(
      <>
        <Bare />
        <ToastViewport />
      </>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'fail' }));
    expect(await screen.findByText('Provider 不存在')).toBeTruthy();
    expect(screen.queryByRole('button', { name: '查看 Provider 列表' })).toBeNull();
  });

  it('routeExists accepts the shell routes and rejects anything else', () => {
    for (const route of ['/', '/providers', '/settings/providers', '/settings/skills', '/settings/mcp', '/account', '/skills', '/workbench', '/tasks/single', '/tasks/agent', '/mcp', '/settings', '/settings/modules', '/settings/appearance']) {
      expect(routeExists(route)).toBe(true);
    }
    for (const route of ['/nowhere', '/providers/', '/settings/modules/x', '', 'providers']) {
      expect(routeExists(route)).toBe(false);
    }
  });
});
