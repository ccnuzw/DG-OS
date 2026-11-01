// V1-FR-008 renderer side (D087): the account gate, sign-in page, nav menu and account page.
//
// jsdom with a mocked Tauri core (every invoke is recorded as [command, args]) and a mocked reload wrapper, around
// the real App and pages. Proven here: first launch shows the welcome page and creating the first account reloads;
// the skip button runs the app for this page load; signed_out shows only sign-in (preselected user, error texts,
// no create control); the menu switches user and signs out; the account page changes the password, creates another
// account without reloading and deletes the account behind a password; the pending-cleanup notice survives the
// reload; and no invoke argument ever carries a user id, token or session. NOT proven: that the host enforces any
// of this (Rust tests), real page reload, layout.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { Modal, ToastViewport, clearToasts } from '@dgos/ui';
import App from '../src/App';
import AccountGate from '../src/accounts/AccountGate';
import { takeCleanupNotice, type AccountState } from '../src/accounts/accountApi';
import { reloadApp } from '../src/accounts/reload';

const host = vi.hoisted(() => ({
  calls: [] as Array<[string, unknown]>,
  state: null as unknown,
  replies: {} as Record<string, unknown[]>,
  rejections: {} as Record<string, unknown>,
}));

vi.mock('@tauri-apps/api/core', () => {
  class Channel {
    onmessage: (message: unknown) => void = () => undefined;
  }
  return {
    Channel,
    invoke: async (command: string, args?: unknown) => {
      host.calls.push([command, args]);
      if (Object.hasOwn(host.rejections, command)) throw host.rejections[command];
      const queued = host.replies[command];
      if (queued && queued.length > 0) return queued.shift();
      if (command === 'get_account_state') return { success: true, data: host.state };
      if (command === 'list_modules' || command === 'list_providers') return { success: true, data: [] };
      if (command === 'list_mcp_servers') return [];
      return null;
    },
  };
});

vi.mock('../src/accounts/reload', () => ({ reloadApp: vi.fn() }));

const alice = { username: 'alice', last_sign_in_at: 1 };
const bob = { username: 'bob', last_sign_in_at: 2 };
const noAccounts: AccountState = { mode: 'no_accounts', current: null, accounts: [], preselected: null };
const signedOut: AccountState = { mode: 'signed_out', current: null, accounts: [alice, bob], preselected: 'bob' };
const signedIn: AccountState = { mode: 'signed_in', current: alice, accounts: [alice, bob], preselected: null };

const ok = (data: unknown) => ({ success: true, data, error: null });
const fail = (code: number, detail?: unknown) => ({ success: false, data: null, error: { code, message: 'host text', detail } });
const reply = (command: string, ...responses: unknown[]) => {
  host.replies[command] = responses;
};
const callsOf = (command: string) => host.calls.filter(([name]) => name === command);

function renderApp(path = '/') {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <AccountGate>
        <App />
      </AccountGate>
      <ToastViewport />
    </MemoryRouter>,
  );
}

const typeInto = (label: string | RegExp, value: string, scope: HTMLElement = document.body) =>
  fireEvent.change(within(scope).getByLabelText(label), { target: { value } });
const press = (name: string, scope: HTMLElement = document.body) =>
  fireEvent.click(within(scope).getByRole('button', { name }));
const appVisible = () => screen.findByText('Welcome to DG-OS');

const FORBIDDEN: Record<string, true> = { user_id: true, userid: true, token: true, session: true, account_id: true, accountid: true };
function forbiddenKeys(value: unknown, found: string[] = []): string[] {
  if (Array.isArray(value)) value.forEach(item => forbiddenKeys(item, found));
  else if (typeof value === 'object' && value !== null) {
    for (const [key, inner] of Object.entries(value)) {
      if (Object.hasOwn(FORBIDDEN, key.toLowerCase())) found.push(key);
      forbiddenKeys(inner, found);
    }
  }
  return found;
}

beforeEach(() => {
  host.calls = [];
  host.state = signedIn;
  host.replies = {};
  host.rejections = {};
  window.sessionStorage.clear();
  vi.mocked(reloadApp).mockClear();
  vi.spyOn(console, 'error').mockImplementation(() => undefined);
});

afterEach(() => {
  // The renderer never names a user: no argument of any command may carry such a key, at any depth.
  expect(host.calls.flatMap(([, args]) => forbiddenKeys(args))).toEqual([]);
  act(() => Modal.closeAll());
  act(() => clearToasts());
  cleanup();
  vi.restoreAllMocks();
});

