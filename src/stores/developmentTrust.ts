import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useDevelopment } from './developmentStore';
import { hostCode } from './hostError';
import { confirmAction } from '../components/development/shared';
import { t } from '../i18n';

export type WorkspaceTrustScope = 'agent' | 'terminal' | 'service';
export interface WorkspaceTrust {
  path: string;
  directory_identity: string;
  agent: boolean;
  agent_revoked: boolean;
  terminal: boolean;
  service: boolean;
  revision: number;
}
const listeners = new Set<() => void>();
function changed() { for (const listener of listeners) listener(); }

/** Presentation of a host-owned grant. No renderer state can grant directory authority. */
export function useWorkspaceTrust(projectId: string | null, taskId: string | null, active = true, options: { autoAgent?: boolean } = {}) {
  const epoch = useDevelopment(state => state.epoch);
  const autoAgent = options.autoAgent === true;
  const owner = JSON.stringify([epoch, projectId, taskId]);
  const latest = useRef({ owner, active, autoAgent });
  latest.current = { owner, active, autoAgent };
  const [state, setState] = useState<{ owner: string; value: WorkspaceTrust | null; loading: boolean; error: string | null }>({ owner, value: null, loading: false, error: null });
  const ticket = useRef(0);
  const mounted = useRef(true);
  const flight = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++ticket.current; }; }, []);
  const current = useCallback(() => mounted.current && latest.current.owner === owner && latest.current.active && epoch === useDevelopment.getState().epoch, [owner, epoch]);
  const refresh = useCallback(async () => {
    if (!projectId || !active || !current()) return;
    const version = ++ticket.current;
    setState(previous => ({ owner, value: previous.owner === owner ? previous.value : null, loading: true, error: null }));
    try {
      let value = await invoke<WorkspaceTrust>('read_development_workspace_trust', { input: { project_id: projectId, task_id: taskId } });
      if (!current() || version !== ticket.current) return;
      if (autoAgent && latest.current.autoAgent && !value.agent && !value.agent_revoked) {
        try {
          value = await invoke<WorkspaceTrust>('activate_development_workspace_agent', { input: { project_id: projectId, task_id: taskId, expected_path: value.path, expected_directory_identity: value.directory_identity, expected_revision: value.revision } });
        } catch (error) {
          // Another surface may have granted or revoked the same exact workspace.
          // A stale activation is never replayed over that newer revision.
          if (hostCode(error) !== 'workspace_trust_conflict' || !current() || version !== ticket.current) throw error;
          value = await invoke<WorkspaceTrust>('read_development_workspace_trust', { input: { project_id: projectId, task_id: taskId } });
        }
      }
      if (current() && version === ticket.current) setState({ owner, value, loading: false, error: null });
    } catch (error) {
      if (current() && version === ticket.current) setState({ owner, value: null, loading: false, error: hostCode(error) });
    }
  }, [projectId, taskId, active, autoAgent, owner, current]);
  useEffect(() => {
    void refresh();
    const listener = () => { void refresh(); };
    listeners.add(listener);
    return () => { listeners.delete(listener); ++ticket.current; };
  }, [refresh]);
  async function setGrant(scope: WorkspaceTrustScope, granted: boolean): Promise<boolean> {
    if (!projectId || !current() || flight.current) return false;
    flight.current = true;
    try {
      const before = await invoke<WorkspaceTrust>('read_development_workspace_trust', { input: { project_id: projectId, task_id: taskId } });
      if (!current()) return false;
      if (before[scope] === granted && (scope !== 'agent' || granted || before.agent_revoked)) { setState({ owner, value: before, loading: false, error: null }); return true; }
      if (scope !== 'agent' || !granted) {
        const accepted = await confirmAction(t(granted ? `desktop.trust.${scope}.title` : 'desktop.trust.revokeTitle'), `${before.path}\n\n${t(granted ? `desktop.trust.${scope}.detail` : 'desktop.trust.revokeDetail')}`, t(granted ? 'desktop.trust.allow' : 'desktop.trust.revoke'));
        if (!accepted || !current()) return false;
      }
      const value = await invoke<WorkspaceTrust>('set_development_workspace_trust', { input: { project_id: projectId, task_id: taskId, expected_directory_identity: before.directory_identity, expected_revision: before.revision, scope, granted } });
      if (!current()) return false;
      setState({ owner, value, loading: false, error: null });
      changed();
      return value[scope] === granted;
    } catch (error) {
      if (current()) setState(previous => ({ owner, value: previous.owner === owner ? previous.value : null, loading: false, error: hostCode(error) }));
      return false;
    } finally { flight.current = false; }
  }
  return {
    value: state.owner === owner ? state.value : null,
    loading: state.owner !== owner || state.loading,
    error: state.owner === owner ? state.error : null,
    refresh,
    grant: (scope: WorkspaceTrustScope) => setGrant(scope, true),
    revoke: (scope: WorkspaceTrustScope) => setGrant(scope, false),
  };
}
