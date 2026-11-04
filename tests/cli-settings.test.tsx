import { StrictMode } from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import CliSettings from '../src/pages/CliSettings';
import { resetLocaleForTesting, setLocaleForTesting } from '../src/i18n/runtime';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const call = vi.mocked(invoke);
const paths = { codex: '/configured/codex', claude: null, gemini: null, node: null };
const ok = (data: unknown) => ({ success: true, data });
function deferred() { let resolve!: (value: unknown) => void; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; }
beforeEach(() => { call.mockReset(); setLocaleForTesting('zh-CN'); });
afterEach(() => { cleanup(); resetLocaleForTesting(); });

it('keeps the newer preference read when StrictMode replays hydration', async () => {
  const old = deferred();
  call.mockReturnValueOnce(old.promise).mockResolvedValueOnce(ok({ revision: 4, paths }));
  render(<StrictMode><CliSettings /></StrictMode>);
  await waitFor(() => expect((screen.getByLabelText('Codex') as HTMLInputElement).value).toBe('/configured/codex'));
  await act(async () => { old.resolve(ok({ revision: 1, paths: { ...paths, codex: '/stale/codex' } })); });
  expect((screen.getByLabelText('Codex') as HTMLInputElement).value).toBe('/configured/codex');
  expect(screen.getByRole('button', { name: '保存' }).hasAttribute('disabled')).toBe(true);
});

it('retains a conflicted edit and requires readback before another save', async () => {
  call.mockResolvedValueOnce(ok({ revision: 2, paths }));
  render(<CliSettings />);
  await waitFor(() => expect((screen.getByLabelText('Codex') as HTMLInputElement).disabled).toBe(false));
  fireEvent.change(screen.getByLabelText('Codex'), { target: { value: '/edited/codex' } });
  call.mockResolvedValueOnce(ok({ saved: false, snapshot: { revision: 3, paths: { ...paths, codex: '/concurrent/codex' } } }));
  fireEvent.click(screen.getByRole('button', { name: '保存' }));
  await screen.findByRole('alert');
  expect((screen.getByLabelText('Codex') as HTMLInputElement).value).toBe('/edited/codex');
  expect(screen.getByRole('button', { name: '保存' }).hasAttribute('disabled')).toBe(true);
  call.mockResolvedValueOnce(ok({ revision: 3, paths: { ...paths, codex: '/concurrent/codex' } }));
  fireEvent.click(screen.getByRole('button', { name: '重新读取配置' }));
  await waitFor(() => expect((screen.getByLabelText('Codex') as HTMLInputElement).value).toBe('/concurrent/codex'));
  expect(screen.queryByRole('alert')).toBeNull();
});
