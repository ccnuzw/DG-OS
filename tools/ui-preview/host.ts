import { DEFAULTS, DENSITIES, type AppearanceSnapshot } from '../../src/ui/appearance/model';
import type { ChatMessage, ChatSession, ChatRun, ChatSummary } from '../../src/stores/chatStore';
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
let previewSummary: ChatSummary | null = null;
const previewSkill = { pin: { skill_id: 'preview-writing', version: '1.0', content_hash: 'preview-only-content-hash' }, name: en ? 'Preview writing guide' : '预览写作指南', description: en ? 'Synthetic metadata; no package is read.' : '合成元数据，不读取真实技能包。', category: 'writing', tags: ['writing'] };
const previewRun: ChatRun = { task_id: 'preview-retained-run', session_id: sessions[0].id, assistant_message_id: 'preview-assistant', source_run_id: null, status: 'interrupted', partial_content: en ? 'Synthetic retained partial response. No external action occurred.' : '合成保留片段，未发生外部动作。', dispatched: false, result_unknown: params.get('fault') === 'chat-unknown', audit_warning: null, total_tokens: null, steps: [{ id: 'preview-load', ordinal: 1, kind: 'skill', phase: 'processing_result', status: 'completed', dispatched: false, result_unknown: false, receipt: { pin: previewSkill.pin, path: 'references/preview.txt', sha256: 'preview-only-resource-hash', bytes: 128 }, created_at: clock / 1000 - 3, updated_at: clock / 1000 - 2 }], created_at: clock / 1000 - 3, updated_at: clock / 1000 - 2 };
if (params.get('fault') === 'chat-unknown') previewRun.steps.push({ id: 'preview-unknown-tool', ordinal: 2, kind: 'tool', phase: 'tool', status: 'dispatched', dispatched: true, result_unknown: true, receipt: null, created_at: clock / 1000 - 2, updated_at: clock / 1000 - 1 });
const cliVersions = new Map<string, string>();
const cliTickets = new Map<string, { tool: string; path: string }>();
let cliTicketSequence = 0;
const previewAssets = new Map<string, { id: string; name: string; mime: string; bytes_len: number; base64: string; sessionId?: string; sha256?: string; expires_at?: number }>();
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
    case 'list_mcp_servers': result = [{ id: 'preview-server', name: en ? 'Preview tool service' : '预览工具服务', enabled: true, status: 'ready' }]; break;
    case 'list_mcp_tools': result = [{ server_id: 'preview-server', tool_name: 'review_sample', description: en ? 'Read an isolated sample; no external action.' : '读取隔离样本，不执行外部操作。', input_schema: { type: 'object', properties: { topic: { type: 'string' } } }, schema_hash: 'preview-hash', discovered_at: clock / 1000 }]; break;
    case 'list_installed_skills': result = [{ id: previewSkill.pin.skill_id, skill_id: previewSkill.pin.skill_id, name: previewSkill.name, description: previewSkill.description, category: 'writing', version: '1.0', content_hash: previewSkill.pin.content_hash, enabled: true, call_type: 'content' }]; break;
    case 'list_modules': case 'list_provider_vendors': result = ok([]); break;
    case 'list_providers': result = ok([{ id: 'preview-provider', name: en ? 'Preview service' : '预览模型服务', provider_type: 'openai_compatible', base_url: 'https://example.invalid', enabled: true, credential_status: 'stored', config_revision: 1 }]); break;
    case 'list_models': result = ok(['preview-model', 'preview-alternate'].map((id, index) => ({ id, provider_id: 'preview-provider', model_id: id, model_name: index ? 'Preview alternate' : 'Preview model', model_type: 'text-generation', supports_streaming: true, capabilities: { streaming: 'supported', vision: 'supported', reasoning: params.get('reasoning') === 'unsupported' ? 'unsupported' : 'supported' } }))); break;
    case 'update_chat_session_target': {
      if (diagnostics.fault === 'target-save') { result = { success: false, error: { code: 1006005 } }; break; }
      const index = sessions.findIndex(session => session.id === input.session_id);
      if (index < 0) throw Error('Unknown preview session');
      const prior = sessions[index];
      sessions[index] = { ...prior, provider_id: input.provider_id, model_id: input.model_id, generation: input.generation, no_skill_confirmed: input.no_skill, skills: input.skill_ids.map((id: string) => prior.skills.find(skill => skill.skill_id === id) ?? { ...previewSkill.pin, skill_id: id }) };
      result = ok(structuredClone(sessions[index])); break;
    }
    case 'import_chat_asset': case 'chat_import_ai_resource': {
      const raw = Uint8Array.from(atob(input.base64), char => char.charCodeAt(0));
      const digest = await crypto.subtle.digest('SHA-256', raw);
      const sha256 = Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, '0')).join('');
      const asset = { id: `preview-material-${++sequence}`, name: input.name, mime: input.mime, bytes_len: raw.length, base64: input.base64, sessionId: args.sessionId, sha256, expires_at: clock / 1000 + 86400 };
      previewAssets.set(asset.id, asset);
      result = ok({ id: asset.id, name: asset.name, mime: asset.mime, bytes_len: asset.bytes_len, sha256, expires_at: asset.expires_at }); break;
    }
    case 'preview_chat_asset': {
      const asset = previewAssets.get(args.assetId);
      if (!asset || asset.sessionId) throw Error('Unknown preview attachment');
      result = ok({ asset: { id: asset.id, name: asset.name, mime: asset.mime, bytes_len: asset.bytes_len }, base64: asset.base64 }); break;
    }
    case 'preview_chat_resource': {
      const asset = previewAssets.get(input.resource_id);
      if (!asset || asset.sessionId !== input.session_id || asset.sha256 !== input.sha256) throw Error('Unknown preview resource');
      result = ok({ base64: asset.base64, mime: asset.mime, start: 0, bytes: asset.bytes_len, total_bytes: asset.bytes_len, truncated: false }); break;
    }
    case 'remove_chat_asset': case 'chat_delete_ai_resource': result = ok(previewAssets.delete(args.assetId ?? args.resourceId)); break;
    case 'list_chat_sessions': result = ok({ items: sessions, next_cursor: null }); break;
    case 'get_chat_session': result = ok(sessions.find(s => s.id === input.session_id) ?? sessions[0]); break;
    case 'list_chat_messages': result = ok({ items: input.session_id === sessions[1].id ? [] : messages, next_before_seq: null }); break;
    case 'inspect_chat_context': result = ok({ model_id: 'preview-model', model_window_tokens: params.get('window') === 'known' ? 8192 : null, host_text_limit_bytes: 24576, reserved_output_tokens: 1024, estimated_input_tokens: diagnostics.fault === 'chat-budget' ? 8000 : 1024, text_bytes: diagnostics.fault === 'chat-budget' ? 26000 : 4096, image_reserve_tokens: 0, skill_bytes: 0, tool_schema_bytes: 0, history_bytes: 4000, dropped_turns: diagnostics.fault === 'chat-trimmed' ? 3 : 0, summary: previewSummary, can_send: diagnostics.fault !== 'chat-budget', rejection: diagnostics.fault === 'chat-budget' ? 'preview_budget_limit' : null }); break;
    case 'chat_skill_catalog': result = ok({ items: [previewSkill], recommendations: input.query ? [{ entry: previewSkill, score: 1, reasons: ['lexical_description:preview'] }] : [] }); break;
    case 'list_chat_runs': result = ok({ items: input.session_id === sessions[0].id ? [previewRun] : [] }); break;
    case 'get_chat_run': result = ok(previewRun); break;
    case 'resolve_chat_run_step': {
      const step = previewRun.steps.find(entry => entry.id === input.step_id);
      if (input.task_id !== previewRun.task_id || !step || input.confirmed !== true || !input.note?.trim()) { result = { success: false, error: { code: 1006017 } }; break; }
      step.receipt = { source: 'user_report', synthetic_preview_only: true, outcome: input.outcome, note: input.note, original_receipt: step.receipt };
      step.status = 'manually_resolved'; step.result_unknown = false;
      previewRun.result_unknown = previewRun.steps.some(entry => entry.result_unknown);
      result = ok(previewRun); break;
    }
    case 'summarize_chat_context': {
      const taskId = `preview-summary-${++sequence}`;
      const assistantId = `${taskId}-assistant`;
      previewSummary = { id: taskId, source_first_seq: 1, source_last_seq: 2, source_digest: 'preview-only-source-digest', content: en ? 'Synthetic summary preview; no model was called.' : '合成摘要预览，未调用模型。', provider_id: 'preview-provider', model_id: 'preview-model', created_at: clock / 1000 };
      args.onEvent.onmessage({ kind: 'summary_done', task_id: taskId, assistant_message_id: assistantId, summary: previewSummary, dispatched: false, result_unknown: false, audit_warning: null });
      result = ok({ task_id: taskId, user_message_id: '', assistant_message_id: assistantId, dropped_turns: 0, replayed: false }); break;
    }
    case 'rename_chat_session': { const s = sessions.find(s => s.id === input.session_id)!; s.title = input.title; result = ok(s); break; }
    case 'send_chat_message': case 'regenerate_chat_message': case 'continue_chat_run': {
      if (scenario === 'chat-error') { result = { success: false, error: { code: 1006005 } }; break; }
      const n = ++sequence;
      const taskId = `preview-task-${n}`;
      const assistantId = `preview-response-${n}`;
      streams.set(taskId, { channel: args.onEvent, assistantId, sessionId: input.session_id ?? sessions[0].id });
      args.onEvent.onmessage({ kind: 'phase', task_id: taskId, assistant_message_id: assistantId, phase: 'model' });
      if (scenario === 'chat-recovery') args.onEvent.onmessage({ kind: 'activity', task_id: taskId, assistant_message_id: assistantId, step: previewRun.steps[0] });
      args.onEvent.onmessage({ kind: 'delta', assistant_message_id: assistantId, text: en ? 'Preview streaming response. No external request.' : '预览流式回复，未发出外部请求。' });
      if (scenario === 'chat-approval') args.onEvent.onmessage({ kind: 'tool_proposal', assistant_message_id: assistantId, proposal: { id: 'preview-proposal', server_id: 'preview-server', server_name: 'Preview only', tool_name: 'review_sample', schema_hash: 'preview-hash', arguments: { path: '/preview/only', explanation: (en ? 'Long approval parameter; no tool will execute.\n' : '长审批参数；不会执行真实工具。\n').repeat(60) }, expires_at: clock/1000+300 } });
      result = ok({ task_id: taskId, user_message_id: `preview-sent-${n}`, assistant_message_id: assistantId, dropped_turns: 0, replayed: false });
      break;
    }
    case 'advance_chat_tool_turn': result = ok(null); break;
    case 'cancel_chat_generation': case 'cancel_chat_task': {
      const stream = streams.get(input.task_id ?? input.taskId);
      if (stream) { stream.channel.onmessage({ kind: 'done', message: { ...messages[1], id: stream.assistantId, session_id: stream.sessionId, status: 'interrupted' }, dispatched: false, result_unknown: false, audit_warning: null }); streams.delete(input.task_id ?? input.taskId); }
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

// Isolated browser events only; this preview never subscribes to native account events.
export async function listen<T>(event: string, handler: (event: { payload: T }) => void): Promise<() => void> {
  const listener = (incoming: Event) => handler({ payload: (incoming as CustomEvent<T>).detail });
  window.addEventListener(`preview:${event}`, listener);
  return () => window.removeEventListener(`preview:${event}`, listener);
}
