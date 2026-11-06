import { invoke } from '@tauri-apps/api/core';
import type { AgentEvent, Json } from '../../stores/agentRunnerStore';
import type { Outcome } from '../../stores/developmentStore';
import { hostCode } from '../../stores/hostError';

export interface AgentOutputReference {
  path: string;
  line?: number;
  source: 'fileChange' | 'tool' | 'diagnostic';
  diff: boolean;
}
export interface AgentReferenceActions {
  onOpenReference?: (reference: AgentOutputReference, mode: 'file' | 'diff') => void | Promise<void>;
  onSelectRun?: (runId: string) => void;
}
const object = (value: Json | undefined): { [key: string]: Json } => value && typeof value === 'object' && !Array.isArray(value) ? value : {};
const lineNumber = (value: Json | undefined): number | undefined => typeof value === 'number' && Number.isSafeInteger(value) && value > 0 && value <= 0xffffffff ? value : undefined;
// This is only a projection filter. The host remains the filesystem authority.
function pathHint(value: Json | undefined): string | null {
  if (typeof value !== 'string' || !value || value.length > 4096 || /[\u0000-\u001f\\]/.test(value) || /^[a-z][a-z\d+.-]*:/i.test(value) || value.split('/').some(part => part === '..' || part === '.git')) return null;
  return value;
}
export function agentEventFailed(event: AgentEvent): boolean {
  const data = object(event.data);
  return ['error', 'failure'].includes(event.kind) || Boolean(data.error) || data.run_status === 'failed' || data.status === 'failed' || data.success === false || data.is_error === true || typeof data.exitCode === 'number' && data.exitCode !== 0 || Array.isArray(data.content) && data.content.some(block => object(block).type === 'tool_result' && object(block).is_error === true);
}

/** Only native structured tool fields and compiler-style diagnostic output qualify.
 * Assistant/user text, markdown links, shell commands and patches are never parsed.
 */
export function agentOutputReferences(event: AgentEvent, toolNames: ReadonlyMap<string, string> = new Map()): { references: AgentOutputReference[]; unresolved: boolean } {
  const data = object(event.data);
  const references: AgentOutputReference[] = [];
  let unresolved = false;
  const add = (value: Json | undefined, source: AgentOutputReference['source'], diff: boolean, line?: number) => {
    const path = pathHint(value);
    if (!path) { unresolved = true; return; }
    if (!references.some(ref => ref.path === path && ref.line === line && ref.diff === diff)) references.push({ path, source, diff, ...(line ? { line } : {}) });
  };
  const diagnostics = (value: Json | undefined) => {
    if (!Array.isArray(value)) return;
    for (const diagnostic of value) {
      const item = object(diagnostic);
      const location = object(item.location);
      const path = item.file_path ?? item.path ?? location.path;
      const line = lineNumber(item.line ?? location.line);
      if (path && line) add(path, 'diagnostic', false, line);
      else unresolved = true;
    }
  };
  const output = (value: Json | undefined, cwd?: string) => {
    if (typeof value !== 'string') return;
    // A full compiler diagnostic, not a free-form mention of a filename. Relative
    // locations need a structured cwd; otherwise only absolute paths qualify.
    for (const row of value.slice(0, 256 * 1024).split('\n')) {
      const match = row.match(/^([^\s:][^:\n]*?):([1-9]\d*):([1-9]\d*):\s*(?:fatal error|error|warning)(?:\s+[A-Z]+\d+)?:\s+\S/) ?? row.match(/^([^\s:][^:\n]*?)\(([1-9]\d*),([1-9]\d*)\):\s*(?:error|warning)\s+[A-Z]+\d+:\s+\S/);
      if (!match) continue;
      const line = lineNumber(Number(match[2]));
      const path = match[1];
      if (!line || !path.startsWith('/') && !cwd) { unresolved = true; continue; }
      add(path.startsWith('/') ? path : `${cwd!.replace(/\/$/, '')}/${path}`, 'diagnostic', false, line);
    }
  };
  if ((event.kind === 'fileChange' || event.kind === 'item/started') && data.type === 'fileChange' && Array.isArray(data.changes)) {
    for (const change of data.changes) {
      const item = object(change);
      add(item.path, 'fileChange', true);
    }
  }
  if (event.kind === 'commandExecution' && data.type === 'commandExecution') {
    diagnostics(data.diagnostics);
    output(data.aggregatedOutput, pathHint(data.cwd) ?? undefined);
  }
  if (event.kind === 'message' && Array.isArray(data.content)) {
    for (const block of data.content) {
      const item = object(block);
      if (data.type === 'assistant' && item.type === 'tool_use' && typeof item.name === 'string' && ['Read', 'Edit', 'Write', 'MultiEdit', 'NotebookEdit'].includes(item.name)) {
        const input = object(item.input);
        add(input.file_path ?? input.notebook_path, 'tool', item.name !== 'Read', item.name === 'Read' ? lineNumber(input.offset) : undefined);
      }
      if (data.type === 'user' && item.type === 'tool_result' && typeof item.tool_use_id === 'string' && toolNames.get(`${event.run ?? ''}:${item.tool_use_id}`) === 'Bash') {
        const result = object(data.tool_use_result);
        diagnostics(result.diagnostics);
        output(result.stderr);
        if (typeof item.content === 'string') output(item.content);
        else if (Array.isArray(item.content)) for (const content of item.content) if (object(content).type === 'text') output(object(content).text);
      }
    }
  }
  return { references, unresolved };
}

export async function resolveDevelopmentFileReference(input: {
  project_id: string; task_id: string; session_id: string; path: string; mode: 'file' | 'diff'; line?: number;
}): Promise<Outcome<{ path: string; line: number | null }>> {
  try { return { ok: true, value: await invoke<{ path: string; line: number | null }>('resolve_development_file_reference', { input }) }; }
  catch (error) { return { ok: false, code: hostCode(error) }; }
}
