import { DEFAULTS, DENSITIES, type AppearanceSnapshot } from '../../src/ui/appearance/model';
import type { ChatMessage, ChatSession, ChatRun, ChatRunStep, ChatSummary, ChatStreamPayload, ChatWebSource } from '../../src/stores/chatStore';
import { contextFixture, usageFixture } from './context-usage.fixtures';
export const clock = 1793966400000;
const params = new URLSearchParams(location.search);
export const scenario = params.get('scenario') ?? 'controls';
export const locale = params.get('locale') === 'en-US' ? 'en-US' : 'zh-CN';
const en = locale === 'en-US';
export const replyFixture = params.get('reply');
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
const streams = new Map<string, { channel: Channel<any>; assistantId: string; sessionId: string; seq: number; text: string; run: ChatRun }>();
let sequence = 0;
let previewSummary: ChatSummary | null = null;
const previewSkill = { pin: { skill_id: 'preview-writing', version: '1.0', content_hash: 'preview-only-content-hash' }, name: en ? 'Preview writing guide' : '预览写作指南', description: en ? 'Synthetic metadata; no package is read.' : '合成元数据，不读取真实技能包。', category: 'writing', tags: ['writing'] };
const previewRun: ChatRun = { task_id: 'preview-retained-run', session_id: sessions[0].id, assistant_message_id: 'preview-assistant', source_run_id: null, status: 'interrupted', partial_content: en ? 'Synthetic retained partial response. No external action occurred.' : '合成保留片段，未发生外部动作。', dispatched: false, result_unknown: params.get('fault') === 'chat-unknown', audit_warning: null, total_tokens: null, steps: [{ id: 'preview-load', ordinal: 1, kind: 'skill', phase: 'processing_result', status: 'completed', dispatched: false, result_unknown: false, receipt: { pin: previewSkill.pin, path: 'references/preview.txt', sha256: 'preview-only-resource-hash', bytes: 128 }, created_at: clock / 1000 - 3, updated_at: clock / 1000 - 2 }], created_at: clock / 1000 - 3, updated_at: clock / 1000 - 2 };
if (params.get('fault') === 'chat-unknown') previewRun.steps.push({ id: 'preview-unknown-tool', ordinal: 2, kind: 'tool', phase: 'tool', status: 'dispatched', dispatched: true, result_unknown: true, receipt: null, created_at: clock / 1000 - 2, updated_at: clock / 1000 - 1 });
// Explicitly isolated fixture data: no provider, MCP, credential vault, or network is called.
const previewSources: ChatWebSource[] = [
  { id: 'preview-search-source', url: 'https://example.invalid/search-result', title: en ? 'Fixture search result — no network request' : '预览搜索结果：未发出联网请求', excerpt: en ? 'Synthetic search excerpt for the real source preview component.' : '用于真实来源预览组件的合成搜索片段。', kind: 'search_result', retrieved_at: clock / 1000, truncated: false },
  { id: 'preview-page-source', url: 'https://example.invalid/retained-page', title: en ? 'Fixture retained page — no page was fetched' : '预览已读网页：未实际抓取页面', excerpt: en ? 'Synthetic retained page excerpt. External navigation is disabled in this preview.' : '合成保留网页片段，本预览禁止实际打开外部网页。', kind: 'web_page', retrieved_at: clock / 1000, truncated: true },
];
function fixtureWebStep(id: string, ordinal: number, operation: 'search' | 'fetch', status: 'running' | 'completed' | 'failed', sources: ChatWebSource[] = [], error?: string): ChatRunStep {
  return { id, ordinal, kind: 'web', phase: 'tool', status, dispatched: false, result_unknown: false, receipt: { web_activity: { id, operation, status, ...(operation === 'search' ? { query: en ? 'Synthetic preview research query' : '合成预览检索问题' } : { url: previewSources[1].url }), sources, error, truncated: sources.some(source => source.truncated) } }, created_at: clock / 1000 - 3, updated_at: clock / 1000 - 2 };
}
if (['completed', 'interrupted', 'unknown', 'partial-failure', 'missing'].includes(replyFixture ?? '')) {
  previewRun.steps.push(
    { id: 'preview-round-1', ordinal: 2, kind: 'model_segment', phase: 'model', status: 'completed', dispatched: false, result_unknown: false, receipt: { round: 1, content: en ? 'Fixture intermediate round: I will compare the supplied material with the retained sources.' : '预览中间轮：先对照所选资料与保留来源。' }, created_at: clock / 1000 - 4, updated_at: clock / 1000 - 3 },
    { id: 'preview-context', ordinal: 3, kind: 'context', phase: 'processing_result', status: 'completed', dispatched: false, result_unknown: false, receipt: { context_receipts: [{ resource_id: 'preview-brief.txt', bytes: 256 }] }, created_at: clock / 1000 - 3, updated_at: clock / 1000 - 2 },
    { id: 'preview-read-only-tool', ordinal: 4, kind: 'tool', phase: 'processing_result', status: 'settled', dispatched: true, result_unknown: false, receipt: { server_name: en ? 'Fixture only' : '仅预览数据', tool_name: 'review_sample', outcome: 'success', result: { content: [{ type: 'text', text: en ? 'Synthetic tool result: two review items were found. No real tool executed.' : '合成工具结果：发现两项待核对内容，未执行真实工具。' }] } }, created_at: clock / 1000 - 3, updated_at: clock / 1000 - 2 },
    fixtureWebStep('preview-search', 5, 'search', 'completed', [previewSources[0]]),
    fixtureWebStep('preview-fetch', 6, 'fetch', replyFixture === 'partial-failure' ? 'failed' : 'completed', replyFixture === 'partial-failure' ? [] : [previewSources[1]], replyFixture === 'partial-failure' ? 'web_html_complexity_limit' : undefined),
  );
  previewRun.status = ['completed', 'partial-failure', 'missing'].includes(replyFixture ?? '') ? 'completed' : 'interrupted';
  previewRun.partial_content = en ? 'This is an isolated component fixture, not an actual assistant result. The retained [search excerpt](dgos-source:preview-search-source) can be previewed without leaving the reply.' : '这是隔离组件场景，并非真实助手执行结果。可以直接预览保留的[搜索片段](dgos-source:preview-search-source)，无需离开回复。';
  if (replyFixture !== 'partial-failure') previewRun.partial_content += en ? '\n\nThe [page excerpt](dgos-source:preview-page-source) is truncated. An [unrecognized citation](dgos-source:not-a-host-source) is not evidence.' : '\n\n[网页片段](dgos-source:preview-page-source)已截断。[未绑定引用](dgos-source:not-a-host-source)不会作为证据展示。';
  if (replyFixture === 'unknown') {
    previewRun.result_unknown = true; previewRun.dispatched = true;
    previewRun.steps.push({ id: 'preview-unknown-operation', ordinal: 7, kind: 'tool', phase: 'tool', status: 'dispatched', dispatched: true, result_unknown: true, receipt: { server_name: 'Fixture only', tool_name: 'review_sample', arguments: { scope: 'synthetic-preview-only' } }, created_at: clock / 1000 - 2, updated_at: clock / 1000 - 1 });
  }
  Object.assign(messages[0], { content: en ? 'Isolated fixture: inspect the reply process and retained sources.' : '隔离预览：检查回复过程与保留来源。' });
  Object.assign(messages[1], { task_id: previewRun.task_id, content: previewRun.partial_content, status: previewRun.status, result_unknown: previewRun.result_unknown, dispatched: previewRun.dispatched, finish_reason: replyFixture === 'interrupted' ? 'length' : 'stop' });
}
const retainedRuns = new Map([[previewRun.task_id, previewRun]]);
let previewSearchConfigured = params.get('web') === 'configured';
const previewWebStatus = () => ({ backend: 'brave', search_configured: previewSearchConfigured, fetch_available: true });
function emitPreview(taskId: string, payload: ChatStreamPayload) {
  const stream = streams.get(taskId);
  if (stream) stream.channel.onmessage({ ...payload, seq: ++stream.seq });
}
function finishPreview(taskId: string, status: 'completed' | 'interrupted') {
  const stream = streams.get(taskId);
  if (!stream) return;
  stream.run.status = status; stream.run.partial_content = stream.text;
  const message: ChatMessage = { ...messages[1], id: stream.assistantId, seq: Math.max(...messages.map(item => item.seq)) + 1, session_id: stream.sessionId, task_id: taskId, status, content: stream.text, result_unknown: false, dispatched: false, finish_reason: status === 'completed' ? 'stop' : null };
  messages.push(message);
  emitPreview(taskId, { kind: 'done', message, dispatched: false, result_unknown: false, audit_warning: null });
  streams.delete(taskId);
}
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
    case 'inspect_chat_context': {
      if (scenario === 'chat-context' && previewSummary && params.get('context') === 'refresh-context-failed') throw Error('Isolated context refresh failure');
      const modelId = sessions.find(session => session.id === input.session_id)?.model_id ?? 'preview-model';
      result = ok(scenario === 'chat-context' ? contextFixture(params.get('context'), modelId, previewSummary) : { model_id: modelId, model_window_tokens: params.get('window') === 'known' ? 8192 : null, host_text_limit_bytes: 24576, reserved_output_tokens: 1024, estimated_input_tokens: diagnostics.fault === 'chat-budget' ? 8000 : 1024, text_bytes: diagnostics.fault === 'chat-budget' ? 26000 : 4096, image_reserve_tokens: 0, skill_bytes: 0, tool_schema_bytes: 0, history_bytes: 4000, dropped_turns: diagnostics.fault === 'chat-trimmed' ? 3 : 0, summary: previewSummary, can_send: diagnostics.fault !== 'chat-budget', rejection: diagnostics.fault === 'chat-budget' ? 'preview_budget_limit' : null, can_summarize: !previewSummary, summary_unavailable_reason: previewSummary ? 'no_history' : null }); break;
    }
    case 'get_chat_usage': {
      if (!sessions.some(session => session.id === args.sessionId)) throw Error('Unknown preview session');
      if (previewSummary && params.get('context') === 'refresh-usage-failed') throw Error('Isolated usage refresh failure');
      result = ok(usageFixture(args.sessionId === sessions[1].id ? 'empty' : params.get('context'), previewSummary)); break;
    }
    case 'chat_skill_catalog': result = ok({ items: [previewSkill], recommendations: input.query ? [{ entry: previewSkill, score: 1, reasons: ['lexical_description:preview'] }] : [] }); break;
    case 'get_chat_web_status': result = ok(previewWebStatus()); break;
    case 'configure_chat_web': {
      // Only remember a synthetic configured flag; never retain or echo submitted secret text.
      previewSearchConfigured = input.remove !== true;
      result = ok(previewWebStatus()); break;
    }
    case 'list_chat_runs': result = ok({ items: [...retainedRuns.values()].filter(run => run.session_id === input.session_id) }); break;
    case 'get_chat_run': {
      const run = retainedRuns.get(input.task_id);
      result = run && !(replyFixture === 'missing' && input.task_id === previewRun.task_id) ? ok(run) : { success: false, error: { code: 1006002 } };
      break;
    }
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
      const modelId = sessions.find(session => session.id === input.session_id)?.model_id ?? 'preview-model';
      const run: ChatRun = { ...previewRun, task_id: taskId, assistant_message_id: assistantId, session_id: input.session_id, status: 'running', steps: [], partial_content: '', total_tokens: null, result_unknown: false };
      retainedRuns.set(taskId, run);
      streams.set(taskId, { channel: args.onEvent, assistantId, sessionId: input.session_id, seq: 0, text: '', run });
      setTimeout(() => {
        if (!streams.has(taskId)) return;
        const kind = params.get('context');
        if (kind === 'summary-failed' || kind === 'summary-unknown') {
          run.status = 'failed'; run.result_unknown = kind === 'summary-unknown';
          emitPreview(taskId, { kind: 'error', assistant_message_id: assistantId, message: null, error_code: 'provider_error', provider_code: null, result_unknown: run.result_unknown, regeneration_discarded: false, dispatched: false, audit_warning: null });
        } else {
          previewSummary = { id: taskId, source_first_seq: 1, source_last_seq: 2, source_digest: 'preview-only-source-digest', content: en ? 'Synthetic summary preview; no model was called.' : '合成摘要预览，未调用模型。', provider_id: 'preview-provider', model_id: modelId, created_at: clock / 1000 };
          run.status = 'completed';
          emitPreview(taskId, { kind: 'summary_done', task_id: taskId, assistant_message_id: assistantId, summary: previewSummary, dispatched: false, result_unknown: false, audit_warning: null });
        }
        streams.delete(taskId);
      }, scenario === 'chat-context' ? 1800 : 0);
      result = ok({ task_id: taskId, user_message_id: '', assistant_message_id: assistantId, dropped_turns: 0, replayed: false }); break;
    }
    case 'rename_chat_session': { const s = sessions.find(s => s.id === input.session_id)!; s.title = input.title; result = ok(s); break; }
    case 'send_chat_message': case 'regenerate_chat_message': case 'continue_chat_run': {
      if (scenario === 'chat-error') { result = { success: false, error: { code: 1006005 } }; break; }
      const n = ++sequence;
      const taskId = `preview-task-${n}`;
      const assistantId = `preview-response-${n}`;
      const sessionId = input.session_id ?? sessions[0].id;
      if (command !== 'regenerate_chat_message') messages.push({ ...messages[0], id: `preview-sent-${n}`, seq: Math.max(...messages.map(item => item.seq)) + 1, session_id: sessionId, task_id: taskId, content: input.content ?? input.intent ?? '', status: 'completed', result_unknown: false });
      const run: ChatRun = { ...previewRun, task_id: taskId, assistant_message_id: assistantId, session_id: sessionId, source_run_id: command === 'continue_chat_run' ? input.source_task_id : null, status: 'running', partial_content: '', result_unknown: false, dispatched: false, steps: [] };
      retainedRuns.set(taskId, run);
      streams.set(taskId, { channel: args.onEvent, assistantId, sessionId, seq: 0, text: '', run });
      emitPreview(taskId, { kind: 'started', task_id: taskId, assistant_message_id: assistantId });
      emitPreview(taskId, { kind: 'model_segment', task_id: taskId, assistant_message_id: assistantId, round: 1 });
      emitPreview(taskId, { kind: 'phase', task_id: taskId, assistant_message_id: assistantId, phase: 'model' });
      const firstRound = en ? 'Isolated preview: inspect the retained source before drafting.' : '隔离预览：先核对保留来源，再撰写答复。';
      emitPreview(taskId, { kind: 'delta', assistant_message_id: assistantId, text: firstRound });
      const segment: ChatRunStep = { id: `${taskId}-round-1`, ordinal: 1, kind: 'model_segment', phase: 'model', status: 'completed', dispatched: false, result_unknown: false, receipt: { round: 1, content: firstRound }, created_at: clock / 1000, updated_at: clock / 1000 };
      const search = fixtureWebStep(`${taskId}-search`, 2, 'search', 'completed', [previewSources[0]]);
      run.steps.push(segment, search);
      for (const step of run.steps) emitPreview(taskId, { kind: 'activity', task_id: taskId, assistant_message_id: assistantId, step });
      if (scenario === 'chat-recovery') { run.steps.push(previewRun.steps[0]); emitPreview(taskId, { kind: 'activity', task_id: taskId, assistant_message_id: assistantId, step: previewRun.steps[0] }); }
      emitPreview(taskId, { kind: 'model_segment', task_id: taskId, assistant_message_id: assistantId, round: 2 });
      const stream = streams.get(taskId)!;
      stream.text = en ? 'Isolated preview response using the [retained search excerpt](dgos-source:preview-search-source). No external request occurred.' : '隔离预览回复，使用[保留搜索片段](dgos-source:preview-search-source)。未发出外部请求。';
      emitPreview(taskId, { kind: 'delta', assistant_message_id: assistantId, text: stream.text });
      if (scenario === 'chat-approval') {
        emitPreview(taskId, { kind: 'tool_proposal', assistant_message_id: assistantId, proposal: { id: 'preview-proposal', server_id: 'preview-server', server_name: 'Preview only — no tool executes', tool_name: 'review_sample', schema_hash: 'preview-hash', arguments: { path: '/preview/only', explanation: (en ? 'Long approval parameter; no tool will execute.\n' : '长审批参数；不会执行真实工具。\n').repeat(60) }, expires_at: clock/1000+300 } });
      } else if (scenario !== 'chat-streaming' && scenario !== 'chat-recovery') {
        finishPreview(taskId, 'completed');
      }
      result = ok({ task_id: taskId, user_message_id: `preview-sent-${n}`, assistant_message_id: assistantId, dropped_turns: 0, replayed: false });
      break;
    }
    case 'advance_chat_tool_turn': {
      const stream = streams.get(input.task_id);
      if (stream) {
        const step: ChatRunStep = { id: `${input.task_id}-approval`, ordinal: 3, kind: 'tool', phase: 'processing_result', status: 'settled', dispatched: false, result_unknown: false, receipt: { server_name: 'Fixture only', tool_name: 'review_sample', outcome: input.action?.type === 'approve' ? 'success' : 'denied', result: { content: [{ type: 'text', text: en ? 'Synthetic approval recorded; no actual tool executed.' : '已记录合成审批，未执行真实工具。' }] } }, created_at: clock / 1000, updated_at: clock / 1000 };
        stream.run.steps.push(step);
        emitPreview(input.task_id, { kind: 'activity', task_id: input.task_id, assistant_message_id: stream.assistantId, step });
        finishPreview(input.task_id, 'completed');
      }
      result = ok(null); break;
    }
    case 'cancel_chat_generation': case 'cancel_chat_task': {
      const taskId = input.task_id ?? input.taskId;
      const stream = streams.get(taskId);
      if (taskId?.startsWith('preview-summary-') && stream) {
        stream.run.status = 'cancelled';
        emitPreview(taskId, { kind: 'error', assistant_message_id: stream.assistantId, message: null, error_code: 'task_cancelled', provider_code: null, result_unknown: false, regeneration_discarded: false, dispatched: false, audit_warning: null });
        streams.delete(taskId);
      } else finishPreview(taskId, 'interrupted');
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
