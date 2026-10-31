import { invoke } from '@tauri-apps/api/core';
import type { McpServer, McpTool } from '../../stores/mcpStore';
import type { Model, Provider } from '../../stores/providerStore';
import type { Skill } from '../../stores/skillsStore';

// Read-only lists the chat page needs to choose a target and tools. Same commands and filters as the single-task
// page (Provider/Skills commands return a CommandResponse or the bare list, MCP commands return bare lists).

interface Envelope<T> { success: boolean; data?: T }

async function enveloped<T>(command: string, args: Record<string, unknown>): Promise<T> {
  const response = await invoke<Envelope<T>>(command, args);
  if (!response?.success || response.data === undefined) throw new Error(command);
  return response.data;
}

export async function fetchProviders(): Promise<Provider[]> {
  const all = await enveloped<Provider[]>('list_providers', {});
  return all.filter(provider => provider.enabled && provider.credential_status === 'stored');
}

export async function fetchModels(providerId: string): Promise<Model[]> {
  const all = await enveloped<Model[]>('list_models', { providerId });
  return all.filter(model => model.model_type === 'text-generation' || model.model_type === 'chat');
}

export async function fetchSkills(): Promise<Skill[]> {
  const all = await invoke<Skill[]>('list_installed_skills', {});
  return all.filter(skill => skill.enabled && skill.call_type === 'content');
}

export interface ConnectedTool extends McpTool { server_name: string }

export const toolKey = (tool: McpTool): string => JSON.stringify([tool.server_id, tool.tool_name, tool.schema_hash]);

export async function fetchConnectedTools(): Promise<ConnectedTool[]> {
  const servers = await invoke<McpServer[]>('list_mcp_servers');
  const groups = await Promise.all(servers.filter(server => server.enabled && server.status === 'ready').map(async server => {
    const listed = await invoke<McpTool[]>('list_mcp_tools', { serverId: server.id });
    return listed.map(tool => ({ ...tool, server_name: server.name }));
  }));
  return groups.flat();
}
