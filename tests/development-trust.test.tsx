import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { confirmAction } from '../src/components/development/shared';
import { useDevelopment } from '../src/stores/developmentStore';
import { useWorkspaceTrust, type WorkspaceTrust } from '../src/stores/developmentTrust';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('../src/components/development/shared', () => ({ confirmAction: vi.fn() }));
vi.mock('../src/i18n', () => ({ t: (key: string) => key }));

let trust: WorkspaceTrust;
const calls = (command: string) => vi.mocked(invoke).mock.calls.filter(([name]) => name === command);
beforeEach(() => {
  trust = { path: '/workspace', directory_identity: 'original', agent: false, agent_revoked: false, terminal: false, service: false, revision: 0 };
  vi.mocked(confirmAction).mockResolvedValue(true);
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === 'read_development_workspace_trust') return { ...trust };
    const input = args!.input as { scope?: 'agent' | 'terminal' | 'service'; granted?: boolean; expected_revision: number; expected_directory_identity: string; expected_path?: string };
    if (input.expected_directory_identity !== trust.directory_identity || (input.expected_path !== undefined && input.expected_path !== trust.path)) throw 'workspace_changed';
    if (input.expected_revision !== trust.revision) throw 'workspace_trust_conflict';
    if (command === 'activate_development_workspace_agent') {
      if (!trust.agent && !trust.agent_revoked) trust = { ...trust, agent: true, revision: trust.revision + 1 };
      return { ...trust };
    }
    if (command === 'set_development_workspace_trust') {
      trust = { ...trust, [input.scope!]: input.granted, agent_revoked: input.scope === 'agent' ? !input.granted : trust.agent_revoked, revision: trust.revision + 1 };
      return { ...trust };
    }
    throw new Error(`Unexpected command ${command}`);
  });
});
afterEach(() => { cleanup(); useDevelopment.getState().reset(); vi.mocked(invoke).mockReset(); vi.mocked(confirmAction).mockReset(); });

it('keeps ordinary reads pure and activates only the entered active workspace', async () => {
  const { result, rerender } = renderHook(({ active, autoAgent }) => useWorkspaceTrust('project', 'task', active, { autoAgent }), { initialProps: { active: false, autoAgent: true } });
  await act(async () => {});
  expect(invoke).not.toHaveBeenCalled();
  rerender({ active: true, autoAgent: false });
  await waitFor(() => expect(result.current.value?.revision).toBe(0));
  expect(calls('activate_development_workspace_agent')).toHaveLength(0);
  rerender({ active: true, autoAgent: true });
  await waitFor(() => expect(result.current.value?.agent).toBe(true));
  expect(calls('activate_development_workspace_agent')).toEqual([['activate_development_workspace_agent', { input: { project_id: 'project', task_id: 'task', expected_path: '/workspace', expected_directory_identity: 'original', expected_revision: 0 } }]]);
  expect(result.current.value).toMatchObject({ terminal: false, service: false });
  expect(confirmAction).not.toHaveBeenCalled();
});

it('retains explicit revocation across refresh and activation changes, then re-enables directly', async () => {
  trust = { ...trust, agent: true, revision: 1 };
  const { result, rerender } = renderHook(({ active }) => useWorkspaceTrust('project', null, active, { autoAgent: true }), { initialProps: { active: true } });
  await waitFor(() => expect(result.current.value?.agent).toBe(true));
  await act(async () => { expect(await result.current.revoke('agent')).toBe(true); });
  expect(confirmAction).toHaveBeenCalledTimes(1);
  expect(result.current.value).toMatchObject({ agent: false, agent_revoked: true });
  rerender({ active: false }); rerender({ active: true });
  await act(async () => { await result.current.refresh(); });
  expect(calls('activate_development_workspace_agent')).toHaveLength(0);
  await act(async () => { expect(await result.current.grant('agent')).toBe(true); });
  expect(result.current.value).toMatchObject({ agent: true, agent_revoked: false });
  expect(confirmAction).toHaveBeenCalledTimes(1);
});

