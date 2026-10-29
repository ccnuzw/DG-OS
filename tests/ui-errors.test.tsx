// AC10, AC18, AC19 (FR-005 Phase 5): error-code mapping, severity routing and ErrorPage of `@dgos/ui`.
//
// What this file can and cannot prove (jsdom, mocked Tauri core, no real host):
//  - Proven here: the mapping table's text comes from the table and never from the host's own message, the generic
//    fallback and its wording, severity routing (minor/moderate -> Toast with 3 s / 5 s, severe -> Modal that waits
//    for acknowledgement), the log line carrying only the code and location, the action button appearing only when
//    the caller can navigate and the route exists, that the table holds no code the registry does not mark in use,
//    ErrorPage's two forms, ErrorBoundary's fallback without leaking the thrown error, and that the real `App` route
//    table shows ErrorPage for an unknown path.
//  - NOT proven here, left to a real WebView (Phase 8) or later phases: that a real host error reaches handleError
//    in the shipped pages (no page calls it yet, pages migrate in Phase 7), and the visual layout of ErrorPage.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { MemoryRouter } from 'react-router-dom';
import {
  ERROR_MESSAGES,
  ErrorBoundary,
  ErrorPage,
  Modal,
  ModalHost,
  ToastViewport,
  clearToasts,
  extractErrorCode,
  handleError,
  lookupError,
} from '@dgos/ui';
import { FADE_OUT_MS } from '../src/ui/feedback/toastStore';

vi.mock('@tauri-apps/api/core', () => {
  class Channel {
    onmessage: (message: unknown) => void = () => undefined;
  }
  return {
    Channel,
    invoke: async (command: string) => (command === 'list_mcp_servers' ? [] : null),
  };
});

import App from '../src/App';

beforeEach(() => {
  vi.spyOn(console, 'error').mockImplementation(() => undefined);
});

