import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';

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
interface McpState {
  servers: McpServer[]; tools: McpTool[]; logsByServer: Record<string, McpUsageLog[]>;
  connectionsByServer: Record<string, McpConnectionLog[]>;
  loading: boolean; pending: string[]; error: string | null;
  fetch: () => Promise<void>;
  create: (request: CreateMcpServerRequest) => Promise<McpServer>;
  update: (id: string, request: Partial<CreateMcpServerRequest>) => Promise<McpServer>;
  getConfig: (id: string) => Promise<CreateMcpServerRequest>;
  toggle: (id: string, enabled: boolean) => Promise<void>;
  remove: (id: string) => Promise<void>;
  connect: (id: string) => Promise<void>;
  disconnect: (id: string) => Promise<void>;
  loadTools: (id: string) => Promise<void>;
  discoverTools: (id: string) => Promise<void>;
  setToken: (id: string, token: string) => Promise<void>;
  clearToken: (id: string) => Promise<void>;
  confirmTool: (request: CallMcpToolRequest) => Promise<string>;
  call: (request: CallMcpToolRequest) => Promise<CallMcpToolResponse>;
  cancelCall: (serverId: string, callId: string) => Promise<boolean>;
  fetchLogs: (id: string) => Promise<void>;
  fetchConnections: (id: string) => Promise<void>;
}
export const mcpErrorMessage = (error: unknown) => {
  const message = typeof error === 'string' ? error : error instanceof Error ? error.message : 'MCP operation failed. Please retry.';
  return message.includes("reading 'invoke'") ? 'MCP management requires the DGOS desktop app. This browser preview has no desktop connection.' : message;
};

export const useMcpStore = create<McpState>((set, get) => {
  const remember = (server: McpServer) => set({ servers: get().servers.map(item => item.id === server.id ? server : item) });
  const rememberTools = (id: string, tools: McpTool[]) => set({ tools: [...get().tools.filter(tool => tool.server_id !== id), ...tools] });
  // A per-server action lock prevents duplicate side effects even before React re-renders.
  const action = async <T,>(key: string, operation: () => Promise<T>): Promise<T> => {
    if (get().pending.includes(key)) throw new Error('An operation is already running for this server.');
    set({ pending: [...get().pending, key], error: null });
    try { return await operation(); }
    catch (error) { set({ error: mcpErrorMessage(error) }); throw error; }
    finally { set({ pending: get().pending.filter(id => id !== key) }); }
  };
  return {
    servers: [], tools: [], logsByServer: {}, connectionsByServer: {}, loading: false, pending: [], error: null,
    fetch: async () => {
      if (get().loading) return;
      set({ loading: true });
      try { set({ servers: await invoke<McpServer[]>('list_mcp_servers') }); }
      catch (error) { set({ error: mcpErrorMessage(error) }); }
      finally { set({ loading: false }); }
    },
    create: request => action('new', async () => {
      const server = await invoke<McpServer>('create_mcp_server', { request });
      set({ servers: [...get().servers, server] });
      return server;
    }),
    update: (serverId, request) => action(serverId, async () => {
      const server = await invoke<McpServer>('update_mcp_server', { serverId, request });
      remember(server); rememberTools(serverId, []); return server;
    }),
    getConfig: serverId => invoke<CreateMcpServerRequest>('get_mcp_server_config', { serverId }),
    toggle: (serverId, enabled) => action(serverId, async () => {
      remember(await invoke<McpServer>('toggle_mcp_server', { serverId, enabled }));
    }),
    remove: serverId => action(serverId, async () => {
      await invoke('delete_mcp_server', { serverId });
      set({ servers: get().servers.filter(item => item.id !== serverId) });
      rememberTools(serverId, []);
    }),
    connect: serverId => action(serverId, async () => {
      remember(await invoke<McpServer>('connect_mcp_server', { serverId }));
      rememberTools(serverId, await invoke<McpTool[]>('list_mcp_tools', { serverId }));
    }),
    disconnect: serverId => action(serverId, async () => {
      remember(await invoke<McpServer>('disconnect_mcp_server', { serverId }));
    }),
    loadTools: async serverId => {
      try { rememberTools(serverId, await invoke<McpTool[]>('list_mcp_tools', { serverId })); }
      catch (error) { set({ error: mcpErrorMessage(error) }); }
    },
    discoverTools: serverId => action(serverId, async () => {
      rememberTools(serverId, await invoke<McpTool[]>('refresh_mcp_tools', { serverId }));
    }),
    setToken: (serverId, token) => action(serverId, async () => {
      remember(await invoke<McpServer>('set_mcp_bearer_token', { serverId, token }));
    }),
    clearToken: serverId => action(serverId, async () => {
      remember(await invoke<McpServer>('clear_mcp_bearer_token', { serverId }));
    }),
    confirmTool: request => invoke<string>('confirm_mcp_tool', { request }),
    call: request => action(request.server_id, () => invoke<CallMcpToolResponse>('call_mcp_tool', { request })),
    // Cancellation must bypass the per-server side-effect lock held by call().
    cancelCall: (serverId, callId) => invoke<boolean>('cancel_mcp_tool', { serverId, callId }),
    fetchLogs: async serverId => {
      try {
        const logs = await invoke<McpUsageLog[]>('list_mcp_usage_logs', { serverId, limit: 50 });
        set({ logsByServer: { ...get().logsByServer, [serverId]: logs } });
      } catch (error) { set({ error: mcpErrorMessage(error) }); }
    },
    fetchConnections: async serverId => {
      try {
        const logs = await invoke<McpConnectionLog[]>('list_mcp_connection_logs', { serverId, limit: 50 });
        set({ connectionsByServer: { ...get().connectionsByServer, [serverId]: logs } });
      } catch (error) { set({ error: mcpErrorMessage(error) }); }
    },
  };
});