it('retains confirmation for terminal and service grants and every revocation', async () => {
  const { result } = renderHook(() => useWorkspaceTrust('project', null));
  await waitFor(() => expect(result.current.value).not.toBeNull());
  for (const scope of ['terminal', 'service'] as const) {
    vi.mocked(confirmAction).mockResolvedValueOnce(false);
    await act(async () => { expect(await result.current.grant(scope)).toBe(false); });
    expect(trust[scope]).toBe(false);
    await act(async () => { expect(await result.current.grant(scope)).toBe(true); });
    await act(async () => { expect(await result.current.revoke(scope)).toBe(true); });
  }
  expect(confirmAction).toHaveBeenCalledTimes(6);
  expect(calls('set_development_workspace_trust')).toHaveLength(4);
});

it('records explicit refusal even before the first agent grant', async () => {
  const { result, rerender } = renderHook(({ autoAgent }) => useWorkspaceTrust('project', null, true, { autoAgent }), { initialProps: { autoAgent: false } });
  await waitFor(() => expect(result.current.value?.revision).toBe(0));
  await act(async () => { expect(await result.current.revoke('agent')).toBe(true); });
  expect(trust).toMatchObject({ agent: false, agent_revoked: true, revision: 1 });
  expect(confirmAction).toHaveBeenCalledTimes(1);
  rerender({ autoAgent: true });
  await waitFor(() => expect(result.current.loading).toBe(false));
  expect(calls('activate_development_workspace_agent')).toHaveLength(0);
});

it('discards old project reads before they can authorize a different workspace', async () => {
  const pending = Promise.withResolvers<WorkspaceTrust>();
  vi.mocked(invoke).mockImplementationOnce(() => pending.promise);
  const { result, rerender } = renderHook(({ project }) => useWorkspaceTrust(project, null, true, { autoAgent: true }), { initialProps: { project: 'old' } });
  rerender({ project: 'current' });
  await waitFor(() => expect(result.current.value?.agent).toBe(true));
  await act(async () => { pending.resolve({ ...trust, path: '/old', directory_identity: 'old', agent: false, revision: 0 }); });
  expect(calls('activate_development_workspace_agent').map(([, args]) => {
    // This fixture observes only the hook's typed in-process invocation.
    const input = args!.input as { project_id: string };
    return input.project_id;
  })).toEqual(['current']);
  expect(result.current.value?.path).toBe('/workspace');
});

it('discards a late activation receipt after the account epoch changes', async () => {
  const pending = Promise.withResolvers<WorkspaceTrust>();
  const handler = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args, options) => command === 'activate_development_workspace_agent' ? pending.promise : handler(command, args, options));
  const { result } = renderHook(() => useWorkspaceTrust('project', null, true, { autoAgent: true }));
  await waitFor(() => expect(calls('activate_development_workspace_agent')).toHaveLength(1));
  trust = { ...trust, agent_revoked: true, revision: 1 };
  await act(async () => { useDevelopment.getState().reset(); });
  await waitFor(() => expect(result.current.value?.agent_revoked).toBe(true));
  await act(async () => { pending.resolve({ ...trust, agent: true, agent_revoked: false }); });
  expect(result.current.value).toMatchObject({ agent: false, agent_revoked: true });
});

it('observes a racing revocation after CAS conflict without retrying activation', async () => {
  const handler = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation(async (command, args, options) => {
    if (command === 'activate_development_workspace_agent') {
      trust = { ...trust, agent_revoked: true, revision: 1 };
      throw 'workspace_trust_conflict';
    }
    return handler(command, args, options);
  });
  const { result } = renderHook(() => useWorkspaceTrust('project', null, true, { autoAgent: true }));
  await waitFor(() => expect(result.current.value?.agent_revoked).toBe(true));
  expect(result.current.value?.agent).toBe(false);
  expect(calls('activate_development_workspace_agent')).toHaveLength(1);
});

it('surfaces replaced-directory failures without falling back to explicit agent grants', async () => {
  const handler = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args, options) => command === 'activate_development_workspace_agent' ? Promise.reject('workspace_changed') : handler(command, args, options));
  const { result } = renderHook(() => useWorkspaceTrust('project', null, true, { autoAgent: true }));
  await waitFor(() => expect(result.current.error).toBe('workspace_changed'));
  expect(result.current.value).toBeNull();
  expect(calls('set_development_workspace_trust')).toHaveLength(0);
  expect(confirmAction).not.toHaveBeenCalled();
});
