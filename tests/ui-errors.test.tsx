// AC10, AC18, AC19 (FR-005 Phase 5): error-code mapping, severity routing and ErrorPage of `@dgos/ui`.
//
// What this file can and cannot prove (jsdom, mocked Tauri core, no real host):
//  - Proven here: the mapping table's text comes from the table and never from the host's own message, the generic
//    fallback and its wording, severity routing (minor/moderate -> Toast with 3 s / 5 s, severe -> Modal that waits
//    for acknowledgement), the log line carrying only the code and location, the action button appearing only when
//    the caller can navigate and the route exists, that the table holds no code the registry does not mark in use,
//    ErrorPage's two forms, ErrorBoundary's fallback without leaking the thrown error, and that the real `App` route
//    table shows ErrorPage for an unknown path.
//  - NOT proven here, left to a real WebView (Phase 8): that a real host error reaches handleError from a shipped
//    page in the running app (page wiring is covered with a mocked core in tests/app-shell.test.tsx and the page
//    test files), and the visual layout of ErrorPage.
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

  it('the six Provider codes added by D098 are mapped, not the generic fallback, and are moderate (5 s Toast, no Modal)', () => {
    vi.useFakeTimers();
    render(<Surface />);
    for (const code of [1001014, 1001017, 1001018, 1001021, 1001022, 1001024]) {
      act(() => {
        Modal.closeAll();
        clearToasts();
      });
      expect(lookupError(code)?.severity, String(code)).toBe('moderate');
      act(() => handleError({ code, message: 'HOST: upstream said no' }));
      const alert = screen.getByRole('alert');
      expect(alert.textContent, `${code} fallback`).not.toContain('错误码');
      expect(alert.textContent, `${code} host text`).not.toContain('HOST:');
      expect(screen.queryAllByRole('dialog', { hidden: true }).length, `${code} dialog`).toBe(0);
      act(() => void vi.advanceTimersByTime(4999));
      expect(screen.getByRole('alert').getAttribute('data-state'), `${code} before 5 s`).toBe('open');
      act(() => void vi.advanceTimersByTime(1));
      expect(screen.getByRole('alert').getAttribute('data-state'), `${code} at 5 s`).toBe('closing');
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

  // D110: the 21 registry codes 1004xxx marked 使用中, with their text and severity.
  const MODULE_CODES: ReadonlyArray<readonly [number, string, string, 'minor' | 'moderate' | 'severe']> = [
    [1004001, '模块不存在', '请检查模块 ID 或刷新模块列表', 'moderate'],
    [1004002, '模块已在运行', '无需重复启动', 'minor'],
    [1004003, '模块加载失败', '模块清单或入口无效，可在模块管理查看崩溃记录', 'moderate'],
    [1004004, '没有权限执行此操作', '请在模块管理确认权限后重试', 'moderate'],
    [1004006, '模块版本冲突', '请更新到兼容版本', 'moderate'],
    [1004007, '模块停止失败', '请稍后重试', 'moderate'],
    [1004009, '模块已禁用', '请先在模块管理中启用', 'moderate'],
    [1004013, '模块更新失败', '已保留当前版本，请稍后重试', 'moderate'],
    [1004015, '模块包无效', '模块文件与清单不一致，请重新安装应用', 'moderate'],
    [1004016, '模块未运行', '请先启动模块', 'minor'],
    [1004017, '模块正在执行其他操作', '请等当前操作完成后重试', 'minor'],
    [1004018, '操作记录不存在', '操作已过期或应用已重启，请刷新状态', 'minor'],
    [1004019, '模块服务不可用', '启动恢复未成功，请重启应用或查看恢复日志', 'severe'],
    [1004020, '模块服务内部错误', '请稍后重试，持续出现时查看日志', 'moderate'],
    [1004021, '模块未通过准入', '该模块不在受信任清单中', 'moderate'],
    [1004022, '模块与当前应用不兼容', '请安装兼容版本或升级应用', 'moderate'],
    [1004023, '内置模块不能卸载', '可以改为禁用', 'minor'],
    [1004024, '参数无效', '请检查输入后重试', 'moderate'],
    [1004025, '模块存储空间已满', '存储超过 5 MiB，请删除不需要的数据', 'moderate'],
    [1004026, '拒绝访问非本机地址', '模块只能访问本机服务', 'moderate'],
    [1004027, '响应过大', '本机服务的响应超过 1 MiB', 'moderate'],
  ];
  const RESERVED_MODULE_CODES = [1004005, 1004008, 1004010, 1004011, 1004012, 1004014];

  it('D110: the 21 module codes 1004xxx map to their registered text, advice and severity', () => {
    const mapped = Object.keys(ERROR_MESSAGES)
      .map(Number)
      .filter((code) => code >= 1004000 && code < 1005000);
    expect(mapped).toEqual(MODULE_CODES.map(([code]) => code));
    for (const [code, message, description, severity] of MODULE_CODES) {
      expect(lookupError(code), String(code)).toMatchObject({ message, description, severity });
    }
  });

  it('D110: only 1004001 and 1004009 carry an action, to the module list', () => {
    for (const [code] of MODULE_CODES) {
      const action = lookupError(code)?.action;
      if (code === 1004001 || code === 1004009) expect(action, String(code)).toEqual({ label: '查看模块列表', route: '/settings/modules' });
      else expect(action, String(code)).toBeUndefined();
    }
  });

  it('D110: the six reserved module codes are absent and take the generic fallback', () => {
    render(<Surface />);
    for (const code of RESERVED_MODULE_CODES) {
      expect(lookupError(code), String(code)).toBeUndefined();
      act(() => clearToasts());
      act(() => handleError({ code, message: 'HOST: reserved' }));
      expect(screen.getByRole('alert').textContent, String(code)).toContain(`操作失败（错误码 ${code}）`);
    }
  });

  it('D110: 1004019 opens a Modal that waits for the user and shows no Toast', async () => {
    vi.useFakeTimers();
    render(<Surface />);
    act(() => handleError({ code: 1004019, message: 'HOST: recovery table corrupt' }));
    const dialog = screen.getByRole('dialog', { name: '模块服务不可用' });
    expect(within(dialog).getByText('启动恢复未成功，请重启应用或查看恢复日志')).toBeTruthy();
    expect(screen.queryByRole('alert')).toBeNull();
    expect(document.body.textContent).not.toContain('HOST:');
    act(() => void vi.advanceTimersByTime(60000));
    expect(screen.getByRole('dialog')).toBeTruthy();
    fireEvent.click(within(dialog).getByRole('button', { name: '确定' }));
    await act(async () => void vi.advanceTimersByTime(0));
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('D110: minor module codes are 3 s toasts without the advice, moderate ones are 5 s toasts with it', () => {
    vi.useFakeTimers();
    render(<Surface />);
    for (const [code, message, description, severity] of MODULE_CODES) {
      if (severity === 'severe') continue;
      act(() => {
        Modal.closeAll();
        clearToasts();
      });
      act(() => handleError({ code, message: 'HOST: nope' }));
      const alert = screen.getByRole('alert');
      expect(within(alert).getByText(message), `${code} message`).toBeTruthy();
      if (severity === 'minor') expect(within(alert).queryByText(description), `${code} advice`).toBeNull();
      else expect(within(alert).getByText(description), `${code} advice`).toBeTruthy();
      expect(alert.textContent, `${code} host text`).not.toContain('HOST:');
      expect(alert.textContent, `${code} fallback`).not.toContain('错误码');
      const duration = severity === 'minor' ? 3000 : 5000;
      act(() => void vi.advanceTimersByTime(duration - 1));
      expect(screen.getByRole('alert').getAttribute('data-state'), `${code} before ${duration}`).toBe('open');
      act(() => void vi.advanceTimersByTime(1));
      expect(screen.getByRole('alert').getAttribute('data-state'), `${code} at ${duration}`).toBe('closing');
    }
  });

  it('D110: the 1004001 and 1004009 action navigates to /settings/modules through handleError', () => {
    const navigate = vi.fn();
    render(<Surface />);
    for (const code of [1004001, 1004009]) {
      act(() => clearToasts());
      act(() => handleError({ code }, { navigate, routeExists: (route) => route === '/settings/modules' }));
      fireEvent.click(screen.getByRole('button', { name: '查看模块列表' }));
    }
    expect(navigate).toHaveBeenCalledTimes(2);
    expect(navigate).toHaveBeenNthCalledWith(1, '/settings/modules');
    expect(navigate).toHaveBeenNthCalledWith(2, '/settings/modules');
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
    expect(within(dialog).getByText('请解锁系统凭据库；如果是开发版升级后首次使用，请重新保存 API Key 或令牌后重试')).toBeTruthy();
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
      act(() => handleError({ code: 1001001 }, { navigate, routeExists: (route) => route === '/settings/providers' }));
      fireEvent.click(screen.getByRole('button', { name: '查看 Provider 列表' }));
      expect(navigate).toHaveBeenCalledExactlyOnceWith('/settings/providers');
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

    it('returns from a missing route to the desktop', async () => {
      render(
        <MemoryRouter initialEntries={['/nope']}>
          <App />
        </MemoryRouter>,
      );
      fireEvent.click(await screen.findByRole('button', { name: '返回首页' }));
      await waitFor(() => expect(document.querySelector('a[href="/"][aria-current="page"]')).not.toBeNull());
      expect(screen.queryByRole('button', { name: '返回首页' })).toBeNull();
    });


    it('an unknown path under /settings keeps the existing Settings behaviour (not the 404 page)', async () => {
      render(
        <MemoryRouter initialEntries={['/settings/whatever']}>
          <App />
        </MemoryRouter>,
      );
      expect(await screen.findByRole('heading', { name: '系统设置' })).toBeTruthy();
      expect(screen.queryByText('404 页面不存在')).toBeNull();
    });
  });
});
