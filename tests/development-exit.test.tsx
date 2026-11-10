import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { useDevelopmentExit } from '../src/workspace/useDevelopmentExit';
import { initializeDesktopState, resetDesktopState, setDesktopSlot, useDesktopPersistence } from '../src/stores/developmentDesktop';
const { listen, confirm } = vi.hoisted(() => ({ listen: vi.fn(), confirm: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen }));
vi.mock('../src/workspace/closeCoordinator', () => ({ confirmDiscard: confirm }));
vi.mock('../src/i18n', () => ({ t: (key: string) => key }));
let exit: (event: { payload: { request_id: string } }) => Promise<void>;
let saved: { revision: number; value: Record<string, unknown> };
let replies: boolean[];
beforeEach(async () => {
  Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: {} });
  saved = { revision: 0, value: {} }; replies = [];
  confirm.mockResolvedValue(true);
  listen.mockImplementation(async (_name, handler) => { exit = handler; return () => {}; });
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === 'read_development_desktop_state') return saved;
    if (command === 'save_development_desktop_state') {
      // This fixture receives only the hook's typed in-process invocation.
      const request = args as { input: { expected_revision: number; value: Record<string, unknown> } };
      const input = request.input;
      if (input.expected_revision !== saved.revision) throw 'desktop_state_conflict';
      saved = { revision: saved.revision + 1, value: structuredClone(input.value) }; return saved;
    }
    if (command === 'reply_development_exit') {
      // This fixture receives only the hook's typed in-process invocation.
      const request = args as { input: { proceed: boolean } };
      replies.push(request.input.proceed); return;
    }
    throw new Error(`Unexpected command ${command}`);
  });
  await initializeDesktopState('alice');
});
afterEach(() => { cleanup(); resetDesktopState(); vi.clearAllMocks(); Reflect.deleteProperty(window, '__TAURI_INTERNALS__'); });
it('does not authorize exit after a failed save and retains the unsaved draft', async () => {
  renderHook(useDevelopmentExit);
  setDesktopSlot('draft', 'unsaved instruction');
  vi.mocked(invoke).mockRejectedValueOnce('storage_failed');
  await act(() => exit({ payload: { request_id: 'exit-1' } }));
  expect(replies).toEqual([false]);
  expect(saved.value).toEqual({});
  expect(useDesktopPersistence.getState()).toMatchObject({ dirty: true, value: { draft: 'unsaved instruction' } });
});
it('drains edits made during save before authorizing shutdown', async () => {
  renderHook(useDevelopmentExit);
  setDesktopSlot('draft', 'first');
  const pending = Promise.withResolvers<typeof saved>();
  vi.mocked(invoke).mockImplementationOnce(() => pending.promise);
  const closing = exit({ payload: { request_id: 'exit-2' } });
  await act(async () => { await Promise.resolve(); });
  expect(replies).toEqual([]);
  setDesktopSlot('draft', 'second');
  saved = { revision: 1, value: { draft: 'first' } };
  await act(async () => { pending.resolve(saved); await closing; });
  expect(saved.value.draft).toBe('second');
  expect(replies).toEqual([true]);
  expect(useDesktopPersistence.getState().dirty).toBe(false);
});
it('cancel leaves the active workspace intact without granting shutdown', async () => {
  renderHook(useDevelopmentExit); confirm.mockResolvedValue(false);
  setDesktopSlot('draft', 'keep working');
  await act(() => exit({ payload: { request_id: 'exit-3' } }));
  expect(replies).toEqual([false]);
  expect(useDesktopPersistence.getState()).toMatchObject({ dirty: true, value: { draft: 'keep working' } });
  expect(saved.revision).toBe(0);
});