afterEach(() => {
  act(() => {
    Modal.closeAll();
    clearToasts();
  });
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

function Surface() {
  return (
    <>
      <ToastViewport />
      <ModalHost />
    </>
  );
}

const logged = () => vi.mocked(console.error).mock.calls;

describe('error code mapping (AC10)', () => {
  it('1001001 shows the mapped text and advice, not the host text', () => {
    render(<Surface />);
    act(() => handleError({ code: 1001001, message: 'HOST: provider row 42 missing in table providers' }));
    const alert = screen.getByRole('alert');
    expect(within(alert).getByText('Provider 不存在')).toBeTruthy();
    expect(within(alert).getByText('请检查 Provider ID 是否正确')).toBeTruthy();
    expect(document.body.textContent).not.toContain('HOST:');
    expect(document.body.textContent).not.toContain('providers');
  });

  it('every mapped code resolves to its own entry, with text and a valid severity', () => {
    for (const [key, info] of Object.entries(ERROR_MESSAGES)) {
      expect(lookupError(Number(key)), key).toBe(info);
      expect(info.message.length, key).toBeGreaterThan(0);
      expect(['minor', 'moderate', 'severe']).toContain(info.severity);
    }
    expect(Object.keys(ERROR_MESSAGES).length).toBeGreaterThan(0);
  });

  it('holds only codes the registry marks 使用中 (reserved, voided and string-coded ones are absent)', () => {
    const registry = readFileSync(join(process.cwd(), 'docs/04-技术架构/当前版本/V1-错误码.md'), 'utf8');
    const status = new Map<number, string>();
    for (const line of registry.split('\n')) {
      const cells = line.split('|').map((cell) => cell.trim());
      if (/^\d{7}$/.test(cells[1] ?? '')) status.set(Number(cells[1]), cells[cells.length - 2] ?? '');
    }
    for (const key of Object.keys(ERROR_MESSAGES)) {
      expect(status.get(Number(key)), `${key} must be in the registry`).toMatch(/^使用中/);
    }
  });

  it('the table is frozen and a mapped action only names a route that exists in the app', () => {
    expect(Object.isFrozen(ERROR_MESSAGES)).toBe(true);
    const appRoutes = ['/providers', '/skills', '/workbench', '/mcp', '/settings', '/settings/modules'];
    for (const [key, info] of Object.entries(ERROR_MESSAGES)) {
      if (info.action) expect(appRoutes, key).toContain(info.action.route);
    }
  });

  it('lookup never reaches prototype properties', () => {
    for (const code of [0, 1, -1, 1.5, Number.NaN, null]) expect(lookupError(code), String(code)).toBeUndefined();
  });

  it('extractErrorCode accepts only a non-negative safe integer `code`', () => {
    expect(extractErrorCode({ code: 1001001 })).toBe(1001001);
    for (const bad of [null, undefined, 'x', 7, {}, { code: '1001001' }, { code: -1 }, { code: 1.5 }, { code: Number.NaN }, { code: 2 ** 60 }, new Error('x')]) {
      expect(extractErrorCode(bad), String(bad)).toBeNull();
    }
  });
});

describe('fallback and severity routing (AC18)', () => {
  it('an unregistered code shows 操作失败（错误码 N）, nothing from the host, no stack', () => {
    render(<Surface />);
    const hostError = Object.assign(new Error('secret=abc stack at /Users/x/db.rs:9'), { code: 9999999, stack: 'STACKTRACE' });
    act(() => handleError(hostError));
    const alert = screen.getByRole('alert');
    expect(alert.textContent).toContain('操作失败（错误码 9999999）');
    expect(document.body.textContent).not.toContain('abc');
    expect(document.body.textContent).not.toContain('STACKTRACE');
    expect(document.body.textContent).not.toContain('db.rs');
  });

  it('a value with no numeric code gets the generic text without a code', () => {
    render(<Surface />);
    act(() => handleError('Error: connection refused at 10.0.0.5'));
    expect(screen.getByRole('alert').textContent).toContain('操作失败');
    expect(screen.getByRole('alert').textContent).not.toContain('10.0.0.5');
    expect(screen.getByRole('alert').textContent).not.toContain('错误码');
  });

  it('a minor error is a 3 s toast without the long advice', () => {
    vi.useFakeTimers();
    render(<Surface />);
    act(() => handleError({ code: 1001007 }));
    const alert = screen.getByRole('alert');
    expect(within(alert).getByText('流式输出中断')).toBeTruthy();
    expect(within(alert).queryByText('请重新生成')).toBeNull();
    act(() => void vi.advanceTimersByTime(2999));
    expect(screen.getByRole('alert').getAttribute('data-state')).toBe('open');
    act(() => void vi.advanceTimersByTime(1));
    expect(screen.getByRole('alert').getAttribute('data-state')).toBe('closing');
  });

  it('a moderate error is a 5 s toast with advice; an unregistered one is moderate too', () => {
    vi.useFakeTimers();
    render(<Surface />);
    act(() => handleError({ code: 1001002 }));
    expect(screen.getByRole('alert').textContent).toContain('API Key 无效');
    expect(screen.getByRole('alert').textContent).toContain('Provider 返回认证错误');
    act(() => void vi.advanceTimersByTime(4999));
    expect(screen.getByRole('alert').getAttribute('data-state')).toBe('open');
    act(() => void vi.advanceTimersByTime(1));
    expect(screen.getByRole('alert').getAttribute('data-state')).toBe('closing');
    act(() => void vi.advanceTimersByTime(FADE_OUT_MS));
    act(() => handleError({ code: 9999999 }));
    act(() => void vi.advanceTimersByTime(4999));
    expect(screen.getByRole('alert').getAttribute('data-state')).toBe('open');
  });

  it('a severe error opens a Modal that waits for the user and shows no Toast', async () => {
    vi.useFakeTimers();
    render(<Surface />);
    act(() => handleError({ code: 1001015, message: 'keyring: secret service said no' }));
    const dialog = screen.getByRole('dialog', { name: '系统凭据库不可用' });
    expect(within(dialog).getByText('请解锁系统凭据库后重试')).toBeTruthy();
    expect(screen.queryByRole('alert')).toBeNull();
    expect(document.body.textContent).not.toContain('keyring');
    act(() => void vi.advanceTimersByTime(60000));
    expect(screen.getByRole('dialog')).toBeTruthy();
    fireEvent.click(within(dialog).getByRole('button', { name: '确定' }));
    await act(async () => void vi.advanceTimersByTime(0));
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('only minor and moderate codes produce a Toast, only severe a Modal', () => {
    render(<Surface />);
    for (const [key, info] of Object.entries(ERROR_MESSAGES)) {
      act(() => {
        Modal.closeAll();
        clearToasts();
      });
      act(() => handleError({ code: Number(key) }));
      expect(screen.queryAllByRole('dialog', { hidden: true }).length, `${key} dialog`).toBe(info.severity === 'severe' ? 1 : 0);
      expect(screen.queryAllByRole('alert').length, `${key} toast`).toBe(info.severity === 'severe' ? 0 : 1);
    }
  });

  it('logs the code and where it happened, nothing else', () => {
    render(<Surface />);
    act(() => handleError({ code: 1001001, message: 'HOST text', stack: 'STACK', request: { apiKey: 'sk-secret' } }, { where: 'provider-list' }));
    act(() => handleError({ code: 9999999, message: 'HOST text' }));
    expect(logged()).toEqual([
      ['[dgos] error', { code: 1001001, where: 'provider-list' }],
      ['[dgos] error', { code: 9999999, where: undefined }],
    ]);
    expect(JSON.stringify(logged())).not.toContain('sk-secret');
  });

  describe('action button', () => {
    const navigate = vi.fn();

    it('is shown when the caller can navigate and the route exists, and navigates on click', () => {
      render(<Surface />);
      act(() => handleError({ code: 1001001 }, { navigate, routeExists: (route) => route === '/providers' }));
      fireEvent.click(screen.getByRole('button', { name: '查看 Provider 列表' }));
      expect(navigate).toHaveBeenCalledExactlyOnceWith('/providers');
    });

    it('is not shown when the route does not exist, or when there is no navigate or routeExists', () => {
      render(<Surface />);
      const cases = [
        { navigate, routeExists: () => false },
        { navigate },
        { routeExists: () => true },
        {},
      ];
      for (const options of cases) {
        act(() => clearToasts());
        act(() => handleError({ code: 1001001 }, options));
        expect(screen.queryByRole('button', { name: '查看 Provider 列表' })).toBeNull();
      }
      expect(navigate).not.toHaveBeenCalled();
    });
  });
});

describe('ErrorPage (AC19)', () => {
  it('the not-found form shows 404 text and a 返回首页 button that goes home', () => {
    const onHome = vi.fn();
    render(<ErrorPage kind="not-found" onHome={onHome} />);
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('404 页面不存在');
    expect(screen.getByText('您访问的页面不存在或已被删除。')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '返回首页' }));
    expect(onHome).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('button', { name: '重试' })).toBeNull();
  });

  it('the error form has its own text, optional retry, and an icon that is decorative', () => {
    const onRetry = vi.fn();
    const { container } = render(<ErrorPage kind="error" onRetry={onRetry} onHome={() => undefined} />);
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('出了点问题');
    expect(screen.queryByText(/404/)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: '重试' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
    expect(container.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true');
  });

  it('shows a button only for the handler it was given', () => {
    render(<ErrorPage kind="error" />);
    expect(screen.queryAllByRole('button')).toHaveLength(0);
  });

  describe('ErrorBoundary', () => {
    function Boom(): never {
      throw new Error('token=abc123 at /Users/x/secret.rs:10');
    }

    it('shows the error form instead of a blank page, without the thrown message, and logs only a fixed line', () => {
      render(
        <ErrorBoundary onHome={() => undefined}>
          <Boom />
        </ErrorBoundary>,
      );
      expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('出了点问题');
      expect(document.body.textContent).not.toContain('abc123');
      expect(document.body.textContent).not.toContain('secret.rs');
      expect(logged()).toContainEqual(['[dgos] error', { code: null, where: 'render' }]);
    });

    it('renders its children when nothing throws, and 重试 re-renders them once the cause is gone', () => {
      let broken = true;
      function Flaky() {
        if (broken) throw new Error('x');
        return <p>recovered</p>;
      }
      render(
        <ErrorBoundary onHome={() => undefined}>
          <Flaky />
        </ErrorBoundary>,
      );
      expect(screen.queryByText('recovered')).toBeNull();
      broken = false;
      fireEvent.click(screen.getByRole('button', { name: '重试' }));
      expect(screen.getByText('recovered')).toBeTruthy();
    });

    it('返回首页 calls the supplied handler', () => {
      const onHome = vi.fn();
      render(
        <ErrorBoundary onHome={onHome}>
          <Boom />
        </ErrorBoundary>,
      );
      fireEvent.click(screen.getByRole('button', { name: '返回首页' }));
      expect(onHome).toHaveBeenCalledTimes(1);
    });
  });

  describe('route table', () => {
    it('an unknown path in the real App shows ErrorPage, not a blank content area', async () => {
      render(
        <MemoryRouter initialEntries={['/definitely/not/a/page']}>
          <App />
        </MemoryRouter>,
      );
      expect(await screen.findByRole('heading', { level: 1, name: '404 页面不存在' })).toBeTruthy();
      expect(document.querySelector('.app-content')?.textContent).toContain('返回首页');
    });

    it('返回首页 navigates to the welcome page', async () => {
      render(
        <MemoryRouter initialEntries={['/nope']}>
          <App />
        </MemoryRouter>,
      );
      fireEvent.click(await screen.findByRole('button', { name: '返回首页' }));
      expect(await screen.findByText('Welcome to DG-OS')).toBeTruthy();
    });

    it('known routes are not caught by the fallback', async () => {
      render(
        <MemoryRouter initialEntries={['/']}>
          <App />
        </MemoryRouter>
      );
      expect(await screen.findByText('Welcome to DG-OS')).toBeTruthy();
      expect(screen.queryByText('404 页面不存在')).toBeNull();
    });

    it('an unknown path under /settings keeps the existing Settings behaviour (not the 404 page)', async () => {
      render(
        <MemoryRouter initialEntries={['/settings/whatever']}>
          <App />
        </MemoryRouter>,
      );
      await waitFor(() => expect(document.querySelector('.app-content')?.textContent).toContain('Settings Page'));
      expect(screen.queryByText('404 页面不存在')).toBeNull();
    });
  });
});
