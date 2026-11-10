import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useDevelopment } from '../src/stores/developmentStore';
import { useNativeExtensions } from '../src/stores/nativeExtensions';
import type { NativeMcp, NativeSelection } from '../src/types/nativeExtensions';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
const project = { id: 'p', owner: 'u', name: 'Project', path: '/old-root', directory_identity: 'old-inode', archived: false, created_at: 0, last_opened_at: null };
const session = { id: 's', owner: 'u', engine: 'codex' as const, native_id: null, workspace: '/old-worktree', executable: null, model: null, project_id: 'p', task_id: 't' };
beforeEach(() => { useDevelopment.setState({ projects: [project], sessions: [session] }); });
afterEach(() => { useDevelopment.getState().reset(); invoke.mockReset(); });

it.each(['project', 'task'] as const)('does not deduplicate or publish an old %s root read into the replacement root', async kind => {
  const selection: NativeSelection = { target: 'Codex', scope: 'ProjectShared', project_id: 'p', task_id: kind === 'task' ? 't' : null };
  const old = Promise.withResolvers<NativeMcp>();
  const current: NativeMcp = { instances: [], incomplete: false };
  invoke.mockImplementationOnce(() => old.promise).mockResolvedValueOnce(current);
  const first = useNativeExtensions.getState().discoverMcp(selection);
  if (kind === 'project') useDevelopment.setState({ projects: [{ ...project, path: '/replacement-root', directory_identity: 'new-inode' }] });
  else useDevelopment.setState({ sessions: [{ ...session, workspace: '/replacement-worktree' }] });
  const second = await useNativeExtensions.getState().discoverMcp(selection);
  expect(second).toEqual({ ok: true, value: current });
  old.resolve({ instances: [], incomplete: true });
  expect(await first).toEqual({ ok: false, code: 'native_extension_RootChanged' });
  const key = JSON.stringify([useDevelopment.getState().epoch, selection.target, selection.scope, selection.project_id, selection.task_id ?? null]);
  expect(useNativeExtensions.getState().mcp[key]).toEqual({ value: current, loading: false, stale: false, error: null });
});

it('removes an observed root immediately on directory replacement, not only after the next fetch', async () => {
  const selection: NativeSelection = { target: 'Claude', scope: 'ProjectShared', project_id: 'p' };
  invoke.mockResolvedValue({ instances: [], incomplete: true });
  await useNativeExtensions.getState().discoverMcp(selection);
  const key = JSON.stringify([useDevelopment.getState().epoch, 'Claude', 'ProjectShared', 'p', null]);
  expect(useNativeExtensions.getState().mcp[key].value?.incomplete).toBe(true);
  useDevelopment.setState({ projects: [{ ...project, directory_identity: 'replacement-at-same-path' }] });
  expect(useNativeExtensions.getState().mcp[key]).toBeUndefined();
});

it('invalidates task-root observations when task ownership changes without moving the project', async () => {
  const task = { id: 't', owner: 'u', project_id: 'p', goal: 'Goal', requirements: '', status: 'todo' as const, created_at: 0 };
  useDevelopment.setState({ tasks: [task] });
  const selection: NativeSelection = { target: 'Codex', scope: 'ProjectShared', project_id: 'p', task_id: 't' };
  invoke.mockResolvedValue({ instances: [], incomplete: true });
  await useNativeExtensions.getState().discoverMcp(selection);
  const key = JSON.stringify([useDevelopment.getState().epoch, 'Codex', 'ProjectShared', 'p', 't']);
  expect(useNativeExtensions.getState().mcp[key]).toBeDefined();
  useDevelopment.setState({ tasks: [{ ...task, owner: 'different-owner' }] });
  expect(useNativeExtensions.getState().mcp[key]).toBeUndefined();
});
