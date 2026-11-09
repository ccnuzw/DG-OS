import { query, getSessionMessages, type SDKUserMessage, type PermissionResult, type Query } from '@anthropic-ai/claude-agent-sdk';
import { z } from 'zod';
import { createInterface } from 'node:readline';
import { randomUUID, createHash } from 'node:crypto';
import { projectSupportedCommands } from './native-observation';
import { applyTurnSettings, projectTurnControls, turnSettingsSchema, validateTurnSettings, type TurnControls } from './turn-controls';

const emit = (method: string, params: unknown) => process.stdout.write(JSON.stringify({ method, params }) + '\n');
const reply = (id: unknown, result: unknown) => process.stdout.write(JSON.stringify({ id, result }) + '\n');
let agent: Query | undefined;
let sessionId: string = randomUUID();
let running = false;
let closed = false;
let opening = false;
let opened = false;
let providerEvidence: ProviderEvidence | undefined;
let activeProvider: z.infer<typeof providerSchema> | undefined;
let activeProviderEnv: Record<string, string> | undefined;
let nativeProtocolSupported = false;
let nativeControlBusy = false;
let wake: (() => void) | undefined;
let turnControls: TurnControls | undefined;
let activeRequestId: string | undefined;
const projectedMessages = new Set<string>();
const projectedResults = new Set<string>();
const queue: SDKUserMessage[] = [];
const receipts = new Map<string, string>();
const permissionRequestIds = new Set<string>();
const permissions = new Map<string, { tool: string; input: Record<string, unknown>; resolve: (result: PermissionResult) => void }>();
const record = z.record(z.string(), z.unknown());
const commandMetadata = z.object({ name: z.string(), description: z.string().optional(), argumentHint: z.string().optional(), builtin: z.boolean().optional() });
const initMetadata = z.object({ slash_commands: z.array(z.string()).optional(), skills: z.array(z.string()).optional(), mcp_servers: z.array(z.object({ name: z.string(), status: z.string(), source: z.string().optional() })).optional(), plugins: z.array(z.object({ name: z.string(), path: z.string(), version: z.string().optional() })).optional() });
// Only the host's private stdio accepts provider credentials. Never serialize this object.
const routeDisableKeys = [
  'CLAUDE_CODE_USE_BEDROCK', 'CLAUDE_CODE_USE_VERTEX', 'CLAUDE_CODE_USE_FOUNDRY',
  'CLAUDE_CODE_USE_ANTHROPIC_AWS', 'CLAUDE_CODE_USE_ANTHROPIC_GOOGLE_CLOUD',
  'CLAUDE_CODE_USE_MANTLE', 'CLAUDE_CODE_USE_GATEWAY',
] as const;
const modelEnvKeys = [
  'ANTHROPIC_DEFAULT_HAIKU_MODEL', 'ANTHROPIC_DEFAULT_SONNET_MODEL',
  'ANTHROPIC_DEFAULT_OPUS_MODEL', 'ANTHROPIC_SMALL_FAST_MODEL',
] as const;
const providerEnvKeys = new Set<string>([
  'ANTHROPIC_BASE_URL', 'ANTHROPIC_AUTH_TOKEN', 'ANTHROPIC_API_KEY',
  ...modelEnvKeys, ...routeDisableKeys,
]);
const providerSchema = z.object({
  model: z.string().min(1).max(256).refine(value => value === value.trim() && !/[\s\x00-\x1f\x7f]/.test(value)),
  base_url: z.string().url().refine(value => {
    const url = new URL(value);
    return ['http:', 'https:'].includes(url.protocol) && !url.username && !url.password && !url.search && !url.hash;
  }),
  auth_style: z.enum(['bearer', 'x_api_key', 'both']),
  env: z.record(z.string(), z.string().refine(value => !/[\x00\r\n]/.test(value))),
}).strict().superRefine((provider, context) => {
  const env = provider.env;
  const bearer = provider.auth_style !== 'x_api_key';
  const apiKey = provider.auth_style !== 'bearer';
  if (Object.keys(env).some(key => !providerEnvKeys.has(key))
    || (env.ANTHROPIC_BASE_URL !== undefined && env.ANTHROPIC_BASE_URL !== provider.base_url)
    || (bearer ? !env.ANTHROPIC_AUTH_TOKEN?.trim() : !!env.ANTHROPIC_AUTH_TOKEN)
    || (apiKey ? !env.ANTHROPIC_API_KEY?.trim() : !!env.ANTHROPIC_API_KEY)
    || modelEnvKeys.some(key => env[key] !== undefined && env[key] !== provider.model)
    || routeDisableKeys.some(key => env[key] !== undefined && env[key] !== '' && env[key] !== '0' && env[key] !== 'false')) {
    context.addIssue({ code: 'custom', message: 'provider_unavailable' });
  }
});
type ProviderEvidence = {
  source: 'cli_control'; model: string; base_url: string;
  auth_style: z.infer<typeof providerSchema>['auth_style']; api_provider: 'firstParty';
  flag_settings_applied: true; route_env_verified: true; auth_helper_disabled: true;
};
const settingsSchema = z.object({
  effective: z.object({ env: z.record(z.string(), z.string()), apiKeyHelper: z.string().optional() }),
  sources: z.array(z.object({ source: z.string(), settings: z.object({ env: z.record(z.string(), z.string()).optional(), apiKeyHelper: z.string().optional() }) })),
  applied: z.object({ model: z.string() }),
});
class BridgeFailure extends Error {
  constructor(readonly kind: string) { super(kind); }
}
const responseUsage = z.object({ input_tokens: z.number().nonnegative().optional(), output_tokens: z.number().nonnegative().optional(), cache_read_input_tokens: z.number().nonnegative().nullable().optional(), cache_creation_input_tokens: z.number().nonnegative().nullable().optional() });
const modelLimits = z.record(z.string(), z.object({ contextWindow: z.number().positive().optional().catch(undefined), maxOutputTokens: z.number().positive().optional().catch(undefined) }));
const nativeMessage = z.object({ type: z.string(), subtype: z.string().optional(), session_id: z.string().optional(), user_message_uuid: z.string().optional(), user_message_uuids: z.array(z.string()).optional(), message: z.object({ content: z.unknown(), id: z.string().optional().catch(undefined), model: z.string().optional().catch(undefined), usage: responseUsage.optional().catch(undefined) }).optional(), event: z.object({ delta: z.object({ type: z.string(), text: z.string().optional() }).passthrough().optional() }).passthrough().optional(), uuid: z.string().optional(), parent_tool_use_id: z.string().nullable().optional(), is_error: z.boolean().optional(), result: z.string().optional(), total_cost_usd: z.number().optional(), usage: z.unknown().optional(), modelUsage: modelLimits.optional().catch(undefined), model: z.string().optional(), tools: z.array(z.unknown()).optional(), tool_name: z.string().optional(), tool_use_id: z.string().optional(), elapsed_time_seconds: z.number().optional() });
const resultTermination = z.object({ terminal_reason: z.string().optional() });
const nativeDetails = z.object({ tool_use_result: z.unknown().optional(), error: z.unknown().optional(), errors: z.array(z.string()).optional() });
const nativeApiError = z.object({ isApiErrorMessage: z.literal(true) });
const commandSchema = z.discriminatedUnion('method', [
  z.object({ id: z.number(), method: z.literal('open'), params: z.object({ workspace: z.string(), executable: z.string(), resume_id: z.string().nullable(), provider: providerSchema }).strict() }),
  z.object({ id: z.number(), method: z.literal('send'), params: z.object({ request_id: z.string().uuid(), text: z.string(), settings: turnSettingsSchema.optional() }).strict() }),
  z.object({ id: z.number(), method: z.literal('respond'), params: z.object({ request_id: z.string(), response: z.object({ behavior: z.enum(['allow', 'deny']), answers: z.record(z.string(), z.string()).optional() }) }) }),
  z.object({ id: z.number(), method: z.literal('native/observe'), params: z.object({ domain: z.enum(['skills', 'mcp']) }).strict() }),
  z.object({ id: z.number(), method: z.literal('native/action'), params: z.object({ action: z.discriminatedUnion('kind', [
    z.object({ kind: z.literal('reload_skills') }).strict(),
    z.object({ kind: z.literal('reload_plugins'), hold_on_cache_impact: z.boolean() }).strict(),
    z.object({ kind: z.literal('reconnect_mcp'), server_name: z.string().max(128) }).strict(),
    z.object({ kind: z.literal('toggle_mcp'), server_name: z.string().max(128), enabled: z.boolean() }).strict(),
  ]) }).strict() }),
  z.object({ id: z.number(), method: z.enum(['interrupt', 'close']), params: record }),
]);
type BridgeCommand = z.infer<typeof commandSchema>;
async function* input() {
  while (!closed) {
    if (!queue.length) {
      const { promise, resolve } = Promise.withResolvers<void>();
      wake = resolve;
      await promise;
    }
    wake = undefined;
    while (!closed && queue.length) yield queue.shift()!;
  }
}
function controlledEnv(provider: z.infer<typeof providerSchema>): Record<string, string> {
  const env = { ...provider.env, ANTHROPIC_BASE_URL: provider.base_url,
    ANTHROPIC_AUTH_TOKEN: provider.env.ANTHROPIC_AUTH_TOKEN ?? '',
    ANTHROPIC_API_KEY: provider.env.ANTHROPIC_API_KEY ?? '' };
  // Empty strings disable both boolean parsers and older raw-truthiness route selectors.
  for (const key of routeDisableKeys) Object.assign(env, { [key]: '' });
  for (const key of modelEnvKeys) Object.assign(env, { [key]: provider.model });
  return env;
}
async function verifyProvider(current: Query, provider: z.infer<typeof providerSchema>, env: Record<string, string>): Promise<ProviderEvidence> {
  // SDK 0.3.317 implements this private-stdio control but omits its public TS signature.
  // CLI get_settings returns effective, sources and the actual running applied.model.
  const controls = current as Query & { getSettings?: () => Promise<unknown> };
  if (typeof controls.getSettings !== 'function') throw new BridgeFailure('provider_verification_failed');
  const settings = settingsSchema.safeParse(await controls.getSettings());
  if (!settings.success) throw new BridgeFailure('provider_verification_failed');
  const { effective, sources, applied } = settings.data;
  const flag = sources.find(source => source.source === 'flagSettings')?.settings;
  if (effective.apiKeyHelper?.trim() || flag?.apiKeyHelper !== '') throw new BridgeFailure('provider_auth_helper_blocked');
  if (applied.model !== provider.model || Object.entries(env).some(([key, value]) => effective.env[key] !== value || flag?.env?.[key] !== value)) {
    throw new BridgeFailure('provider_verification_failed');
  }
  // reinitialize is a control-only request, not a prompt. accountInfo() is cached at
  // first initialize and cannot establish the backend after a flag-tier change.
  const initialized = await current.reinitialize();
  if (initialized.account.apiProvider !== 'firstParty') throw new BridgeFailure('provider_verification_failed');
  return { source: 'cli_control', model: applied.model, base_url: effective.env.ANTHROPIC_BASE_URL!,
    auth_style: provider.auth_style, api_provider: initialized.account.apiProvider,
    flag_settings_applied: true, route_env_verified: true, auth_helper_disabled: true };
}
async function observeTurnSettings(input: { effort?: { level: string } }) {
  if (turnControls && input.effort?.level && activeRequestId) {
    turnControls.reasoning.current = input.effort.level;
    turnControls.reasoning.source = 'claude/native_hook_effort';
    turnControls.reasoning.reason = null;
    emit('controls', { controls: turnControls, request_id: activeRequestId, session_id: sessionId });
  }
  return {};
}
function safeNativeDetails(value: unknown) {
  const parsed = nativeDetails.safeParse(value);
  if (!parsed.success) return {};
  const content = nativeMessage.safeParse(value);
  const blocks = content.success ? content.data.message?.content : undefined;
  const failed = parsed.data.error !== undefined || !!parsed.data.errors?.length
    || (Array.isArray(blocks) && blocks.some(block => z.object({ type: z.literal('tool_result'), is_error: z.literal(true) }).safeParse(block).success));
  return {
    ...(parsed.data.tool_use_result !== undefined ? { tool_use_result: failed ? 'engine_failed' : parsed.data.tool_use_result } : {}),
    ...(parsed.data.error !== undefined ? { error: 'engine_failed' } : {}),
    ...(parsed.data.errors?.length ? { errors: ['engine_failed'] } : {}),
  };
}
function project(value: unknown, history = false) {
  const parsed = nativeMessage.safeParse(value);
  if (!parsed.success) return;
  const message = parsed.data;
  if (closed) return;
  if (message.session_id && message.session_id !== sessionId) {
    if (history) throw new BridgeFailure('resume_identity_mismatch');
    emit('failure', { kind: 'session_identity_mismatch', message: 'session_identity_mismatch', request_id: activeRequestId });
    void close();
    return;
  }
  const echoed = message.user_message_uuids ?? (message.user_message_uuid ? [message.user_message_uuid] : []);
  const requestId = !history && activeRequestId && (echoed.includes(activeRequestId) || (!echoed.length && message.type !== 'result')) ? activeRequestId : undefined;
  const identity = { session_id: message.session_id ?? sessionId, uuid: message.uuid,
    user_message_uuid: message.user_message_uuid, user_message_uuids: message.user_message_uuids,
    request_id: requestId, history };
  if (nativeApiError.safeParse(value).success) {
    if (!history && echoed.length && !requestId) return;
    emit('failure', { kind: 'engine_failed', message: 'engine_failed', ...identity });
    return;
  }
  if (message.type === 'assistant' || message.type === 'user') {
    const key = message.uuid ? `${identity.session_id}:${message.uuid}` : undefined;
    if (key && projectedMessages.has(key)) return;
    if (key) projectedMessages.add(key);
    const details = safeNativeDetails(value);
    const content = 'error' in details || 'errors' in details || message.is_error ? 'engine_failed' : message.message?.content;
    const safeContent = Array.isArray(content) ? content.map(block => {
      const failed = z.object({ type: z.literal('tool_result'), is_error: z.literal(true), tool_use_id: z.string() }).safeParse(block);
      return failed.success ? { ...failed.data, content: 'engine_failed' } : block;
    }) : content;
    emit('message', { type: message.type, content: safeContent, parent_tool_use_id: message.parent_tool_use_id, model: message.message?.model, usage: message.type === 'assistant' ? message.message?.usage : undefined, native_message_id: message.message?.id, usage_source: message.type === 'assistant' && message.message?.usage ? 'native_response' : undefined, ...details, ...identity });
  } else if (message.type === 'stream_event') {
    const delta = message.event?.delta;
    if (delta?.type === 'text_delta') emit('text_delta', { text: delta.text, parent_tool_use_id: message.parent_tool_use_id, ...identity });
  } else if (message.type === 'result') {
    const key = message.uuid ? `${identity.session_id}:${message.uuid}` : undefined;
    if (key && projectedResults.has(key)) return;
    if (key) projectedResults.add(key);
    if (!requestId) {
      if (activeRequestId && !echoed.length) {
        emit('result', { ...identity, request_id: activeRequestId, terminal_reason: 'result_identity_unknown' });
        void close();
      }
      return;
    }
    running = false;
    activeRequestId = undefined;
    const termination = resultTermination.safeParse(value);
    emit('result', { subtype: message.subtype, is_error: message.is_error, result: message.is_error ? 'engine_failed' : message.result, total_cost_usd: message.total_cost_usd, usage: message.usage, model_limits: message.modelUsage, terminal_reason: termination.success ? termination.data.terminal_reason : undefined, ...safeNativeDetails(value), ...identity });
  } else if (message.type === 'system' && message.subtype === 'init') {
    // The requested session identity is pinned before query(); never adopt a fallback session.
    const metadata = initMetadata.safeParse(value);
    emit('native/initialized', { session_id: sessionId, model: message.model, tools: message.tools, ...(metadata.success ? metadata.data : {}), status: 'observed_not_executed' });
  } else if (message.type === 'tool_progress') {
    emit('tool_progress', { tool_name: message.tool_name, tool_use_id: message.tool_use_id, elapsed_time_seconds: message.elapsed_time_seconds, ...identity });
  }
}
async function close() {
  closed = true; opened = false; running = false; activeRequestId = undefined; queue.length = 0; wake?.();
  activeProvider = undefined; activeProviderEnv = undefined;
  for (const permission of permissions.values()) permission.resolve({ behavior: 'deny', message: 'Session closed' });
  permissions.clear(); agent?.close();
}
function safeIdentifier(value: string): string | null {
  return value.length > 0 && value.length <= 128 && /^[a-zA-Z0-9_.:@\-]+$/.test(value) ? value : null;
}
async function handle(command: BridgeCommand) {
  if (command.method === 'open') {
    const params = command.params;
    if (opening || agent || closed) throw new BridgeFailure('already_open');
    opening = true;
    const provider = params.provider;
    const providerEnv = controlledEnv(provider);
    try {
    sessionId = params.resume_id || sessionId;
    if (params.resume_id) {
      const history = await getSessionMessages(params.resume_id, { dir: params.workspace });
      // The pinned SDK returns [] for a missing transcript. Do not let query()
      // silently turn an unavailable history into a fresh running session.
      if (!history.length) throw new BridgeFailure('resume_history_unavailable');
      emit('history', { truncated: history.length > 2000 });
      for (const message of history.slice(-2000)) project(message, true);
    }
    if (closed) throw new BridgeFailure('closed');
    agent = query({ prompt: input(), options: {
      cwd: params.workspace,
      pathToClaudeCodeExecutable: params.executable,
      systemPrompt: { type: 'preset', preset: 'claude_code' },
      settingSources: ['user', 'project', 'local'],
      permissionMode: 'default',
      includePartialMessages: true,
      hooks: {
        PreToolUse: [{ hooks: [async input => observeTurnSettings(input)] }],
        Stop: [{ hooks: [async input => observeTurnSettings(input)] }],
      },
      model: provider.model,
      // Startup inherits the host's controlled env; this explicit overlay contains
      // no argv settings. The higher flag tier is applied over private stdio below.
      env: { ...process.env, ...providerEnv },
      stderr: () => {},
      ...(params.resume_id ? { resume: params.resume_id } : { sessionId }),
      canUseTool: async (tool, original, options) => {
        const id = options.requestId;
        if (!id || permissionRequestIds.has(id)) return { behavior: 'deny', message: 'Invalid permission request identity' };
        permissionRequestIds.add(id);
        const { promise, resolve } = Promise.withResolvers<PermissionResult>();
        let settled = false;
        const settle = (result: PermissionResult) => { if (settled) return; settled = true; permissions.delete(id); options.signal.removeEventListener('abort', abort); emit('permission_resolved', { id }); resolve(result); };
        const abort = () => settle({ behavior: 'deny', message: 'Request cancelled' });
        if (options.signal.aborted || closed) { abort(); return await promise; }
        permissions.set(id, { tool, input: original, resolve: settle });
        options.signal.addEventListener('abort', abort, { once: true });
        emit('permission', { id, tool, input: original, tool_use_id: options.toolUseID, agent_id: options.agentID, request_id: activeRequestId, session_id: sessionId });
        return await promise;
      },
    }});
    const current = agent;
    void (async () => {
      try { for await (const message of current) project(message); }
      catch { if (!closed) emit('failure', { kind: 'engine_failed', message: 'engine_failed', request_id: activeRequestId }); }
      finally { await close(); emit('closed', {}); }
    })();
    const initialized = await current.initializationResult();
    // This adapter is pinned to the installed SDK and actual responding CLI.
    nativeProtocolSupported = initialized.claude_code_version === '2.1.319';
    if (closed) throw new BridgeFailure('closed');
    // Null would remove the flag key and restore lower-tier helpers. Empty string
    // disables the helper; managed policy may win, in which case readback refuses it.
    await current.applyFlagSettings({ env: providerEnv, apiKeyHelper: '', model: provider.model });
    providerEvidence = await verifyProvider(current, provider, providerEnv);
    if (closed) throw new BridgeFailure('closed');
    activeProvider = provider; activeProviderEnv = providerEnv;
    turnControls = projectTurnControls(await current.supportedModels().catch(() => []), provider.model, nativeProtocolSupported);
    opened = true;
    emit('native/initialized', { commands: z.array(commandMetadata).parse(initialized.commands), status: 'observed_not_executed' });
    return { session_id: sessionId, provider_evidence: providerEvidence, native_protocol_supported: nativeProtocolSupported, controls: turnControls };
    } catch (error) {
      await close();
      throw error instanceof BridgeFailure ? error : new BridgeFailure('provider_load_failed');
    } finally { opening = false; }
  }
  if (command.method === 'close') { await close(); return {}; }
  if (!agent || closed) throw new BridgeFailure('closed');
  if (!opened) throw new BridgeFailure('not_ready');
  if (command.method === 'native/observe' || command.method === 'native/action') {
    if (!nativeProtocolSupported) throw new BridgeFailure('unsupported_operation');
    if (nativeControlBusy) throw new BridgeFailure('busy');
    nativeControlBusy = true;
    try {
      if (command.method === 'native/observe') {
        if (command.params.domain === 'skills') {
          const commands = await agent.supportedCommands();
          if (closed) throw new BridgeFailure('closed');
          return projectSupportedCommands(commands);
        }
        const servers = await agent.mcpServerStatus();
        if (closed) throw new BridgeFailure('closed');
        return { servers: servers.slice(0, 256).map(value => ({
          name: safeIdentifier(value.name), status: value.status,
          source: ['sdk','plugin','user','project','local','dynamic','managed','enterprise','claudeai','agent'].includes(value.source || '') ? value.source : undefined,
          scope: ['user','project','local','managed','claudeai'].includes(value.scope || '') ? value.scope : undefined,
          tools: value.status === 'connected' ? value.tools?.slice(0, 64).map(tool => ({ name: safeIdentifier(tool.name) })) : [],
        })), incomplete: servers.length > 256 || servers.some(value => (value.tools?.length || 0) > 64) };
      }
      if (running || permissions.size > 0) throw new BridgeFailure('busy');
      const action = command.params.action;
      if (action.kind === 'reload_skills') {
        const result = await agent.reloadSkills();
        if (closed) throw new BridgeFailure('closed');
        return { state: Array.isArray(result.skills) ? 'applied' : 'unknown' };
      }
      if (action.kind === 'reload_plugins') {
        const result = await agent.reloadPlugins({ holdOnCacheImpact: action.hold_on_cache_impact });
        if (closed) throw new BridgeFailure('closed');
        if (result.error_count !== 0) return { state: 'unknown' };
        // An older CLI may ignore holdOnCacheImpact; absence cannot prove held semantics.
        if (action.hold_on_cache_impact && typeof result.held !== 'boolean') return { state: 'unknown' };
        return { state: result.held ? 'held' : 'applied' };
      }
      if (safeIdentifier(action.server_name) !== action.server_name) throw new BridgeFailure('invalid_request');
      const servers = await agent.mcpServerStatus();
      if (closed) throw new BridgeFailure('closed');
      const matches = servers.filter(value => value.name === action.server_name);
      if (matches.length !== 1) throw new BridgeFailure('unsupported_operation');
      const server = matches[0];
      // Never infer loose ownership from a display name or mutate managed/plugin resources.
      if (!['user','project','local'].includes(server.source || '')) throw new BridgeFailure('unsupported_operation');
      if (action.kind === 'reconnect_mcp') await agent.reconnectMcpServer(action.server_name);
      else await agent.toggleMcpServer(action.server_name, action.enabled);
      if (closed) throw new BridgeFailure('closed');
      return { state: 'applied' };
    } finally { nativeControlBusy = false; }
  }
  if (command.method === 'send') {
    const params = command.params;
    // Match host sparse settings semantics before receipt identity or controls.
    if (params.settings && Object.keys(params.settings).length === 0) delete params.settings;
    if (typeof params.text !== 'string' || !params.text.trim() || Buffer.byteLength(params.text) > 65536) throw new BridgeFailure('invalid_prompt');
    const fingerprint = createHash('sha256').update(JSON.stringify([params.text, params.settings ?? null])).digest('hex');
    if (receipts.has(params.request_id)) {
      if (receipts.get(params.request_id) !== fingerprint) throw new BridgeFailure('request_conflict');
      return {};
    }
    if (running || nativeControlBusy) throw new BridgeFailure('busy');
    if (receipts.size >= 1024) throw new BridgeFailure('request_capacity');
    if (!activeProvider || !activeProviderEnv) throw new BridgeFailure('closed');
    if (params.settings) {
      if (!turnControls) throw new BridgeFailure('turn_controls_unavailable');
      try { validateTurnSettings(turnControls, params.settings); }
      catch (error) { throw new BridgeFailure(error instanceof Error ? error.message : 'invalid_turn_settings'); }
    }
    // No prompt reaches the SDK until fresh CLI readback accepts the pinned route.
    // Reserve the turn across the asynchronous control round trips.
    running = true;
    try { await verifyProvider(agent, activeProvider, activeProviderEnv); }
    catch { await close(); throw new BridgeFailure('provider_verification_failed'); }
    try {
      if (params.settings && turnControls) await applyTurnSettings(agent, turnControls, params.settings);
    } catch {
      if (turnControls) {
        turnControls.reasoning.current = null; turnControls.reasoning.reason = 'result_unknown';
        turnControls.permission.current = null; turnControls.permission.reason = 'result_unknown';
        emit('controls', { controls: turnControls, session_id: sessionId });
      }
      await close(); throw new BridgeFailure('turn_configuration_failed');
    }
    if (closed) throw new BridgeFailure('closed');
    receipts.set(params.request_id, fingerprint);
    activeRequestId = params.request_id;
    // commandSchema has already validated the host request ID as a UUID.
    queue.push({ type: 'user', message: { role: 'user', content: params.text }, parent_tool_use_id: null, session_id: sessionId, uuid: params.request_id as SDKUserMessage['uuid'], origin: { kind: 'human' } });
    if (turnControls) emit('controls', { controls: turnControls, request_id: activeRequestId, session_id: sessionId });
    wake?.(); return { controls: turnControls };
  }
  if (command.method === 'respond') {
    const params = command.params;
    const pending = permissions.get(params.request_id);
    if (!pending) throw new BridgeFailure('request_expired');
    const response = params.response;
    if (response?.behavior === 'deny') pending.resolve({ behavior: 'deny', message: 'Denied by user' });
    else if (response?.behavior === 'allow') {
      const updatedInput = { ...pending.input };
      if (response.answers) {
        if (pending.tool !== 'AskUserQuestion') throw new BridgeFailure('invalid_response');
        const questions = z.array(z.object({ question: z.string() }).passthrough()).safeParse(pending.input.questions);
        if (!questions.success || Object.keys(response.answers).some(key => !questions.data.some(question => question.question === key))) throw new BridgeFailure('invalid_response');
        updatedInput.answers = response.answers;
      }
      pending.resolve({ behavior: 'allow', updatedInput });
    } else throw new BridgeFailure('invalid_response');
    return {};
  }
  if (command.method === 'interrupt') { await agent.interrupt(); return {}; }
  throw new BridgeFailure('unsupported_operation');
}
const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
lines.on('line', line => {
  if (Buffer.byteLength(line) > 4 * 1024 * 1024) { void close(); return; }
  let value: unknown;
  try { value = JSON.parse(line); } catch { void close(); return; }
  const parsed = commandSchema.safeParse(value);
  if (!parsed.success) { void close(); return; }
  const message = parsed.data;
  void handle(message).then(result => {
    reply(message.id, result);
    if (message.method === 'open' && opened) emit('ready', { session_id: sessionId, provider_evidence: providerEvidence });
  }, error => {
    const kind = error instanceof BridgeFailure ? error.kind : 'operation_failed';
    if (message.method === 'open') emit('failure', { kind, message: kind });
    process.stdout.write(JSON.stringify({ id: message.id, error: { code: -32000, message: kind } }) + '\n');
  });
});
lines.on('close', () => { void close(); });
process.on('SIGTERM', () => { void close().finally(() => process.exit(0)); });
process.on('SIGINT', () => { void close().finally(() => process.exit(0)); });