describe('gate', () => {
  it('reads the state first and shows a retryable error panel instead of the app when the read fails', async () => {
    reply('get_account_state', fail(1000000), ok(signedIn));
    renderApp();
    expect(await screen.findByRole('heading', { name: '无法读取账户状态' })).toBeTruthy();
    expect(screen.queryByText('Welcome to DG-OS')).toBeNull();
    press('重试');
    await appVisible();
    expect(callsOf('get_account_state')).toHaveLength(2);
  });

  it('a host without the account commands (Tauri "Command … not found") runs the app ungated with no account menu', async () => {
    host.rejections.get_account_state = 'Command get_account_state not found';
    renderApp();
    await appVisible();
    expect(screen.queryByRole('heading', { name: '无法读取账户状态' })).toBeNull();
    expect(screen.queryByText('创建账户')).toBeNull();
  });

  it('any other IPC rejection still shows the retry panel, not the app', async () => {
    host.rejections.get_account_state = 'Command sign_in not found';
    renderApp();
    expect(await screen.findByRole('heading', { name: '无法读取账户状态' })).toBeTruthy();
    expect(screen.queryByText('Welcome to DG-OS')).toBeNull();
  });

  it('a database that could not be opened shows the reason and the file location, and offers no retry', async () => {
    const path = '/Users/someone/Library/Application Support/com.dgos.app/dgos.db';
    reply('get_account_state', fail(1008012, { stage: 'open', reason: 'Execution Error: file is not a database', path }));
    renderApp();
    expect(await screen.findByRole('heading', { name: '无法打开本机数据库' })).toBeTruthy();
    expect(screen.getByText('打开数据库文件时失败')).toBeTruthy();
    expect(screen.getByText('Execution Error: file is not a database')).toBeTruthy();
    expect(screen.getByText(path)).toBeTruthy();
    expect(screen.getByText(/没有修改这个文件/)).toBeTruthy();
    expect(screen.queryByRole('button', { name: '重试' })).toBeNull();
    expect(screen.queryByText('Welcome to DG-OS')).toBeNull();
    expect(callsOf('get_account_state')).toHaveLength(1);
  });

  it('a failed migration is named as such', async () => {
    reply('get_account_state', fail(1008012, { stage: 'migrate', reason: 'Custom Error: Migration file is missing', path: '/d/dgos.db' }));
    renderApp();
    expect(await screen.findByText('升级数据库结构时失败')).toBeTruthy();
    expect(screen.getByText('Custom Error: Migration file is missing')).toBeTruthy();
    expect(screen.getByText('/d/dgos.db')).toBeTruthy();
  });

  it('1008012 without a usable detail falls back to the retry panel', async () => {
    reply('get_account_state', fail(1008012, { stage: 'open' }), ok(signedIn));
    renderApp();
    expect(await screen.findByRole('heading', { name: '无法读取账户状态' })).toBeTruthy();
    press('重试');
    await appVisible();
  });
});

