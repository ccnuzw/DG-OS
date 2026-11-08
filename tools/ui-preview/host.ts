import { DEFAULTS, DENSITIES, type AppearanceSnapshot } from '../../src/ui/appearance/model';
import type { ChatMessage, ChatSession } from '../../src/stores/chatStore';
export const clock = 1793966400000;
const params = new URLSearchParams(location.search);
export const scenario = params.get('scenario') ?? 'controls';
export const locale = params.get('locale') === 'en-US' ? 'en-US' : 'zh-CN';
const en = locale === 'en-US';
const preferences = structuredClone(DEFAULTS);
preferences.mode = params.get('theme') === 'dark' ? 'dark' : 'light';
const density = Number(params.get('density'));
if ((DENSITIES as readonly number[]).includes(density)) preferences.density = density as typeof preferences.density;
for (const key of ['uiSize', 'readingSize', 'codeSize'] as const) {
  const value = Number(params.get(key));
  if (Number.isInteger(value) && value >= 12 && value <= (key === 'uiSize' ? 20 : 24)) preferences.typography[key] = value;
}
if (params.has('readingSize')) preferences.typography.readingFollowUi = false;
preferences.effects.reducedMotion = 'reduce';
let appearance: AppearanceSnapshot = { ...preferences, revision: 1 };
let languagePreference: 'zh-CN' | 'en-US' | 'system' = locale;
let languageLocale: 'zh-CN' | 'en-US' = locale;
interface CliProbe { status: string; path: string | null; version: string | null }
interface CliUpdate { status: string; current_version: string | null; latest_version: string | null; method: string | null }
type CliTool = 'codex' | 'claude' | 'gemini' | 'node';
interface CliSnapshot { revision: number; paths: Record<CliTool, string | null>; detections: Partial<Record<CliTool, CliProbe & { update: CliUpdate | null }>> }
const cliStorage = params.get('cliStorage');
const storedCli = cliStorage ? localStorage.getItem(`dgos-cli-preview-${cliStorage}`) : null;
let saved: CliSnapshot = storedCli ? JSON.parse(storedCli) : { revision: 1, paths: { codex: '/preview/configured/codex', claude: null, gemini: null, node: null }, detections: {} };
function persistCli() { if (cliStorage) localStorage.setItem(`dgos-cli-preview-${cliStorage}`, JSON.stringify(saved)); }
let desktop = { revision: 1, values: { pinnedModuleIds: [] as string[], showEntries: true, showRecent: true, sidebarCollapsed: false } };
export const sessions: ChatSession[] = [1,2].map(n => ({ id: `preview-${n}`, title: en ? `Preview conversation ${n} — long title for keyboard navigation` : `预览会话${n}：长标题与键盘导航`, title_source: 'user', provider_id: 'preview-provider', model_id: 'preview-model', skills: [], no_skill_confirmed: true, generation: {}, created_at: clock/1000, updated_at: clock/1000 }));
const paragraph = en ? 'This preview uses the production chat surface. Long text wraps in the reading column; code keeps its own font and scroll area. ' : '此预览使用生产聊天组件。长正文沿阅读列换行，代码保留独立字体和滚动区域。';
export const messages: ChatMessage[] = [
  { id: 'preview-user', role: 'user', content: en ? 'Review this long response and code sample.' : '请检查长消息和代码的阅读与滚动。' },
  { id: 'preview-assistant', role: 'assistant', content: `${paragraph.repeat(12)}\n\n\`\`\`typescript\n${Array.from({length:24}, (_,i) => `const previewRow${i} = '${'long-path/'.repeat(12)}';`).join('\n')}\n\`\`\`\n\n${paragraph.repeat(12)}` },
].map((m, i) => ({ ...m, role: m.role as 'user'|'assistant', session_id: sessions[0].id, seq: i+1, status: 'completed', task_id: null, finish_reason: null, error_code: null, total_tokens: 256, result_unknown: false, created_at: clock/1000, updated_at: clock/1000 }));
export const diagnostics = { ready: false, scenario, clock, calls: [] as { command: string }[], blocked: [] as string[], fault: params.get('fault') ?? 'none', saved: () => structuredClone(saved) };
Object.assign(window, { __preview: diagnostics });
const ok = (data: unknown) => ({ success: true, data });
export class Channel<T> { onmessage: (event: T) => void = () => {}; }
const streams = new Map<string, { channel: Channel<any>; assistantId: string; sessionId: string }>();
let sequence = 0;
const cliVersions = new Map<string, string>();
const cliTickets = new Map<string, { tool: string; path: string }>();
let cliTicketSequence = 0;
export async function invoke<T>(command: string, args: Record<string, any> = {}): Promise<T> {
  diagnostics.calls.push({ command });
  const input = args.input ?? args;
  if ((diagnostics.fault === 'initial-error' && command === 'get_cli_preferences') || (diagnostics.fault === 'probe-error' && command === 'probe_cli')) throw Error('Preview fixture failure');
  let result: unknown;
  switch (command) {
    case 'read_workspace_app_catalog': result = ok({ catalogRevision: 'preview', entries: [] }); break;
    case 'get_desktop_preferences': result = ok(structuredClone(desktop)); break;
    case 'set_desktop_preferences':
      desktop = { revision: desktop.revision+1, values: structuredClone(args.values) };
      result = ok({ kind: 'saved', snapshot: structuredClone(desktop) }); break;
    case 'get_appearance_preferences': result = ok({ kind: 'ready', snapshot: appearance, repairWarnings: [] }); break;
    case 'set_appearance_preferences': appearance = { ...args.snapshot, revision: appearance.revision+1 }; result = ok({ kind: 'saved', snapshot: appearance }); break;
    case 'get_language_preferences': result = ok({ preference: languagePreference, locale: languageLocale, catalog_version: 'v1' }); break;
    case 'set_language_preferences':
      languagePreference = args.language;
      languageLocale = args.language === 'system' ? locale : args.language;
      result = ok({ preference: languagePreference, locale: languageLocale, catalog_version: 'v1' }); break;
    case 'get_cli_preferences': result = ok(structuredClone(saved)); break;
    case 'set_cli_preferences':
      if (diagnostics.fault === 'unknown') throw Error('Preview result unknown');
      if (diagnostics.fault === 'conflict') { result = ok({ saved: false, snapshot: saved }); break; }
      if (args.expectedRevision !== saved.revision) { result = ok({ saved: false, snapshot: structuredClone(saved) }); break; }
      saved = { ...saved, revision: saved.revision+1, paths: structuredClone(args.paths) };
      saved.detections.node = { status: 'available', path: saved.paths.node, version: 'v22.1.0', update: null };
      persistCli(); result = ok({ saved: true, snapshot: structuredClone(saved) }); break;
    case 'probe_cli': {
      const tool = args.tool as CliTool;
      const probe = { status: 'available', path: args.path ?? `/preview/bin/${tool}`, version: cliVersions.get(tool) ?? '1.0.0' };
      saved.paths[tool] = probe.path; saved.detections[tool] = { ...probe, update: null }; saved.revision += 1; persistCli();
      result = ok({ ...probe, managed: false }); break;
    }
    case 'check_cli_update': {
      const delay = Number(params.get('cliCheckDelay'));
      if (delay > 0) await new Promise<void>(resolve => setTimeout(resolve, Math.min(delay, 25000)));
      const status = diagnostics.fault === 'check-error' ? 'check_failed' : diagnostics.fault === 'unsupported' ? 'unsupported' : diagnostics.fault === 'up-to-date' || cliVersions.has(args.tool) ? 'up_to_date' : 'available';
      const ticket = status === 'available' ? `preview-ticket-${++cliTicketSequence}` : null;
      if (ticket) cliTickets.set(ticket, { tool: args.tool, path: args.path ?? `/preview/bin/${args.tool}` });
      const update = { status, current_version: cliVersions.get(args.tool) ?? '1.0.0', latest_version: '1.1.0', method: 'npm' };
      const record = saved.detections[args.tool as CliTool];
      if (record) { record.update = update; saved.revision += 1; persistCli(); }
      result = ok({ ...update, ticket }); break;
    }
    case 'update_cli': {
      const installation = cliTickets.get(args.ticket);
      cliTickets.delete(args.ticket);
      const status = !installation ? 'stale' : diagnostics.fault === 'unknown' ? 'unknown' : diagnostics.fault === 'update-failed' ? 'failed' : diagnostics.fault === 'stale' ? 'stale' : diagnostics.fault === 'busy' ? 'busy' : 'updated';
      if (status === 'updated' && installation) {
        cliVersions.set(installation.tool, '1.1.0');
        const tool = installation.tool as CliTool;
        saved.paths[tool] = installation.path; saved.detections[tool] = { status: 'available', path: installation.path, version: '1.1.0', update: null }; saved.revision += 1; persistCli();
      }
      result = ok({ status, probe: status === 'updated' && installation ? { status: 'available', path: installation.path, version: '1.1.0' } : null }); break;
    }
    case 'probe_node_directory':
      result = ok(diagnostics.fault === 'node-invalid' ? { status: 'not_installed', path: null, version: null } : { status: 'available', path: '/preview/node-install/bin/node', version: 'v22.1.0' }); break;
    case 'list_local_fonts': result = ok([{family:'Arial',monospace:false},{family:'Menlo',monospace:true}]); break;
    case 'list_mcp_servers': case 'list_installed_skills': result = []; break;
    case 'list_modules': case 'list_provider_vendors': result = ok([]); break;
    case 'list_providers': result = ok([{ id: 'preview-provider', name: en ? 'Preview only provider' : '仅预览 Provider', provider_type: 'openai', base_url: 'https://example.invalid', enabled: true, credential_status: 'stored', config_revision: 1 }]); break;
    case 'list_models': result = ok([{ id: 'preview-model', provider_id: 'preview-provider', model_id: 'preview-model', model_name: 'Preview model', model_type: 'text-generation', supports_streaming: true }]); break;
    case 'list_chat_sessions': result = ok({ items: sessions, next_cursor: null }); break;
    case 'get_chat_session': result = ok(sessions.find(s => s.id === input.session_id) ?? sessions[0]); break;
    case 'list_chat_messages': result = ok({ items: input.session_id === sessions[1].id ? [] : messages, next_before_seq: null }); break;
    case 'rename_chat_session': { const s = sessions.find(s => s.id === input.session_id)!; s.title = input.title; result = ok(s); break; }
    case 'send_chat_message': case 'regenerate_chat_message': {
      if (scenario === 'chat-error') { result = { success: false, error: { code: 1006005 } }; break; }
      const n = ++sequence;
      const taskId = `preview-task-${n}`;
      const assistantId = `preview-response-${n}`;
      streams.set(taskId, { channel: args.onEvent, assistantId, sessionId: input.session_id ?? sessions[0].id });
      args.onEvent.onmessage({ kind: 'delta', assistant_message_id: assistantId, text: en ? 'Preview streaming response. No external request.' : '预览流式回复，未发出外部请求。' });
      if (scenario === 'chat-approval') args.onEvent.onmessage({ kind: 'tool_proposal', assistant_message_id: assistantId, proposal: { id: 'preview-proposal', server_id: 'preview-server', server_name: 'Preview only', tool_name: 'review_sample', schema_hash: 'preview-hash', arguments: { path: '/preview/only', explanation: (en ? 'Long approval parameter; no tool will execute.\n' : '长审批参数；不会执行真实工具。\n').repeat(60) }, expires_at: clock/1000+300 } });
      result = ok({ task_id: taskId, user_message_id: `preview-sent-${n}`, assistant_message_id: assistantId, dropped_turns: 0, replayed: false });
      break;
    }
    case 'advance_chat_tool_turn': result = ok(null); break;
    case 'cancel_chat_generation': case 'cancel_chat_task': {
      const stream = streams.get(input.task_id ?? input.taskId);
      if (stream) stream.channel.onmessage({ kind: 'done', message: { ...messages[1], id: stream.assistantId, session_id: stream.sessionId, status: 'interrupted' } });
      result = ok(null); break;
    }
    case 'get_lan_status': result = ok({ enabled: false, state: 'stopped', address: null, preferred_port: 8635, port_fallback: false, failure_code: null, session_count: 0, code_expires_at: null }); break;
    case 'list_lan_devices': result = ok([]); break;
    case 'list_lan_audit_events': result = ok({ items: [], next_cursor: null }); break;
    case 'get_storage_overview': result = ok({ categories: ['old_chats','session_assets','temp_files','cache','activity_records'].map(category => ({ category, status: 'empty', count: 0, bytes: 0 })) }); break;
    default:
      diagnostics.blocked.push(command);
      throw Error(`Preview blocks command: ${command}`);
  }
  return result as T;
}
export const getName = async () => 'DG-OS Preview';
export const getVersion = async () => 'preview';
export const getTauriVersion = async () => 'preview — no native host';
export async function openUrl(_url: string): Promise<void> { diagnostics.blocked.push('openUrl'); throw Error('External navigation disabled in preview'); }
export async function open(): Promise<string | null> {
  diagnostics.calls.push({ command: 'plugin-dialog.open' });
  if (diagnostics.fault === 'node-picker-error') throw Error('Preview picker failure');
  return diagnostics.fault === 'node-cancel' ? null : '/preview/node-install';
}
