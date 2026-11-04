import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Engine, Json } from './agentRunnerStore';
import { useDevelopment } from './developmentStore';
import { contextGeneration, useContextSelection } from './developmentSelection';

export type CapabilityProvider = { source: 'native' } | { source: 'dgos'; provider_id: string; model_id: string; revision: number };
export interface PinnedSkill { id: string; version: string; hash: string }
export interface SelectedMcpTool { server_id: string; tool_name: string; schema_hash: string }
export interface CapabilityConfiguration { engine: Engine; provider: CapabilityProvider; skills: PinnedSkill[]; mcp: SelectedMcpTool[] }
export interface CapabilityRevision { revision: number; permission_epoch: number; configuration: CapabilityConfiguration; created_at: number }
export interface CapabilitySnapshot { revision: number; permission_epoch: number; engine: Engine; cli_version: string; configuration: CapabilityConfiguration; provider_status: string; skills_status: string; mcp_status: string; native_source: string }
export interface CapabilityCatalog {
  providers: { id: string; name: string; provider_type: string; config_revision: number; enabled: boolean; models: { id: string; name: string; model_type: string }[] }[];
  skills: (PinnedSkill & { name: string; enabled: boolean })[];
  mcp: (SelectedMcpTool & { server_name: string; input_schema: Json })[];
}
export interface CapabilityConfirmation { id: string; project_id: string; task_id: string; session_id: string; run_id: string; server_id: string; tool_name: string; schema_hash: string; arguments: Json }
export const readCapabilities = (projectId: string) => invoke<CapabilityRevision | null>('read_development_capabilities', { projectId });
export const readCapabilityCatalog = () => invoke<CapabilityCatalog>('read_development_capability_catalog');
export const saveCapabilities = (project_id: string, expected_revision: number, configuration: CapabilityConfiguration) => invoke<CapabilityRevision>('save_development_capabilities', { input: { project_id, expected_revision, configuration } });
export const revokeCapabilities = (projectId: string) => invoke<number>('revoke_development_capabilities', { projectId });
export const readSessionCapabilities = (sessionId: string) => invoke<CapabilitySnapshot | null>('get_development_session_capabilities', { sessionId });
export const listCapabilityConfirmations = (sessionId: string) => invoke<CapabilityConfirmation[]>('list_development_capability_confirmations', { sessionId });
export const respondCapabilityConfirmation = (session_id: string, confirmation_id: string, approved: boolean) => invoke<void>('respond_development_capability_confirmation', { input: { session_id, confirmation_id, approved } });

// Invalidate readers after a local save/revoke without replacing an explicitly selected session revision.
let changes = 0;
const listeners = new Set<() => void>();
export function capabilitiesChanged() { ++changes; listeners.forEach(listener => listener()); }
export function useProjectCapabilities(projectId: string | null) {
  const epoch = useDevelopment(state => state.epoch);
  const { generation, pending } = useContextSelection();
  const [change, setChange] = useState(changes);
  const [state, setState] = useState<{ projectId: string | null; epoch: number; generation: number; change: number; value: CapabilityRevision | null; error: unknown; loading: boolean }>({ projectId: null, epoch, generation, change, value: null, error: null, loading: false });
  useEffect(() => { const listener = () => setChange(changes); listeners.add(listener); return () => { listeners.delete(listener); }; }, []);
  useEffect(() => {
    let disposed = false;
    const current = () => !disposed && epoch === useDevelopment.getState().epoch && generation === contextGeneration();
    setState({ projectId, epoch, generation, change, value: null, error: null, loading: Boolean(projectId) });
    if (projectId && !pending) void readCapabilities(projectId).then(value => { if (current()) setState({ projectId, epoch, generation, change, value, error: null, loading: false }); }, error => { if (current()) setState({ projectId, epoch, generation, change, value: null, error, loading: false }); });
    return () => { disposed = true; };
  }, [projectId, epoch, generation, pending, change]);
  if (state.projectId !== projectId || state.epoch !== epoch || state.generation !== generation || state.change !== change || pending) return { value: null, error: null, loading: true };
  return state;
}
