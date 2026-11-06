import { StrictMode } from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import CliSettings from '../src/pages/CliSettings';
import { resetLocaleForTesting, setLocaleForTesting } from '../src/i18n/runtime';

const { confirmDiscard } = vi.hoisted(() => ({ confirmDiscard: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('../src/workspace/closeCoordinator', () => ({ confirmDiscard }));
const call = vi.mocked(invoke);
const paths = { codex: '/configured/codex', claude: null, gemini: null, node: null };
const ok = (data: unknown) => ({ success: true, data });
function deferred() { let resolve!: (value: unknown) => void; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; }
beforeEach(() => { call.mockReset(); confirmDiscard.mockReset(); setLocaleForTesting('zh-CN'); });
afterEach(() => { cleanup(); resetLocaleForTesting(); });

it('keeps the newer preference read when StrictMode replays hydration', async () => {
  const old = deferred();
  call.mockReturnValueOnce(old.promise).mockResolvedValueOnce(ok({ revision: 4, paths }));
  render(<StrictMode><CliSettings /></StrictMode>);
  await waitFor(() => expect((screen.getByRole('textbox', { name: /^Codex/ }) as HTMLInputElement).value).toBe('/configured/codex'));
  await act(async () => { old.resolve(ok({ revision: 1, paths: { ...paths, codex: '/stale/codex' } })); });
  expect((screen.getByRole('textbox', { name: /^Codex/ }) as HTMLInputElement).value).toBe('/configured/codex');
  expect(screen.getByRole('button', { name: '保存' }).hasAttribute('disabled')).toBe(true);
});

it('retains a conflicted edit and requires readback before another save', async () => {
  call.mockResolvedValueOnce(ok({ revision: 2, paths }));
  render(<CliSettings />);
  await waitFor(() => expect((screen.getByRole('textbox', { name: /^Codex/ }) as HTMLInputElement).disabled).toBe(false));
  fireEvent.change(screen.getByRole('textbox', { name: /^Codex/ }), { target: { value: '/edited/codex' } });
  call.mockResolvedValueOnce(ok({ saved: false, snapshot: { revision: 3, paths: { ...paths, codex: '/concurrent/codex' } } }));
  fireEvent.click(screen.getByRole('button', { name: '保存' }));
  await screen.findByRole('alert');
  expect((screen.getByRole('textbox', { name: /^Codex/ }) as HTMLInputElement).value).toBe('/edited/codex');
  expect(screen.getByRole('button', { name: '保存' }).hasAttribute('disabled')).toBe(true);
  confirmDiscard.mockResolvedValueOnce(true);
  call.mockResolvedValueOnce(ok({ revision: 3, paths: { ...paths, codex: '/concurrent/codex' } }));
  fireEvent.click(screen.getByRole('button', { name: '重新读取配置' }));
  await waitFor(() => expect((screen.getByRole('textbox', { name: /^Codex/ }) as HTMLInputElement).value).toBe('/concurrent/codex'));
  expect(screen.queryByRole('alert')).toBeNull();
});
it('keeps a failed save draft and only shows saved values after confirmed readback', async () => {
  call.mockResolvedValueOnce(ok({ revision: 2, paths }));
  render(<CliSettings />);
  await waitFor(() => expect((screen.getByRole('textbox', { name: /^Codex/ }) as HTMLInputElement).disabled).toBe(false));
  const input = screen.getByRole('textbox', { name: /^Codex/ });
  fireEvent.change(input, { target: { value: '/draft/codex' } });
  call.mockResolvedValueOnce({ success: false, error: { message: 'sensitive host failure' } });
  fireEvent.click(screen.getByRole('button', { name: '保存' }));
  expect(await screen.findByText('保存未确认或发生冲突。草稿已保留；重新读取配置后才能再次保存。')).toBeTruthy();
  expect((input as HTMLInputElement).value).toBe('/draft/codex');
  expect(screen.getByText('/configured/codex')).toBeTruthy();
  expect(screen.queryByText('sensitive host failure')).toBeNull();
  expect(screen.getByRole('button', { name: '保存' }).hasAttribute('disabled')).toBe(true);
});

it('preserves a draft when reload discard is cancelled', async () => {
  call.mockResolvedValueOnce(ok({ revision: 2, paths }));
  render(<CliSettings />);
  await waitFor(() => expect((screen.getByRole('textbox', { name: /^Codex/ }) as HTMLInputElement).disabled).toBe(false));
  const input = screen.getByRole('textbox', { name: /^Codex/ });
  fireEvent.change(input, { target: { value: '/draft/codex' } });
  confirmDiscard.mockResolvedValueOnce(false);
  fireEvent.click(screen.getByRole('button', { name: '重新读取配置' }));
  await waitFor(() => expect(confirmDiscard).toHaveBeenCalledTimes(1));
  expect((input as HTMLInputElement).value).toBe('/draft/codex');
  expect(call).toHaveBeenCalledTimes(1);
});

it('shows probe status for its edited tool without saving the draft', async () => {
  call.mockResolvedValueOnce(ok({ revision: 2, paths }));
  render(<CliSettings />);
  await waitFor(() => expect((screen.getByRole('textbox', { name: /^Codex/ }) as HTMLInputElement).disabled).toBe(false));
  fireEvent.change(screen.getByRole('textbox', { name: /^Node\.js/ }), { target: { value: '/draft/node' } });
  call.mockResolvedValueOnce(ok({ status: 'available', path: '/probe/node', version: 'v22.1.0', executable: true, managed: false }));
  fireEvent.click(screen.getAllByRole('button', { name: '检测版本' })[3]);
  expect(await screen.findByText('可执行，版本查询成功。运行时仍须通过协议握手。')).toBeTruthy();
  expect(screen.getByText('/configured/codex')).toBeTruthy();
  expect((screen.getByRole('textbox', { name: /^Node\.js/ }) as HTMLInputElement).value).toBe('/draft/node');
  expect(call).toHaveBeenCalledTimes(2);
  expect(call.mock.calls.some(([command]) => command === 'set_cli_preferences')).toBe(false);
});
it('attributes a probe failure to its tool without leaking host details', async () => {
  call.mockResolvedValueOnce(ok({ revision: 2, paths }));
  render(<CliSettings />);
  await waitFor(() => expect((screen.getByRole('textbox', { name: /^Codex/ }) as HTMLInputElement).disabled).toBe(false));
  call.mockResolvedValueOnce({ success: false, error: { message: 'sensitive probe detail' } });
  fireEvent.click(screen.getAllByRole('button', { name: '检测版本' })[2]);
  expect(await screen.findByText('版本检测失败。保存路径没有改变。')).toBeTruthy();
  expect(screen.queryByText('sensitive probe detail')).toBeNull();
  const input = screen.getByRole('textbox', { name: /^Gemini/ }) as HTMLInputElement;
  expect(input.getAttribute('aria-invalid')).toBe('true');
  expect(document.getElementById(input.getAttribute('aria-describedby') ?? '')?.textContent).toBe('版本检测失败。保存路径没有改变。');
  expect(screen.getByRole('textbox', { name: /^Codex/ }).getAttribute('aria-invalid')).toBeNull();
  expect(within(screen.getByRole('region', { name: 'Gemini' })).getByRole('button', { name: '检测版本' })).toBeTruthy();
  expect(screen.queryByRole('alert')).toBeNull();
  fireEvent.change(input, { target: { value: '/corrected/gemini' } });
  expect(input.getAttribute('aria-invalid')).toBeNull();
  expect(input.getAttribute('aria-describedby')).toBeNull();
  expect(screen.queryByText('版本检测失败。保存路径没有改变。')).toBeNull();
});

it('does not acknowledge a save until the follow-up read succeeds', async () => {
  call.mockResolvedValueOnce(ok({ revision: 2, paths }));
  render(<CliSettings />);
  await waitFor(() => expect((screen.getByRole('textbox', { name: /^Codex/ }) as HTMLInputElement).disabled).toBe(false));
  fireEvent.change(screen.getByRole('textbox', { name: /^Codex/ }), { target: { value: '/draft/codex' } });
  const readback = Promise.withResolvers<unknown>();
  call.mockResolvedValueOnce(ok({ saved: true, snapshot: { revision: 3, paths: { ...paths, codex: '/draft/codex' } } })).mockReturnValueOnce(readback.promise);
  fireEvent.click(screen.getByRole('button', { name: '保存' }));
  await waitFor(() => expect(call).toHaveBeenCalledTimes(3));
  expect(screen.queryByText('已保存并确认')).toBeNull();
  expect(screen.getByText('/configured/codex')).toBeTruthy();
  await act(async () => { readback.resolve({ success: false }); });
  expect((screen.getByRole('textbox', { name: /^Codex/ }) as HTMLInputElement).value).toBe('/draft/codex');
  expect(screen.getByRole('button', { name: '保存' }).hasAttribute('disabled')).toBe(true);
  expect(screen.queryByText('已保存并确认')).toBeNull();
});

it('announces confirmed save in a dedicated status without enclosing actions or the error alert', async () => {
  call.mockResolvedValueOnce(ok({ revision: 2, paths }));
  render(<CliSettings />);
  await waitFor(() => expect((screen.getByRole('textbox', { name: /^Codex/ }) as HTMLInputElement).disabled).toBe(false));
  const status = screen.getByRole('status');
  expect(status.textContent).toBe('配置已读取');
  expect(status.querySelector('button')).toBeNull();
  expect(status.closest('footer')?.hasAttribute('aria-live')).toBe(false);
  fireEvent.change(screen.getByRole('textbox', { name: /^Codex/ }), { target: { value: '/saved/codex' } });
  expect(status.textContent).toBe('有未保存的草稿');
  const confirmed = { revision: 3, paths: { ...paths, codex: '/saved/codex' } };
  call.mockResolvedValueOnce(ok({ saved: true, snapshot: confirmed })).mockResolvedValueOnce(ok(confirmed));
  fireEvent.click(screen.getByRole('button', { name: '保存' }));
  await waitFor(() => expect(status.textContent).toBe('已保存并确认'));
  expect(screen.queryByRole('alert')).toBeNull();
  expect(screen.getByText('/saved/codex')).toBeTruthy();
  expect(screen.getByRole('button', { name: '保存' }).hasAttribute('disabled')).toBe(true);
});
