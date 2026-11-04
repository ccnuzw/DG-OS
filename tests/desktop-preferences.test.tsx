import { beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { useDesktopPreferences } from '../src/workspace/desktopPreferences';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const call = vi.mocked(invoke);
const values = { pinnedModuleIds: ['first', 'second'], showEntries: false, showRecent: true, sidebarCollapsed: true };
const response = (revision: number) => ({ success: true, data: { revision, values } });
function pending<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }
beforeEach(() => { useDesktopPreferences.getState().reset(); call.mockReset(); });
it('does not save defaults before hydration and retains the saved order', async () => {
  const read = pending<ReturnType<typeof response>>();
  call.mockReturnValueOnce(read.promise);
  const loading = useDesktopPreferences.getState().load();
  useDesktopPreferences.getState().patch({ showEntries: true });
  expect(call).toHaveBeenCalledTimes(1);
  read.resolve(response(7)); await loading;
  expect(useDesktopPreferences.getState().values).toEqual(values);
  expect(useDesktopPreferences.getState().revision).toBe(7);
});
it('serializes changes made during a save against the returned revision', async () => {
  call.mockResolvedValueOnce(response(2)); await useDesktopPreferences.getState().load();
  const write = pending<unknown>();
  call.mockReturnValueOnce(write.promise).mockResolvedValueOnce({ success: true, data: { kind: 'saved', snapshot: { revision: 4, values } } });
  useDesktopPreferences.getState().patch({ showRecent: false });
  useDesktopPreferences.getState().patch({ pinnedModuleIds: ['second', 'first'] });
  write.resolve({ success: true, data: { kind: 'saved', snapshot: { revision: 3, values } } });
  await vi.waitFor(() => expect(useDesktopPreferences.getState().dirty).toBe(false));
  expect(call.mock.calls[2]).toEqual(['set_desktop_preferences', { expectedRevision: 3, values: { ...values, showRecent: false, pinnedModuleIds: ['second', 'first'] } }]);
});
it('requires readback after a conflict or uncertain write instead of replaying changes', async () => {
  call.mockResolvedValueOnce(response(2)); await useDesktopPreferences.getState().load();
  call.mockResolvedValueOnce({ success: true, data: { kind: 'conflict', snapshot: { revision: 3, values } } });
  useDesktopPreferences.getState().patch({ showRecent: false });
  await vi.waitFor(() => expect(useDesktopPreferences.getState().error).toBe('conflict'));
  useDesktopPreferences.getState().patch({ showEntries: true });
  await useDesktopPreferences.getState().save();
  expect(call).toHaveBeenCalledTimes(2);
  call.mockResolvedValueOnce(response(3)); await useDesktopPreferences.getState().load();
  expect(useDesktopPreferences.getState().values).toEqual(values);
  call.mockRejectedValueOnce(new Error('reply lost'));
  useDesktopPreferences.getState().patch({ showRecent: false });
  await vi.waitFor(() => expect(useDesktopPreferences.getState().error).toBe('failed'));
  useDesktopPreferences.getState().patch({ showEntries: true });
  expect(call).toHaveBeenCalledTimes(4);
});
it('discards both reads and writes that arrive after account reset', async () => {
  const read = pending<ReturnType<typeof response>>(); call.mockReturnValueOnce(read.promise);
  const loading = useDesktopPreferences.getState().load(); useDesktopPreferences.getState().reset();
  read.resolve(response(7)); await loading;
  expect(useDesktopPreferences.getState().loaded).toBe(false);
  call.mockResolvedValueOnce(response(2)); await useDesktopPreferences.getState().load();
  const write = pending<unknown>(); call.mockReturnValueOnce(write.promise);
  useDesktopPreferences.getState().patch({ showRecent: false }); useDesktopPreferences.getState().reset();
  write.resolve({ success: true, data: { kind: 'saved', snapshot: { revision: 3, values } } });
  await Promise.resolve(); await Promise.resolve();
  expect(useDesktopPreferences.getState().revision).toBe(0);
  expect(useDesktopPreferences.getState().values.pinnedModuleIds).toEqual([]);
});
