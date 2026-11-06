import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { useDevelopment } from './developmentStore';
import { beginResourceRead, failResourceRead, type ResourceReadState } from '../resourceState';

export type McpTransport = 'stdio' | 'http';
export type McpStatus = 'configured' | 'connecting' | 'ready' | 'degraded' | 'error' | 'disconnecting';
export interface McpServer {
  id: string; user_id: string; name: string; transport: McpTransport; enabled: boolean;
  auto_connect: boolean; status: McpStatus; last_error_class?: string; credential_ref?: string;
  reconnect_attempts: number; reconnect_exhausted: boolean;
  created_at: number; updated_at: number;
}
export interface McpTool {
  server_id: string; tool_name: string; title?: string; description?: string;
  input_schema: unknown; output_schema?: unknown; schema_hash: string; discovered_at: number;
}
export interface McpUsageLog {
  id: number; server_id: string; tool_name: string; task_ref?: string; caller?: string; outcome: string;
  error_class?: string; duration_ms?: number; created_at: number;
}
export interface McpConnectionLog {
  id: number; server_id: string; event: string; status: McpStatus; error_class?: string;
  duration_ms?: number; reconnect_attempt?: number; created_at: number;
}
export interface StdioConfig { executable: string; args: string[]; cwd?: string }
export interface HttpConfig { url: string; credential_ref?: string; allow_insecure_http?: boolean }
export interface CreateMcpServerRequest {
  name: string; transport: McpTransport; stdio?: StdioConfig; http?: HttpConfig;
  enabled?: boolean; auto_connect?: boolean;
}
export interface CallMcpToolRequest {
  server_id: string; tool_name: string; arguments: unknown; task_ref?: string;
  timeout_ms?: number; confirmation_token?: string; call_id?: string;
}
export interface CallMcpToolResponse {
  server_id: string; tool_name: string; is_error: boolean; content: unknown;
  structured_content?: unknown;
  duration_ms: number; usage_log_id: number; audit_warning: boolean;
  call_id: string; outcome: 'success' | 'error' | 'timeout' | 'cancelled';
}
export type McpDiscovery = {
  operationId: number; accountEpoch: number; serverId: string;
} & ({ status: 'pending' | 'succeeded' } | { status: 'failed' | 'unconfirmed'; error: string });
interface McpState {
  servers: McpServer[]; tools: McpTool[]; logsByServer: Record<string, McpUsageLog[]>;
  connectionsByServer: Record<string, McpConnectionLog[]>;
  reads: Record<string, ResourceReadState>;
  discoveryByServer: Record<string, McpDiscovery>;
  pending: string[]; error: string | null;
  markReadsStale: () => void;
  fetch: (refreshIf?: () => boolean) => Promise<void>;
  create: (request: CreateMcpServerRequest) => Promise<McpServer>;
  update: (id: string, request: Partial<CreateMcpServerRequest>) => Promise<McpServer>;
  getConfig: (id: string) => Promise<CreateMcpServerRequest>;
  toggle: (id: string, enabled: boolean) => Promise<void>;
  remove: (id: string) => Promise<void>;
  connect: (id: string) => Promise<void>;
  disconnect: (id: string) => Promise<void>;
  loadTools: (id: string, refreshIf?: () => boolean) => Promise<void>;
  discoverTools: (id: string) => Promise<void>;
  setToken: (id: string, token: string) => Promise<void>;
  clearToken: (id: string) => Promise<void>;
  confirmTool: (request: CallMcpToolRequest) => Promise<string>;
  call: (request: CallMcpToolRequest) => Promise<CallMcpToolResponse>;
  cancelCall: (serverId: string, callId: string) => Promise<boolean>;
  fetchLogs: (id: string, refreshIf?: () => boolean) => Promise<void>;
  fetchConnections: (id: string, refreshIf?: () => boolean) => Promise<void>;
}
export const mcpErrorMessage = (error: unknown) => {
  const message = typeof error === 'string' ? error : error instanceof Error ? error.message : 'MCP operation failed. Please retry.';
  return message.includes("reading 'invoke'") ? 'MCP management requires the DGOS desktop app. This browser preview has no desktop connection.' : message;
};

