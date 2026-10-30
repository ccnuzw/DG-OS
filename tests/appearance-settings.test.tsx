// V1-FR-007 AC12/AC13 (D071): the appearance page.
//
// jsdom with a mocked Tauri core and the real FR-005 appearance controller. Proven here: the controls start at the
// host's values, a choice is applied at once and sent as a typed patch, a failed save puts the controls back on
// the host-confirmed value with exactly one error Toast, quick consecutive choices end on the last one, and the
// page offers no font-size or compact-mode control. NOT proven: layout, focus ring and screen-reader output
// (real WebView, FR-005 Phase 8).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { ToastViewport, clearToasts, disposeAppearance, initAppearance } from '@dgos/ui';
import App from '../src/App';

type Prefs = { theme: string; density: number };

const host = vi.hoisted(() => ({
  stored: { theme: 'light', density: 100 } as { theme: string; density: number },
  sets: [] as unknown[],
  failNext: 0,
  gate: null as null | Promise<void>,
  readGate: null as null | Promise<void>,
}));

vi.mock('@tauri-apps/api/core', () => {
  class Channel {
    onmessage: (message: unknown) => void = () => undefined;
  }
  return {
    Channel,
    invoke: async (command: string, args?: { patch?: Partial<Prefs> }) => {
      if (command === 'list_mcp_servers') return [];
      if (command === 'get_appearance_preferences') {
        if (host.readGate) await host.readGate;
        return { success: true, data: { ...host.stored } };
      }
      if (command === 'set_appearance_preferences') {
        host.sets.push(args?.patch);
        if (host.gate) await host.gate;
        if (host.failNext > 0) {
          host.failNext -= 1;
          return { success: false, error: { code: 1005006, message: 'db path /Users/x' } };
        }
        host.stored = { ...host.stored, ...args?.patch };
        return { success: true, data: { ...host.stored } };
      }
      return null;
    },
  };
});

const root = document.documentElement;

async function openPage() {
  await initAppearance();
  render(
    <MemoryRouter initialEntries={['/settings/appearance']}>
      <App />
      <ToastViewport />
    </MemoryRouter>,
  );
  await screen.findByRole('heading', { name: '外观设置' });
}

const radio = (name: string) => screen.getByRole('radio', { name }) as HTMLButtonElement;
const checked = (name: string) => radio(name).getAttribute('aria-checked') === 'true';

beforeEach(() => {
  host.stored = { theme: 'light', density: 100 };
  host.sets = [];
  host.failNext = 0;
  host.gate = null;
  host.readGate = null;
  vi.spyOn(console, 'error').mockImplementation(() => undefined);
});

afterEach(() => {
  cleanup();
  clearToasts();
  disposeAppearance();
  root.removeAttribute('data-theme');
  root.removeAttribute('data-density');
  vi.restoreAllMocks();
});

describe('route and entry', () => {
  it('is reached from the Settings home link', async () => {
    await initAppearance();
    render(
      <MemoryRouter initialEntries={['/settings']}>
        <App />
      </MemoryRouter>,
    );
    fireEvent.click(screen.getByRole('link', { name: 'Appearance' }));
    expect(await screen.findByRole('heading', { name: '外观设置' })).toBeTruthy();
  });
});

describe('initial state', () => {
  it('shows the host values and offers only theme and interface scale', async () => {
    host.stored = { theme: 'auto', density: 150 };
    await openPage();
    expect(checked('跟随系统')).toBe(true);
    expect(checked('150%')).toBe(true);
    expect(screen.getAllByRole('radiogroup').map((group) => group.getAttribute('aria-labelledby') !== null)).toEqual([true, true]);
    expect(screen.getByText('主题')).toBeTruthy();
    expect(screen.getByText('界面缩放')).toBeTruthy();
    expect(screen.getAllByRole('radio')).toHaveLength(3 + 5);
    expect(screen.queryByText(/字体|紧凑/)).toBeNull();
    expect(root.hasAttribute('data-font-size')).toBe(false);
  });

  it('picks up the host values when the page renders before the first read has finished', async () => {
    host.stored = { theme: 'dark', density: 150 };
    let release: () => void = () => undefined;
    host.readGate = new Promise<void>((resolve) => {
      release = resolve;
    });
    render(
      <MemoryRouter initialEntries={['/settings/appearance']}>
        <App />
      </MemoryRouter>,
    );
    await screen.findByRole('heading', { name: '外观设置' });
    // Defaults until the host answers.
    expect(checked('亮色')).toBe(true);
    expect(checked('100%')).toBe(true);
    await act(async () => {
      release();
    });
    await waitFor(() => expect(checked('暗色')).toBe(true));
    expect(checked('150%')).toBe(true);
    expect(checked('亮色')).toBe(false);
    expect(root.getAttribute('data-theme')).toBe('dark');
  });
});

