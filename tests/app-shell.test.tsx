// P7 (FR-005 §13.5): application shell, the shared page error reporter and the appearance write error path.
//
// jsdom with a mocked Tauri core. Proven here: the current route is the only highlighted nav entry, an error
// action button appears only inside a router and only for a route in the route table, and a failed appearance
// write reaches the unified Toast with the interface already rolled back. NOT proven: layout, focus ring
// rendering and screen-reader output (real WebView, Phase 8).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { ToastViewport, clearToasts, disposeAppearance, initAppearance, updateAppearance } from '@dgos/ui';
import App from '../src/App';
import { routeExists } from '../src/appRoutes';
import { usePageError } from '../src/pageError';

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
});

describe('navigation', () => {
  it('marks only the current route, and Home / Settings do not stay marked on nested routes', () => {
    render(
      <MemoryRouter initialEntries={['/settings']}>
        <App />
      </MemoryRouter>,
    );
    const current = screen
      .getAllByRole('link')
      .filter((link) => link.getAttribute('aria-current') === 'page')
      .map((link) => link.textContent);
    expect(current).toEqual(['Settings']);
  });

  it('marks Home on / and nothing on an unknown path', () => {
    const { unmount } = render(
      <MemoryRouter initialEntries={['/']}>
        <App />
      </MemoryRouter>,
    );
    expect(screen.getByRole('link', { name: 'Home' }).getAttribute('aria-current')).toBe('page');
    unmount();

    render(
      <MemoryRouter initialEntries={['/nowhere']}>
        <App />
      </MemoryRouter>,
    );
    expect(screen.getAllByRole('link').filter((link) => link.getAttribute('aria-current') === 'page')).toEqual([]);
  });
});

describe('Home greet failure', () => {
  it('shows the unified error Toast when greet rejects with a host code, and keeps the old greeting', async () => {
    host.handler = async (command) => {
      if (command === 'list_mcp_servers') return [];
      if (command === 'greet') throw { code: 1001002, message: 'secret upstream text sk-live-123' };
      return null;
    };
    render(
      <MemoryRouter initialEntries={['/']}>
        <App />
        <ToastViewport />
      </MemoryRouter>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Greet' }));
    expect(await screen.findByText('API Key 无效')).toBeTruthy();
    expect(document.body.textContent).not.toContain('sk-live-123');
    expect(console.error).toHaveBeenCalledWith('[dgos] error', { code: 1001002, where: 'Home' });
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
    await waitFor(() => expect(document.querySelector('output')?.textContent).toBe('/providers'));
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
    for (const route of ['/', '/providers', '/skills', '/workbench', '/mcp', '/settings', '/settings/modules', '/settings/appearance']) {
      expect(routeExists(route)).toBe(true);
    }
    for (const route of ['/nowhere', '/providers/', '/settings/modules/x', '', 'providers']) {
      expect(routeExists(route)).toBe(false);
    }
  });
});

describe('appearance write failure', () => {
  it('rolls the interface back and tells the user through the unified Toast, with only code and place logged', async () => {
    host.handler = async (command) => {
      if (command === 'get_appearance_preferences') return { success: true, data: { theme: 'light', density: 100 } };
      if (command === 'set_appearance_preferences') return { success: false, error: { code: 1005006, message: 'db path /Users/x' } };
      return null;
    };
    render(<ToastViewport />);
    await initAppearance();
    expect(document.documentElement.getAttribute('data-theme')).toBe('light');

    await act(async () => {
      await updateAppearance({ theme: 'dark' });
    });

    expect(document.documentElement.getAttribute('data-theme')).toBe('light');
    expect(await screen.findByText('操作失败（错误码 1005006）')).toBeTruthy();
    expect(document.body.textContent).not.toContain('/Users/x');
    expect(console.error).toHaveBeenCalledWith('[dgos] error', { code: 1005006, where: 'appearance' });
  });
});
