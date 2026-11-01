import { extractErrorCode } from '@dgos/ui';
import { invoke } from '@tauri-apps/api/core';

// V1-FR-008 local accounts: typed wrappers for the six host commands. The host decides who is signed in; no
// command takes a user id, token or account id, and the renderer keeps none (D087). A failed command rejects with
// an `AccountCommandError`, whose numeric `code` is what `handleError` and `accountErrorText` read.

export type AccountMode = 'no_accounts' | 'signed_out' | 'signed_in';

export interface AccountSummary {
  username: string;
  last_sign_in_at: number | null;
}

export interface AccountState {
  mode: AccountMode;
  current: AccountSummary | null;
  accounts: AccountSummary[];
  preselected: string | null;
}

export interface DeleteAccountResult {
  state: AccountState;
  credential_cleanup_pending: boolean;
}

interface CommandResponse<T> {
  success: boolean;
  data?: T | null;
  error?: { code?: number; message?: string; detail?: unknown } | null;
}

export class AccountCommandError extends Error {
  readonly code: number | null;
  readonly detail: unknown;

  constructor(code: number | null, detail?: unknown) {
    super(code === null ? 'account command failed' : `account command failed (${code})`);
    this.name = 'AccountCommandError';
    this.code = code;
    this.detail = detail;
  }
}

/** Tauri's rejection for a command the host did not register (`tauri` webview `Command {name} not found`). */
function isUnregistered(command: string, error: unknown): boolean {
  return error === `Command ${command} not found`;
}

class AccountsUnavailable extends Error {}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  let response: CommandResponse<T> | null | undefined;
  try {
    response = args === undefined ? await invoke<CommandResponse<T>>(command) : await invoke<CommandResponse<T>>(command, args);
  } catch (error) {
    if (isUnregistered(command, error)) throw new AccountsUnavailable();
    throw new AccountCommandError(extractErrorCode(error));
  }
  if (!response || response.success !== true || response.data === undefined || response.data === null) {
    const code = response?.error?.code;
    throw new AccountCommandError(typeof code === 'number' ? code : null, response?.error?.detail);
  }
  return response.data;
}

/**
 * The current account state, or `null` when the host has no account commands at all (a host built without account
 * storage, such as the WebView smoke harnesses). A host that has them always answers; a failed answer rejects.
 */
export async function getAccountState(): Promise<AccountState | null> {
  try {
    return await call<AccountState>('get_account_state');
  } catch (error) {
    if (error instanceof AccountsUnavailable) return null;
    throw error;
  }
}
export const createAccount = (username: string, password: string): Promise<AccountState> =>
  call('create_account', { input: { username, password } });
export const signIn = (username: string, password: string): Promise<AccountState> =>
  call('sign_in', { input: { username, password } });
export const signOut = (): Promise<AccountState> => call('sign_out');
export const changePassword = (currentPassword: string, newPassword: string): Promise<AccountState> =>
  call('change_password', { input: { current_password: currentPassword, new_password: newPassword } });
export const deleteAccount = (password: string): Promise<DeleteAccountResult> =>
  call('delete_account', { input: { password } });

// Client-side pre-checks mirror the host's rules (scalar-value lengths, no trimming); the host stays the authority.
const USERNAME_MAX = 32;
const PASSWORD_MIN = 8;
const PASSWORD_MAX = 128;

export function usernameProblem(username: string): string | undefined {
  const length = [...username].length;
  return length >= 1 && length <= USERNAME_MAX ? undefined : '用户名需为 1～32 个字符';
}

export function passwordProblem(password: string): string | undefined {
  const length = [...password].length;
  return length >= PASSWORD_MIN && length <= PASSWORD_MAX ? undefined : '密码需为 8～128 个字符';
}

export function confirmProblem(password: string, confirm: string): string | undefined {
  return password === confirm ? undefined : '两次输入的密码不一致';
}

/** Inline text for the account error codes a form can show next to its fields; `undefined` for any other error. */
export function accountErrorText(error: unknown): string | undefined {
  if (!(error instanceof AccountCommandError)) return undefined;
  switch (error.code) {
    case 1008001:
      return '用户名已存在';
    case 1008002:
      return '密码需为 8～128 个字符';
    case 1008003:
      return '用户名需为 1～32 个字符，且首尾不能有空白、不能含控制字符';
    case 1008005:
      return '用户名或密码错误';
    case 1008007:
      return '登录状态已失效，请重新登录';
    case 1008009:
      return '该账户正在删除，将在应用重新启动后继续完成';
    case 1008011: {
      const seconds = retryAfterSeconds(error.detail);
      return seconds !== null
        ? `尝试次数过多，请 ${Math.max(1, Math.ceil(seconds))} 秒后再试`
        : '尝试次数过多，请稍后再试';
    }
    default:
      return undefined;
  }
}

// One-shot notice that survives the reload after an account deletion whose credential cleanup is still pending.
// The value is a flag only: nothing about the account is stored.
const CLEANUP_NOTICE_KEY = 'dgos.account.cleanupPending';

export function markCleanupNotice(): void {
  try {
    window.sessionStorage.setItem(CLEANUP_NOTICE_KEY, '1');
  } catch {
    // Storage unavailable: the deletion itself already succeeded, only the reminder is lost.
  }
}

/** True once after `markCleanupNotice`; reading it clears it. */
export function takeCleanupNotice(): boolean {
  try {
    const pending = window.sessionStorage.getItem(CLEANUP_NOTICE_KEY) !== null;
    if (pending) window.sessionStorage.removeItem(CLEANUP_NOTICE_KEY);
    return pending;
  } catch {
    return false;
  }
}

function retryAfterSeconds(detail: unknown): number | null {
  if (typeof detail !== 'object' || detail === null || !('retry_after_seconds' in detail)) return null;
  const seconds = detail.retry_after_seconds;
  return typeof seconds === 'number' && Number.isFinite(seconds) ? seconds : null;
}