describe('AC12 theme', () => {
  it('applies the choice at once, sends only the typed theme patch and keeps the control on it', async () => {
    await openPage();
    await act(async () => {
      fireEvent.click(radio('暗色'));
    });
    expect(root.getAttribute('data-theme')).toBe('dark');
    await waitFor(() => expect(host.sets).toEqual([{ theme: 'dark' }]));
    expect(checked('暗色')).toBe(true);

    await act(async () => {
      fireEvent.click(radio('跟随系统'));
    });
    await waitFor(() => expect(host.sets).toEqual([{ theme: 'dark' }, { theme: 'auto' }]));
    expect(checked('跟随系统')).toBe(true);
    expect(host.stored.theme).toBe('auto');
  });

  it('puts the control back on the confirmed theme after a failed save, with one Toast and no leaked detail', async () => {
    await openPage();
    host.failNext = 1;
    await act(async () => {
      fireEvent.click(radio('暗色'));
    });
    await waitFor(() => expect(checked('亮色')).toBe(true));
    expect(checked('暗色')).toBe(false);
    expect(root.getAttribute('data-theme')).toBe('light');
    expect(await screen.findAllByText('操作失败（错误码 1005006）')).toHaveLength(1);
    expect(document.body.textContent).not.toContain('/Users/x');
    expect(host.stored.theme).toBe('light');
  });

  it('keeps the control on the first confirmed value when a later change is the one that fails', async () => {
    await openPage();
    await act(async () => {
      fireEvent.click(radio('暗色'));
    });
    await waitFor(() => expect(checked('暗色')).toBe(true));
    host.failNext = 1;
    await act(async () => {
      fireEvent.click(radio('跟随系统'));
    });
    await waitFor(() => expect(checked('暗色')).toBe(true));
    expect(checked('跟随系统')).toBe(false);
    expect(root.getAttribute('data-theme')).toBe('dark');
  });
});

describe('AC13 interface scale', () => {
  it('goes through the five steps, applies the root attribute and sends only the density patch', async () => {
    await openPage();
    await act(async () => {
      fireEvent.click(radio('125%'));
    });
    expect(root.getAttribute('data-density')).toBe('125');
    await act(async () => {
      fireEvent.click(radio('175%'));
    });
    await waitFor(() => expect(host.sets).toEqual([{ density: 125 }, { density: 175 }]));
    expect(checked('175%')).toBe(true);
    expect(root.getAttribute('data-density')).toBe('175');
    expect(host.stored.density).toBe(175);
  });

  it('restores the confirmed step after a failed save', async () => {
    host.stored = { theme: 'light', density: 125 };
    await openPage();
    host.failNext = 1;
    await act(async () => {
      fireEvent.click(radio('75%'));
    });
    await waitFor(() => expect(checked('125%')).toBe(true));
    expect(root.getAttribute('data-density')).toBe('125');
    expect(await screen.findAllByText('操作失败（错误码 1005006）')).toHaveLength(1);
  });
});

describe('concurrent choices', () => {
  it('ends on the last choice for both controls when changes arrive while a save is in flight', async () => {
    await openPage();
    let release: () => void = () => undefined;
    host.gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    await act(async () => {
      fireEvent.click(radio('暗色'));
      fireEvent.click(radio('150%'));
      fireEvent.click(radio('跟随系统'));
    });
    // Applied at once, while nothing is confirmed yet.
    expect(checked('跟随系统')).toBe(true);
    expect(checked('150%')).toBe(true);
    host.gate = null;
    await act(async () => {
      release();
    });
    await waitFor(() => expect(host.stored).toEqual({ theme: 'auto', density: 150 }));
    expect(checked('跟随系统')).toBe(true);
    expect(checked('150%')).toBe(true);
    // One write in flight at a time: the three choices merged into at most two requests.
    expect(host.sets.length).toBeLessThanOrEqual(2);
  });

  it('does not touch React state or report anything when the page unmounts before the save ends', async () => {
    await openPage();
    let release: () => void = () => undefined;
    host.gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    await act(async () => {
      fireEvent.click(radio('暗色'));
    });
    cleanup();
    host.gate = null;
    await act(async () => {
      release();
      await Promise.resolve();
    });
    expect(console.error).not.toHaveBeenCalled();
    expect(host.stored.theme).toBe('dark');
  });
});

describe('preview', () => {
  it('shows an example Toast from the preview button', async () => {
    await openPage();
    fireEvent.click(screen.getByRole('button', { name: '显示通知' }));
    expect(await screen.findByText('这是一条示例通知')).toBeTruthy();
  });
});