// refresh_tools publishes its snapshot before its final list_tools read. Only
// errors known to precede publication prove failure; database/IPC errors do not.
const discoveryDefinitelyFailed = (error: unknown): boolean => typeof error === 'string' && (
  ['MCP server is not connected', 'MCP server is disabled', 'MCP permission denied',
    'MCP call timed out', 'MCP call was cancelled'].includes(error)
  || error.startsWith('MCP protocol error: ')
  || error.startsWith('MCP server returned an error: ')
  || error.startsWith('transport error: ')
);
export const useMcpStore = create<McpState>((set, get) => {
  // Account changes already advance this epoch in reloadApp; activity never changes ownership.
  const revisions = new Map<string, number>();
  const reads = new Map<string, { revision: number; promise: Promise<void>; refreshIf?: () => boolean; refreshRequired?: boolean }>();
  let discoverySequence = 0;
  const readState = (key: string): ResourceReadState => get().reads[key] ?? { status: 'idle' };
  const setRead = (key: string, state: ResourceReadState) => set({ reads: { ...get().reads, [key]: state } });
  const invalidate = (id: string) => {
    for (const key of ['servers', `tools:${id}`, `usage:${id}`, `connections:${id}`]) {
      revisions.set(key, (revisions.get(key) ?? 0) + 1);
    }
  };
  const ownedInvoke = async <T,>(command: string, args?: Record<string, unknown>): Promise<T> => {
    const epoch = useDevelopment.getState().epoch;
    const value = await invoke<T>(command, args);
    if (epoch !== useDevelopment.getState().epoch) throw new Error('The account changed during the MCP operation.');
    return value;
  };
  // One flight per account/resource. A read requested after a mutation waits for the old
  // flight, then takes a fresh snapshot; joining an obsolete flight must not lose that intent.
  const snapshot = <T,>(key: string, command: string, apply: (value: T) => void, args?: Record<string, unknown>, refreshIf?: () => boolean): Promise<void> => {
    const epoch = useDevelopment.getState().epoch;
    const revision = revisions.get(key) ?? 0;
    const existing = reads.get(key);
    if (existing) {
      if (existing.revision !== revision) existing.refreshRequired = true;
      if (refreshIf) existing.refreshIf = refreshIf;
      return existing.promise;
    }
    setRead(key, beginResourceRead(readState(key)));
    const current = () => epoch === useDevelopment.getState().epoch && revision === (revisions.get(key) ?? 0);
    const promise: Promise<void> = invoke<T>(command, args).then(value => {
      if (current()) { apply(value); setRead(key, { status: 'ready', refresh: { status: 'current' } }); }
    }).catch(error => {
      if (current()) setRead(key, failResourceRead(readState(key), mcpErrorMessage(error)));
    }).finally(() => {
      if (reads.get(key)?.promise !== promise) return;
      reads.delete(key);
      if (epoch === useDevelopment.getState().epoch && !current()) {
        const state = readState(key);
        if (state.status === 'loading') setRead(key, { status: 'idle' });
        else if (state.status === 'ready' && state.refresh.status === 'refreshing') {
          setRead(key, { status: 'ready', refresh: state.refresh.error ? { status: 'error', error: state.refresh.error } : { status: 'stale' } });
        }
      }
      if (epoch === useDevelopment.getState().epoch && (flight.refreshRequired || flight.refreshIf?.())) {
        return snapshot(key, command, apply, args);
      }
    });
    const flight: { revision: number; promise: Promise<void>; refreshIf?: () => boolean; refreshRequired?: boolean } = { revision, promise };
    reads.set(key, flight);
    return promise;
  };
  useDevelopment.subscribe((state, previous) => {
    if (state.epoch === previous.epoch) return;
    reads.clear(); revisions.clear();
    set({ servers: [], tools: [], logsByServer: {}, connectionsByServer: {}, reads: {}, discoveryByServer: {}, pending: [], error: null });
  });
  const remember = (server: McpServer, epoch: number) => {
    if (epoch === useDevelopment.getState().epoch) set({ servers: get().servers.map(item => item.id === server.id ? server : item) });
  };
  const rememberTools = (id: string, tools: McpTool[], epoch: number) => {
    if (epoch === useDevelopment.getState().epoch) {
      set({ tools: [...get().tools.filter(tool => tool.server_id !== id), ...tools] });
      setRead(`tools:${id}`, { status: 'ready', refresh: { status: 'current' } });
    }
  };
  // A per-server action lock prevents duplicate side effects even before React re-renders.
  const action = async <T,>(key: string, operation: (epoch: number) => Promise<T>, reportFailure = true): Promise<T> => {
    if (get().pending.includes(key)) throw new Error('An operation is already running for this server.');
    const epoch = useDevelopment.getState().epoch;
    invalidate(key);
    set({ pending: [...get().pending, key], error: null });
    try { return await operation(epoch); }
    catch (error) { if (reportFailure && epoch === useDevelopment.getState().epoch) set({ error: mcpErrorMessage(error) }); throw error; }
    finally {
      if (epoch === useDevelopment.getState().epoch) {
        invalidate(key);
        set({ pending: get().pending.filter(id => id !== key) });
      }
    }
  };
  return {
    servers: [], tools: [], logsByServer: {}, connectionsByServer: {}, reads: {}, discoveryByServer: {}, pending: [], error: null,
    markReadsStale: () => {
      const next = { ...get().reads };
      for (const [key, state] of Object.entries(next)) {
        if (state.status === 'ready' && state.refresh.status === 'current') next[key] = { status: 'ready', refresh: { status: 'stale' } };
      }
      set({ reads: next });
    },
    fetch: refreshIf => snapshot<McpServer[]>('servers', 'list_mcp_servers', value => set({ servers: value }), undefined, refreshIf),
    create: request => action('new', async epoch => {
      const server = await ownedInvoke<McpServer>('create_mcp_server', { request });
      if (epoch !== useDevelopment.getState().epoch) throw new Error('The account changed during the MCP operation.');
      set({ servers: [...get().servers, server] });
      return server;
    }),
    update: (serverId, request) => action(serverId, async epoch => {
      const server = await ownedInvoke<McpServer>('update_mcp_server', { serverId, request });
      remember(server, epoch);
      if (epoch === useDevelopment.getState().epoch) {
        set({ tools: get().tools.filter(tool => tool.server_id !== serverId) });
        setRead(`tools:${serverId}`, { status: 'idle' });
      }
      return server;
    }),
    getConfig: serverId => ownedInvoke<CreateMcpServerRequest>('get_mcp_server_config', { serverId }),
    toggle: (serverId, enabled) => action(serverId, async epoch => {
      remember(await ownedInvoke<McpServer>('toggle_mcp_server', { serverId, enabled }), epoch);
    }),
    remove: serverId => action(serverId, async epoch => {
      await ownedInvoke('delete_mcp_server', { serverId });
      if (epoch !== useDevelopment.getState().epoch) return;
      const nextReads = { ...get().reads };
      for (const key of [`tools:${serverId}`, `usage:${serverId}`, `connections:${serverId}`]) delete nextReads[key];
      set({ servers: get().servers.filter(item => item.id !== serverId), tools: get().tools.filter(tool => tool.server_id !== serverId), reads: nextReads });
    }),
    connect: serverId => action(serverId, async epoch => {
      remember(await ownedInvoke<McpServer>('connect_mcp_server', { serverId }), epoch);
      if (epoch !== useDevelopment.getState().epoch) return;
      rememberTools(serverId, await ownedInvoke<McpTool[]>('list_mcp_tools', { serverId }), epoch);
    }),
    disconnect: serverId => action(serverId, async epoch => {
      remember(await ownedInvoke<McpServer>('disconnect_mcp_server', { serverId }), epoch);
    }),
    loadTools: (serverId, refreshIf) => {
      const epoch = useDevelopment.getState().epoch;
      return snapshot<McpTool[]>(`tools:${serverId}`, 'list_mcp_tools', value => rememberTools(serverId, value, epoch), { serverId }, refreshIf);
    },
    discoverTools: serverId => {
      if (get().discoveryByServer[serverId]?.status === 'unconfirmed') return Promise.reject(new Error('The previous tool refresh result is unconfirmed. Read the saved snapshot; do not repeat discovery.'));
      return action(serverId, async epoch => {
        const identity = { operationId: ++discoverySequence, accountEpoch: epoch, serverId };
        const receipt = (state: McpDiscovery) => {
          if (epoch === useDevelopment.getState().epoch) set({ discoveryByServer: { ...get().discoveryByServer, [serverId]: state } });
        };
        receipt({ ...identity, status: 'pending' });
        try {
          const tools = await ownedInvoke<McpTool[]>('refresh_mcp_tools', { serverId });
          rememberTools(serverId, tools, epoch);
          receipt({ ...identity, status: 'succeeded' });
        } catch (error) {
          receipt({ ...identity, status: discoveryDefinitelyFailed(error) ? 'failed' : 'unconfirmed', error: mcpErrorMessage(error) });
          throw error;
        }
      }, false);
    },
    setToken: (serverId, token) => action(serverId, async epoch => {
      remember(await ownedInvoke<McpServer>('set_mcp_bearer_token', { serverId, token }), epoch);
    }),
    clearToken: serverId => action(serverId, async epoch => {
      remember(await ownedInvoke<McpServer>('clear_mcp_bearer_token', { serverId }), epoch);
    }),
    confirmTool: request => ownedInvoke<string>('confirm_mcp_tool', { request }),
    call: request => action(request.server_id, () => ownedInvoke<CallMcpToolResponse>('call_mcp_tool', { request })),
    // Cancellation must bypass the per-server side-effect lock held by call().
    cancelCall: (serverId, callId) => ownedInvoke<boolean>('cancel_mcp_tool', { serverId, callId }),
    fetchLogs: (serverId, refreshIf) => snapshot<McpUsageLog[]>(`usage:${serverId}`, 'list_mcp_usage_logs', value => {
      set({ logsByServer: { ...get().logsByServer, [serverId]: value } });
    }, { serverId, limit: 50 }, refreshIf),
    fetchConnections: (serverId, refreshIf) => snapshot<McpConnectionLog[]>(`connections:${serverId}`, 'list_mcp_connection_logs', value => {
      set({ connectionsByServer: { ...get().connectionsByServer, [serverId]: value } });
    }, { serverId, limit: 50 }, refreshIf),
  };
});
