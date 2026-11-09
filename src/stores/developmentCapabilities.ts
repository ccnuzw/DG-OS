import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Engine, Json } from './agentRunnerStore';
import { useDevelopment } from './developmentStore';
import { contextGeneration, useContextSelection } from './developmentSelection';

export interface CliProviderRef { provider_id: string; model_id: string; provider_revision: number }
export interface CliProviderConfiguration { engine: Engine; provider: CliProviderRef }
export interface CliProviderRevision { revision: number; permission_epoch: number; configuration: CliProviderConfiguration; created_at: number }
export interface CliProviderSnapshot { binding_id: string; engine: Engine; provider_id: string; model_id: string; provider_revision: number; revision: number; permission_epoch: number; [evidence: string]: Json | undefined }
export type SessionConfigurationView = { format: 'legacy_history'; history: Json | null; read_only: true } | { format: 'provider_bound'; snapshot: CliProviderSnapshot; read_only: boolean };
export interface CliProviderCatalog { engine: Engine; project_id: string; providers: { id: string; name: string; config_revision: number; enabled: boolean; availability: { available: boolean; reason: string | null }; models: { id: string; name: string; model_type: string }[] }[] }
export const readCliProviderRevision = (projectId: string) => invoke<CliProviderRevision | null>('read_development_cli_provider', { projectId });
export const readCliProviderCatalog = (projectId: string, engine: Engine) => invoke<CliProviderCatalog>('read_development_cli_provider_catalog', { projectId, engine });
export const saveCliProvider = (project_id: string, expected_revision: number, configuration: CliProviderConfiguration) => invoke<CliProviderRevision>('save_development_cli_provider', { input: { project_id, expected_revision, configuration } });
export const revokeCliProvider = (projectId: string) => invoke<number>('revoke_development_cli_provider', { projectId });
export const readSessionConfiguration = (sessionId: string) => invoke<SessionConfigurationView>('get_development_session_configuration', { sessionId });
export const recheckCliCleanup = () => invoke<void>('recheck_development_cli_cleanup');

// Refresh saved revisions after local changes without replacing a selected revision.
let changes = 0;
const listeners = new Set<() => void>();
export function capabilitiesChanged() { ++changes; listeners.forEach(listener => listener()); }
export function useProjectCapabilities(projectId: string | null) {
  const epoch = useDevelopment(state => state.epoch);
  const { generation, pending } = useContextSelection();
  const [change, setChange] = useState(changes);
  const [state, setState] = useState<{ projectId: string | null; epoch: number; generation: number; change: number; value: CliProviderRevision | null; error: unknown; loading: boolean }>({ projectId: null, epoch, generation, change, value: null, error: null, loading: false });
  useEffect(() => { const listener = () => setChange(changes); listeners.add(listener); return () => { listeners.delete(listener); }; }, []);
  useEffect(() => {
    let disposed = false;
    const current = () => !disposed && epoch === useDevelopment.getState().epoch && generation === contextGeneration();
    setState({ projectId, epoch, generation, change, value: null, error: null, loading: Boolean(projectId) });
    if (projectId && !pending) void readCliProviderRevision(projectId).then(value => { if (current()) setState({ projectId, epoch, generation, change, value, error: null, loading: false }); }, error => { if (current()) setState({ projectId, epoch, generation, change, value: null, error, loading: false }); });
    return () => { disposed = true; };
  }, [projectId, epoch, generation, pending, change]);
  if (state.projectId !== projectId || state.epoch !== epoch || state.generation !== generation || state.change !== change || pending) return { value: null, error: null, loading: true };
  return state;
}