describe('first launch (no_accounts)', () => {
  it('shows the welcome page, blocks a confirmation mismatch, then creates the account and reloads', async () => {
    host.state = noAccounts;
    renderApp();
    expect(await screen.findByRole('heading', { name: '欢迎使用 DGOS' })).toBeTruthy();
    expect(screen.queryByText('Welcome to DG-OS')).toBeNull();

    typeInto('用户名', 'alice');
    typeInto('密码', 'password1');
    typeInto('确认密码', 'password2');
    press('创建账户');
    expect(await screen.findByText('两次输入的密码不一致')).toBeTruthy();
    expect(callsOf('create_account')).toHaveLength(0);
    expect(reloadApp).not.toHaveBeenCalled();

    reply('create_account', ok(signedIn));
    typeInto('确认密码', 'password1');
    press('创建账户');
    await waitFor(() => expect(reloadApp).toHaveBeenCalledTimes(1));
    expect(callsOf('create_account')).toEqual([['create_account', { input: { username: 'alice', password: 'password1' } }]]);
  });

  it('checks lengths before calling the host', async () => {
    host.state = noAccounts;
    renderApp();
    await screen.findByRole('heading', { name: '欢迎使用 DGOS' });
    typeInto('密码', 'short');
    typeInto('确认密码', 'short');
    press('创建账户');
    expect(await screen.findByText('用户名需为 1～32 个字符')).toBeTruthy();
    expect(screen.getByText('密码需为 8～128 个字符')).toBeTruthy();
    expect(callsOf('create_account')).toHaveLength(0);
  });

  it('"暂不创建，继续使用" renders the app for this page load and offers 创建账户 in the nav', async () => {
    host.state = noAccounts;
    renderApp();
    await screen.findByRole('heading', { name: '欢迎使用 DGOS' });
    press('暂不创建，继续使用');
    await appVisible();
    expect(screen.queryByRole('heading', { name: '欢迎使用 DGOS' })).toBeNull();
    expect(within(screen.getByRole('navigation')).getByRole('link', { name: '创建账户' })).toBeTruthy();
    expect(callsOf('create_account')).toHaveLength(0);
  });

  it('the account page offers the first-account form while skipped; creating reloads', async () => {
    host.state = noAccounts;
    renderApp('/account');
    await screen.findByRole('heading', { name: '欢迎使用 DGOS' });
    press('暂不创建，继续使用');
    expect(await screen.findByRole('heading', { name: '创建第一个账户' })).toBeTruthy();

    reply('create_account', ok(signedIn));
    typeInto('用户名', 'alice');
    typeInto('密码', 'password1');
    typeInto('确认密码', 'password1');
    press('创建账户');
    await waitFor(() => expect(reloadApp).toHaveBeenCalledTimes(1));
  });
});

describe('signed out', () => {
  beforeEach(() => {
    host.state = signedOut;
  });

  it('shows only the sign-in page with the preselected user and no create-account control', async () => {
    renderApp();
    expect(await screen.findByRole('heading', { name: '登录 DGOS' })).toBeTruthy();
    expect(screen.getByRole('radio', { name: 'bob' }).getAttribute('aria-checked')).toBe('true');
    expect(screen.getByRole('radio', { name: 'alice' }).getAttribute('aria-checked')).toBe('false');
    expect(screen.queryByText(/创建|新建|注册/)).toBeNull();
    expect(screen.queryByText('Welcome to DG-OS')).toBeNull();
  });

  it('shows the wrong-password text, the throttle seconds and the deletion notice without reloading', async () => {
    renderApp();
    await screen.findByRole('heading', { name: '登录 DGOS' });

    reply('sign_in', fail(1008005));
    typeInto('密码', 'wrong-password');
    press('登录');
    expect(await screen.findByText('用户名或密码错误')).toBeTruthy();

    reply('sign_in', fail(1008011, { retry_after_seconds: 30 }));
    typeInto('密码', 'wrong-password');
    press('登录');
    expect(await screen.findByText(/30 秒/)).toBeTruthy();

    reply('sign_in', fail(1008009));
    typeInto('密码', 'wrong-password');
    press('登录');
    expect(await screen.findByText(/正在删除.*重新启动/)).toBeTruthy();

    expect(reloadApp).not.toHaveBeenCalled();
    expect(callsOf('sign_in')[0]).toEqual(['sign_in', { input: { username: 'bob', password: 'wrong-password' } }]);
  });

  it('signs in as the chosen user and reloads', async () => {
    renderApp();
    await screen.findByRole('heading', { name: '登录 DGOS' });
    fireEvent.click(screen.getByRole('radio', { name: 'alice' }));
    reply('sign_in', ok(signedIn));
    typeInto('密码', 'password1');
    press('登录');
    await waitFor(() => expect(reloadApp).toHaveBeenCalledTimes(1));
    expect(callsOf('sign_in')).toEqual([['sign_in', { input: { username: 'alice', password: 'password1' } }]]);
  });
});

