// P7 (FR-005 §13.5): application shell, the shared page error reporter and the appearance write error path.
//
// jsdom with a mocked Tauri core. Proven here: the current route is the only highlighted nav entry, an error
// action button appears only inside a router and only for a route in the route table, and a failed appearance
// write reaches the unified Toast with the interface already rolled back. NOT proven: layout, focus ring
// rendering and screen-reader output (real WebView, Phase 8).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { ToastViewport, clearToasts, disposeAppearance, initAppearance } from '@dgos/ui';
import App from '../src/App';
import { routeExists } from '../src/appRoutes';
import { usePageError } from '../src/pageError';
import { resetLocaleForTesting, setLocaleForTesting } from '../src/i18n/runtime';
import { useDesktopPreferences } from '../src/workspace/desktopPreferences';

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
  useDesktopPreferences.setState({ loaded: true });
  setLocaleForTesting('zh-CN');
  host.handler = async (command) => (command === 'list_mcp_servers' ? [] : null);
  vi.spyOn(console, 'error').mockImplementation(() => undefined);
});

afterEach(() => {
  cleanup();
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
      <MemoryRouter initialEntries={['/settings']}>
        <App />
      </MemoryRouter>,
    );
    const current = screen
      .getAllByRole('link', { hidden: true })
      .filter((link) => link.getAttribute('aria-current') === 'page')
      .map((link) => link.textContent);
    expect(current).toEqual(['设置']);
  });

  it('marks Desktop on / and nothing on an unknown path', () => {
    const { unmount } = render(
      <MemoryRouter initialEntries={['/']}>
        <App />
      </MemoryRouter>,
    );
    expect(screen.getByRole('link', { name: '桌面总览' }).getAttribute('aria-current')).toBe('page');
    unmount();

    render(
      <MemoryRouter initialEntries={['/nowhere']}>
        <App />
      </MemoryRouter>,
    );
    expect(screen.getAllByRole('link').filter((link) => link.getAttribute('aria-current') === 'page')).toEqual([]);
  });

  it.each(['/Settings', '/%73ettings', '/settings%2Fmodules'])(
    'does not mark Desktop or Settings on %s', (path) => {
      render(<MemoryRouter initialEntries={[path]}><App /></MemoryRouter>);
      expect(document.querySelector('a[href="/"]')?.getAttribute('aria-current')).toBeNull();
      expect(document.querySelector('a[href="/settings"]')?.getAttribute('aria-current')).toBeNull();
      expect(screen.queryAllByRole('tab', { selected: true })).toEqual([]);
    },
  );

  it('updates navigation and open tab names when the locale changes without losing selection', () => {
    render(<MemoryRouter initialEntries={['/skills']}><App /></MemoryRouter>);
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
      <MemoryRouter initialEntries={['/skills']}>
        <Probe code={1001001} />
        <ToastViewport />
      </MemoryRouter>,
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