describe('signed in: nav menu', () => {
  it('shows the current user, switches to another user with that user\'s password and reloads', async () => {
    renderApp();
    await appVisible();
    const nav = screen.getByRole('navigation');
    expect(within(nav).getByText('alice')).toBeTruthy();
    expect(within(nav).getByRole('link', { name: '账户' })).toBeTruthy();

    press('切换用户');
    const dialog = await screen.findByRole('dialog', { name: '切换用户' });
    expect(within(dialog).getByRole('radio', { name: 'bob' }).getAttribute('aria-checked')).toBe('true');
    expect(within(dialog).queryByRole('radio', { name: 'alice' })).toBeNull();

    reply('sign_in', fail(1008005));
    typeInto('密码', 'wrong-password', dialog);
    press('切换', dialog);
    expect(await within(dialog).findByText('用户名或密码错误')).toBeTruthy();
    expect(reloadApp).not.toHaveBeenCalled();
    expect(screen.getByText('Welcome to DG-OS')).toBeTruthy();
    expect(within(nav).getByText('alice')).toBeTruthy();

    reply('sign_in', ok({ ...signedIn, current: bob }));
    typeInto('密码', 'good-password', dialog);
    press('切换', dialog);
    await waitFor(() => expect(reloadApp).toHaveBeenCalledTimes(1));
    expect(callsOf('sign_in')[1]).toEqual(['sign_in', { input: { username: 'bob', password: 'good-password' } }]);
  });

  it('signs out and reloads', async () => {
    renderApp();
    await appVisible();
    reply('sign_out', ok(signedOut));
    press('注销');
    await waitFor(() => expect(reloadApp).toHaveBeenCalledTimes(1));
    expect(callsOf('sign_out')).toHaveLength(1);
  });
});

describe('signed in: account page', () => {
  it('creates another account without reloading and re-reads the state', async () => {
    renderApp('/account');
    expect(await screen.findByRole('heading', { name: '账户' })).toBeTruthy();
    reply('create_account', ok({ ...signedIn, accounts: [alice, bob, { username: 'carol', last_sign_in_at: null }] }));
    typeInto('用户名', 'carol');
    typeInto('密码', 'password1');
    typeInto('确认密码', 'password1');
    press('新建账户');
    expect(await screen.findByText('账户已创建')).toBeTruthy();
    expect(reloadApp).not.toHaveBeenCalled();
    expect(callsOf('create_account')).toEqual([['create_account', { input: { username: 'carol', password: 'password1' } }]]);
    await waitFor(() => expect(callsOf('get_account_state')).toHaveLength(2));
  });

  it('changes the password and reloads; a wrong current password keeps the page', async () => {
    renderApp('/account');
    await screen.findByRole('heading', { name: '账户' });

    reply('change_password', fail(1008005));
    typeInto('当前密码', 'wrong-password');
    typeInto('新密码', 'new-password1');
    typeInto('确认新密码', 'new-password1');
    press('修改密码');
    expect(await screen.findByText('当前密码错误')).toBeTruthy();
    expect(reloadApp).not.toHaveBeenCalled();

    reply('change_password', ok(signedOut));
    typeInto('当前密码', 'old-password');
    press('修改密码');
    await waitFor(() => expect(reloadApp).toHaveBeenCalledTimes(1));
    expect(callsOf('change_password')[1]).toEqual([
      'change_password',
      { input: { current_password: 'old-password', new_password: 'new-password1' } },
    ]);
  });

  it('deletes the account only with a password, then reloads and shows the pending-cleanup notice after remount', async () => {
    renderApp('/account');
    await screen.findByRole('heading', { name: '账户' });
    press('删除账户');
    const dialog = await screen.findByRole('dialog', { name: '删除账户？' });
    expect(within(dialog).getByText(/永久删除/)).toBeTruthy();

    press('确认删除', dialog);
    expect(await within(dialog).findByText('请输入密码以确认删除')).toBeTruthy();
    expect(callsOf('delete_account')).toHaveLength(0);

    reply('delete_account', ok({ state: signedOut, credential_cleanup_pending: true }));
    typeInto('当前密码', 'password1', dialog);
    press('确认删除', dialog);
    await waitFor(() => expect(reloadApp).toHaveBeenCalledTimes(1));
    expect(callsOf('delete_account')).toEqual([['delete_account', { input: { password: 'password1' } }]]);

    cleanup();
    act(() => Modal.closeAll());
    host.state = signedOut;
    renderApp();
    expect(await screen.findByText('账户已删除，但部分系统凭据尚未清理')).toBeTruthy();
    expect(takeCleanupNotice()).toBe(false);
  });

  it('leaves no notice behind when the credential cleanup finished', async () => {
    renderApp('/account');
    await screen.findByRole('heading', { name: '账户' });
    press('删除账户');
    const dialog = await screen.findByRole('dialog', { name: '删除账户？' });
    reply('delete_account', ok({ state: signedOut, credential_cleanup_pending: false }));
    typeInto('当前密码', 'password1', dialog);
    press('确认删除', dialog);
    await waitFor(() => expect(reloadApp).toHaveBeenCalledTimes(1));
    expect(takeCleanupNotice()).toBe(false);
  });
});
