import { query, getSessionMessages, type SDKUserMessage, type PermissionResult, type Query } from '@anthropic-ai/claude-agent-sdk';
import { z } from 'zod';
import { createInterface } from 'node:readline';
import { randomUUID, createHash } from 'node:crypto';
import { projectSupportedCommands } from './native-observation';
import { commandsSchema, modelsSchema, mcpStatusSchema, nativeCapabilities, verifiedWrites, isUnsupportedControl, resultRequestId, validateQuestionAnswers, observeActiveSkillSources, type NativeCapabilities } from './native-protocol';
import { applyTurnSettings, projectTurnControls, turnSettingsSchema, validateTurnSettings, responseInstructions, deepPlanInstructions, nativePermission, type TurnControls, type TurnSettings } from './turn-controls';
import { NativeRunBindings, NativeTranscriptProjection, responseUsage, safeNativeDetails, safeUnknownEvent } from './native-transcript';

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
let optionalWritesVerified = false;
let nativeCapabilitiesState: NativeCapabilities | undefined;
let nativeControlBusy = false;
let wake: (() => void) | undefined;
let turnControls: TurnControls | undefined;
let activeRequestId: string | undefined;
let activeLocalCommand: string | undefined;
let queryEpoch = 0;
let queryConfig: { workspace: string; executable: string } | undefined;
let queryHasHistory = false;
let queryFullAuto = false;
let queryPreferences: TurnSettings['response_preferences'];
let queryCollaboration: TurnSettings['collaboration_mode'] = 'default';
let queryDrain: Promise<void> | undefined;
const projectedMessages = new Set<string>();
const projectedResults = new Set<string>();
const transcript = new NativeTranscriptProjection();
const taskRequests = new NativeRunBindings();
const queue: SDKUserMessage[] = [];
const receipts = new Map<string, string>();
const permissionRequestIds = new Set<string>();
const permissions = new Map<string, { tool: string; input: Record<string, unknown>; resolve: (result: PermissionResult) => void }>();
const record = z.record(z.string(), z.unknown());
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
const modelLimits = z.record(z.string(), z.object({ contextWindow: z.number().positive().optional().catch(undefined), maxOutputTokens: z.number().positive().optional().catch(undefined), inputTokens: z.number().nonnegative().optional(), outputTokens: z.number().nonnegative().optional(), cacheReadInputTokens: z.number().nonnegative().optional(), cacheCreationInputTokens: z.number().nonnegative().optional(), costUSD: z.number().nonnegative().optional(), costBasis: z.enum(['list', 'managed', 'unknown']).optional(), canonicalModel: z.string().optional(), provider: z.string().optional() }));
const nativeMessage = z.object({ type: z.string(), subtype: z.string().optional(), session_id: z.string().optional(), user_message_uuid: z.string().optional(), user_message_uuids: z.array(z.string()).optional(), message: z.object({ content: z.unknown(), id: z.string().optional().catch(undefined), model: z.string().optional().catch(undefined), usage: responseUsage.optional().catch(undefined) }).optional(), event: z.object({ delta: z.object({ type: z.string(), text: z.string().optional() }).passthrough().optional() }).passthrough().optional(), uuid: z.string().optional(), parent_tool_use_id: z.string().nullable().optional(), is_error: z.boolean().optional(), result: z.string().optional(), total_cost_usd: z.number().nonnegative().optional(), duration_ms: z.number().nonnegative().optional(), duration_api_ms: z.number().nonnegative().optional(), usage: z.unknown().optional(), modelUsage: modelLimits.optional().catch(undefined), model: z.string().optional(), tools: z.array(z.unknown()).optional(), tool_name: z.string().optional(), tool_use_id: z.string().optional(), elapsed_time_seconds: z.number().optional() });
const resultTermination = z.object({ terminal_reason: z.string().optional() });
const nativeApiError = z.object({ isApiErrorMessage: z.literal(true) });
const commandSchema = z.discriminatedUnion('method', [
  z.object({ id: z.number(), method: z.literal('open'), params: z.object({ workspace: z.string(), executable: z.string(), resume_id: z.string().nullable(), provider: providerSchema }).strict() }),
  z.object({ id: z.number(), method: z.literal('send'), params: z.object({ request_id: z.string().uuid(), text: z.string(), settings: turnSettingsSchema.optional() }).strict() }),
  z.object({ id: z.number(), method: z.literal('compression/observe'), params: z.object({}).strict() }),
  z.object({ id: z.number(), method: z.literal('respond'), params: z.object({ request_id: z.string(), response: z.object({ behavior: z.enum(['allow', 'deny']), answers: z.record(z.string(), z.string()).optional() }) }) }),
  z.object({ id: z.number(), method: z.literal('native/observe'), params: z.object({ domain: z.enum(['skills', 'mcp']) }).strict() }),
  z.object({ id: z.number(), method: z.literal('native/action'), params: z.object({ action: z.discriminatedUnion('kind', [
    z.object({ kind: z.literal('reload_skills') }).strict(),
    z.object({ kind: z.literal('reload_plugins'), hold_on_cache_impact: z.boolean() }).strict(),
    z.object({ kind: z.literal('reconnect_mcp'), server_name: z.string().max(128) }).strict(),
    z.object({ kind: z.literal('toggle_mcp'), server_name: z.string().max(128), enabled: z.boolean() }).strict(),
  ]) }).strict() }),
  z.object({ id: z.number(), method: z.literal('interrupt'), params: z.object({ request_id: z.string().uuid() }).strict() }),
  z.object({ id: z.number(), method: z.literal('close'), params: record }),
]);
type BridgeCommand = z.infer<typeof commandSchema>;
async function* input(epoch: number) {
  while (!closed && epoch === queryEpoch) {
    if (!queue.length) {
      const { promise, resolve } = Promise.withResolvers<void>();
      wake = resolve;
      await promise;
    }
    wake = undefined;
    while (!closed && epoch === queryEpoch && queue.length) yield queue.shift()!;
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
  if (typeof controls.getSettings !== 'function' || typeof current.reinitialize !== 'function') throw new BridgeFailure('provider_verification_failed');
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
function project(value: unknown, history = false) {
  const parsed = nativeMessage.safeParse(value);
  if (!parsed.success) {
    const summary = safeUnknownEvent(value);
    if (!closed && summary) emit('native/unsupported', { ...summary, session_id: sessionId, history });
    return;
  }
  const message = parsed.data;
  if (closed) return;
  if (message.session_id && message.session_id !== sessionId) {
    if (history) throw new BridgeFailure('resume_identity_mismatch');
    emit('failure', { kind: 'session_identity_mismatch', message: 'session_identity_mismatch', request_id: activeRequestId });
    void close();
    return;
  }
  const echoed = [...(message.user_message_uuids ?? []), ...(message.user_message_uuid ? [message.user_message_uuid] : [])];
  const local = z.object({ local_command: z.string().optional() }).safeParse(value);
  let requestId = history ? undefined : resultRequestId({ ...message, local_command: local.success ? local.data.local_command : undefined }, activeRequestId, activeLocalCommand, optionalWritesVerified);
  if (!history && message.type !== 'result') requestId = taskRequests.resolve(value, requestId);
  const identity = { session_id: message.session_id ?? sessionId, uuid: message.uuid,
    user_message_uuid: message.user_message_uuid, user_message_uuids: message.user_message_uuids,
    request_id: requestId, history };
  if (nativeApiError.safeParse(value).success && !message.parent_tool_use_id) {
    if (!history && echoed.length && !requestId) return;
    emit('failure', { kind: 'engine_failed', message: 'engine_failed', ...identity });
    transcript.clear();
    return;
  }
  if (message.type === 'assistant' || message.type === 'user' || message.type === 'stream_event' || message.type === 'tool_progress'
    || message.type === 'system' && ['task_started', 'task_progress', 'task_notification'].includes(message.subtype ?? '')) {
    for (const event of transcript.project(value, history)) {
      // resolve() records tool_use IDs from both block starts and full frames.
      emit(event.kind, { ...event.data, ...(event.kind === 'native/unsupported' ? { session_id: sessionId, request_id: requestId, history } : identity) });
    }
  } else if (message.type === 'system' && message.subtype === 'local_command_output') {
    const output = z.object({ content: z.string() }).safeParse(value);
    const key = message.uuid ? `${identity.session_id}:${message.uuid}` : undefined;
    if (!output.success || key && projectedMessages.has(key)) return;
    if (key) projectedMessages.add(key);
    emit('message', { type: 'assistant', content: output.data.content, native_subtype: 'local_command_output', ...identity });
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
    transcript.clearMain();
    for (const permission of permissions.values()) permission.resolve({ behavior: 'deny', message: 'Turn completed' });
    activeRequestId = undefined;
    activeLocalCommand = undefined;
    const termination = resultTermination.safeParse(value);
    const details = safeNativeDetails(value);
    const usage = responseUsage.safeParse(message.usage);
    emit('result', { subtype: message.subtype, is_error: message.is_error, result: message.is_error || 'error' in details || 'errors' in details ? 'engine_failed' : message.result, local_command: local.success ? local.data.local_command : undefined, total_cost_usd: message.total_cost_usd, cost_kind: message.total_cost_usd === undefined ? undefined : 'estimated', duration_ms: message.duration_ms, duration_api_ms: message.duration_api_ms, usage: usage.success ? usage.data : undefined, usage_source: usage.success ? 'native_result' : undefined, usage_final: true, model_limits: message.modelUsage, model_usage: message.modelUsage, terminal_reason: termination.success ? termination.data.terminal_reason : undefined, ...details, ...identity });
  } else if (message.type === 'system' && message.subtype === 'init') {
    // The requested session identity is pinned before query(); never adopt a fallback session.
    const metadata = initMetadata.safeParse(value);
    emit('native/initialized', { session_id: sessionId, model: message.model, tools: message.tools, ...(metadata.success ? metadata.data : {}), status: 'observed_not_executed' });
  } else {
    const summary = safeUnknownEvent(value);
    if (summary) emit('native/unsupported', { ...summary, session_id: sessionId, request_id: requestId, history });
  }
}
async function close() {
  closed = true; opened = false; running = false; activeRequestId = undefined; queue.length = 0; wake?.();
  activeLocalCommand = undefined;
  transcript.clear(); taskRequests.clear();
  activeProvider = undefined; activeProviderEnv = undefined;
  for (const permission of permissions.values()) permission.resolve({ behavior: 'deny', message: 'Session closed' });
  permissions.clear(); agent?.close();
}
function safeIdentifier(value: string): string | null {
  return value.length > 0 && value.length <= 128 && /^[a-zA-Z0-9_.:@\-]+$/.test(value) ? value : null;
}
async function startNative(provider: z.infer<typeof providerSchema>, providerEnv: Record<string, string>, resume: boolean, settings: TurnSettings = {}) {
  if (!queryConfig) throw new BridgeFailure('closed');
  const epoch = ++queryEpoch;
  const previous = agent;
  wake?.(); previous?.close();
  const draining = queryDrain;
  await draining;
  if (closed) throw new BridgeFailure('closed');
  transcript.clear();
  queryFullAuto = (settings.permission_mode ?? turnControls?.permission.current) === 'full_auto';
  queryPreferences = settings.response_preferences ?? queryPreferences;
  queryCollaboration = settings.collaboration_mode ?? queryCollaboration;
  const current = query({ prompt: input(epoch), options: {
    cwd: queryConfig.workspace,
    pathToClaudeCodeExecutable: queryConfig.executable,
    systemPrompt: { type: 'preset', preset: 'claude_code', append: responseInstructions(queryPreferences), snapshot: false },
    planModeInstructions: queryCollaboration === 'deep' ? deepPlanInstructions : undefined,
    settingSources: ['user', 'project', 'local'],
    permissionMode: turnControls ? nativePermission(turnControls, settings) : 'default',
    ...(queryFullAuto ? { allowDangerouslySkipPermissions: true } : {}),
    includePartialMessages: true,
    forwardSubagentText: true,
    hooks: {
      PreToolUse: [{ hooks: [async input => observeTurnSettings(input)] }],
      Stop: [{ hooks: [async input => observeTurnSettings(input)] }],
    },
    model: provider.model,
    env: { ...process.env, ...providerEnv },
    stderr: () => {},
    ...(resume ? { resume: sessionId } : { sessionId }),
    canUseTool: async (tool, original, options) => {
      const id = options.requestId;
      if (!id || permissionRequestIds.has(id) || !running || !activeRequestId || epoch !== queryEpoch) return { behavior: 'deny', message: 'Invalid permission request identity' };
      if ((queryCollaboration === 'plan' || queryCollaboration === 'deep') && !['AskUserQuestion', 'ExitPlanMode'].includes(tool)) return { behavior: 'deny', message: 'Planning mode does not authorize side effects. Select an execution mode first.' };
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
  agent = current;
  queryDrain = (async () => {
    try { for await (const message of current) if (epoch === queryEpoch) project(message); }
    catch { if (!closed && epoch === queryEpoch) emit('failure', { kind: 'engine_failed', message: 'engine_failed', request_id: activeRequestId }); }
    finally { if (epoch === queryEpoch) { await close(); emit('closed', {}); } }
  })();
  void queryDrain.catch(() => {});
  const initialized = await current.initializationResult();
  await current.applyFlagSettings({ env: providerEnv, apiKeyHelper: '', model: provider.model });
  providerEvidence = await verifyProvider(current, provider, providerEnv);
  return initialized;
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
      // SDK 0.3.317 parses the complete local transcript before limit/offset;
      // retain the latest-window semantics rather than page/reparse it repeatedly.
      const history = await getSessionMessages(params.resume_id, { dir: params.workspace, includeSystemMessages: true });
      if (!history.length) throw new BridgeFailure('resume_history_unavailable');
      emit('history', { truncated: history.length > 2000 });
      for (const message of history.slice(-2000)) project(message, true);
    }
    if (closed) throw new BridgeFailure('closed');
    queryConfig = { workspace: params.workspace, executable: params.executable };
    queryHasHistory = !!params.resume_id;
    const initialized = await startNative(provider, providerEnv, queryHasHistory);
    const current = agent!;
    optionalWritesVerified = verifiedWrites(initialized.claude_code_version);
    nativeCapabilitiesState = nativeCapabilities(current, initialized.claude_code_version);
    if (closed) throw new BridgeFailure('closed');
    activeProvider = provider; activeProviderEnv = providerEnv;
    // Optional metadata must not invalidate a Provider-verified ordinary query.
    const commands = commandsSchema.safeParse(initialized.commands);
    if (!commands.success) emit('native/unavailable', { domain: 'skills', kind: 'native_invalid_format' });
    if (nativeCapabilitiesState.observe_mcp) {
      try {
        if (!mcpStatusSchema.safeParse(await current.mcpServerStatus()).success) emit('native/unavailable', { domain: 'mcp', kind: 'native_invalid_format' });
      } catch (error) {
        if (isUnsupportedControl(error, 'mcp_status')) nativeCapabilitiesState.observe_mcp = false;
        emit('native/unavailable', { domain: 'mcp', kind: isUnsupportedControl(error, 'mcp_status') ? 'unsupported_operation' : 'operation_failed' });
      }
    }
    const models = modelsSchema.safeParse(initialized.models);
    turnControls = projectTurnControls(models.success ? models.data : [], provider.model, optionalWritesVerified);
    if (typeof current.setPermissionMode !== 'function') {
      turnControls.permission = { ...turnControls.permission, available: false, options: [], current: null, reason: 'native_protocol_unsupported' };
      turnControls.collaboration = { ...turnControls.collaboration, available: false, options: [], current: null, reason: 'native_protocol_unsupported' };
    }
    opened = true;
    if (commands.success) emit('native/initialized', { commands: commands.data, status: 'observed_not_executed' });
    return { session_id: sessionId, provider_evidence: providerEvidence, native_capabilities: nativeCapabilitiesState, verified_write_protocol: optionalWritesVerified, controls: turnControls };
    } catch (error) {
      await close();
      throw error instanceof BridgeFailure ? error : new BridgeFailure('provider_load_failed');
    } finally { opening = false; }
  }
  if (command.method === 'close') { await close(); return {}; }
  if (!agent || closed) throw new BridgeFailure('closed');
  if (!opened) throw new BridgeFailure('not_ready');
  if (command.method === 'compression/observe') {
    if (!optionalWritesVerified || typeof agent.supportedCommands !== 'function') return { supported: false };
    const commands = commandsSchema.safeParse(await agent.supportedCommands());
    const matches = commands.success ? commands.data.filter(command => command.name === 'compact') : [];
    return { supported: matches.length === 1 && matches[0]!.builtin === true };
  }
  if (command.method === 'native/observe' || command.method === 'native/action') {
    if (!nativeCapabilitiesState) throw new BridgeFailure('unsupported_operation');
    if (nativeControlBusy) throw new BridgeFailure('busy');
    nativeControlBusy = true;
    let attemptedCapability: keyof NativeCapabilities = command.method === 'native/observe' ? command.params.domain === 'skills' ? 'observe_skills' : 'observe_mcp' : command.params.action.kind;
    try {
      if (command.method === 'native/observe') {
        if (command.params.domain === 'skills') {
          if (!nativeCapabilitiesState.observe_skills) throw new BridgeFailure('unsupported_operation');
          const commands = commandsSchema.safeParse(await agent.supportedCommands());
          if (closed) throw new BridgeFailure('closed');
          if (!commands.success) throw new BridgeFailure('native_invalid_format');
          const observation = await observeActiveSkillSources(agent, commands.data, projectSupportedCommands(commands.data));
          if (closed) throw new BridgeFailure('closed');
          return observation;
        }
        if (!nativeCapabilitiesState.observe_mcp) throw new BridgeFailure('unsupported_operation');
        const parsed = mcpStatusSchema.safeParse(await agent.mcpServerStatus());
        if (!parsed.success) throw new BridgeFailure('native_invalid_format');
        const servers = parsed.data;
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
        if (!nativeCapabilitiesState.reload_skills) throw new BridgeFailure('unsupported_operation');
        if (!nativeCapabilitiesState.observe_skills) throw new BridgeFailure('unsupported_operation');
        attemptedCapability = 'observe_skills';
        if (!commandsSchema.safeParse(await agent.supportedCommands()).success) throw new BridgeFailure('native_invalid_format');
        attemptedCapability = 'reload_skills';
        const result = await agent.reloadSkills();
        if (closed) throw new BridgeFailure('closed');
        return { state: Array.isArray(result.skills) ? 'applied' : 'unknown' };
      }
      if (action.kind === 'reload_plugins') {
        if (!nativeCapabilitiesState.reload_plugins) throw new BridgeFailure('unsupported_operation');
        if (!nativeCapabilitiesState.observe_skills) throw new BridgeFailure('unsupported_operation');
        attemptedCapability = 'observe_skills';
        if (!commandsSchema.safeParse(await agent.supportedCommands()).success) throw new BridgeFailure('native_invalid_format');
        attemptedCapability = 'reload_plugins';
        const result = await agent.reloadPlugins({ holdOnCacheImpact: action.hold_on_cache_impact });
        if (closed) throw new BridgeFailure('closed');
        if (result.error_count !== 0) return { state: 'unknown' };
        // An older CLI may ignore holdOnCacheImpact; absence cannot prove held semantics.
        if (action.hold_on_cache_impact && typeof result.held !== 'boolean') return { state: 'unknown' };
        return { state: result.held ? 'held' : 'applied' };
      }
      if (safeIdentifier(action.server_name) !== action.server_name) throw new BridgeFailure('invalid_request');
      if (!nativeCapabilitiesState[action.kind] || !nativeCapabilitiesState.observe_mcp) throw new BridgeFailure('unsupported_operation');
      attemptedCapability = 'observe_mcp';
      const parsed = mcpStatusSchema.safeParse(await agent.mcpServerStatus());
      if (!parsed.success) throw new BridgeFailure('native_invalid_format');
      const servers = parsed.data;
      if (closed) throw new BridgeFailure('closed');
      const matches = servers.filter(value => value.name === action.server_name);
      if (matches.length !== 1) throw new BridgeFailure('unsupported_operation');
      const server = matches[0];
      // Never infer loose ownership from a display name or mutate managed/plugin resources.
      if (!['user','project','local'].includes(server.source || '')) throw new BridgeFailure('unsupported_operation');
      attemptedCapability = action.kind;
      if (action.kind === 'reconnect_mcp') await agent.reconnectMcpServer(action.server_name);
      else await agent.toggleMcpServer(action.server_name, action.enabled);
      if (closed) throw new BridgeFailure('closed');
      return { state: 'applied' };
    } catch (error) {
      const capability = attemptedCapability;
      const subtype = { observe_skills: 'initialize', observe_mcp: 'mcp_status', reload_skills: 'reload_skills', reload_plugins: 'reload_plugins', reconnect_mcp: 'mcp_reconnect', toggle_mcp: 'mcp_toggle' }[capability];
      if (isUnsupportedControl(error, subtype)) {
        nativeCapabilitiesState[capability] = false;
        if (capability === 'observe_skills') nativeCapabilitiesState.reload_skills = false;
        emit('native/capabilities', nativeCapabilitiesState);
        throw new BridgeFailure('unsupported_operation');
      }
      throw error;
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
    if (running || nativeControlBusy || permissions.size > 0) throw new BridgeFailure('busy');
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
      if (params.settings && turnControls) {
        const settings = params.settings;
        const fullAuto = (settings.permission_mode ?? turnControls.permission.current) === 'full_auto';
        const collaboration = settings.collaboration_mode ?? queryCollaboration;
        const preferences = settings.response_preferences ?? queryPreferences;
        if (fullAuto !== queryFullAuto || collaboration !== queryCollaboration || JSON.stringify(preferences) !== JSON.stringify(queryPreferences)) {
          // Startup-only SDK bypass flag is armed only after an explicit session selection.
          // Restart the identical native session before submitting any prompt; never retry the prompt.
          await startNative(activeProvider!, activeProviderEnv!, queryHasHistory, settings);
          // A restarted process must retain the selected effort unless this send explicitly resets it.
          if (settings.reasoning_effort === undefined && turnControls.reasoning.current) {
            await agent!.applyFlagSettings({ effortLevel: turnControls.reasoning.current as Parameters<Query['applyFlagSettings']>[0]['effortLevel'] });
          }
        }
        await applyTurnSettings(agent!, turnControls, settings);
      }
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
    queryHasHistory = true;
    activeRequestId = params.request_id;
    transcript.clearMain();
    const commandName = /^\/([^\s]+)(?:\s|$)/.exec(params.text.trim())?.[1];
    // CLI 2.1.319 hXi() sanitizes result.local_command in its !shouldQuery branch.
    activeLocalCommand = commandName?.toLowerCase().replace(/[^a-z]+/g, '_').replace(/^_+|_+$/g, '').slice(0, 64).replace(/_+$/, '') || (commandName ? 'custom' : undefined);
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
      if ((queryCollaboration === 'plan' || queryCollaboration === 'deep') && !['AskUserQuestion', 'ExitPlanMode'].includes(pending.tool)) throw new BridgeFailure('invalid_response');
      if (pending.tool === 'ExitPlanMode' && turnControls) {
        // This approval is the explicit plan-execution transition, never approval of other queued tools.
        await agent.setPermissionMode(nativePermission(turnControls, { collaboration_mode: 'default' }));
        turnControls.collaboration.current = 'default';
        queryCollaboration = 'default';
        emit('controls', { controls: turnControls, request_id: activeRequestId, session_id: sessionId });
      }
      const updatedInput = { ...pending.input };
      if (pending.tool === 'AskUserQuestion') {
        if (!validateQuestionAnswers(pending.input, response.answers)) throw new BridgeFailure('invalid_response');
        updatedInput.answers = response.answers;
      } else if (response.answers) throw new BridgeFailure('invalid_response');
      pending.resolve({ behavior: 'allow', updatedInput });
    } else throw new BridgeFailure('invalid_response');
    return {};
  }
  if (command.method === 'interrupt') {
    if (!running || activeRequestId !== command.params.request_id) throw new BridgeFailure('not_running');
    await agent.interrupt();
    transcript.clear();
    return {};
  }
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
    process.stdout.write(JSON.stringify({ id: message.id, error: { code: kind === 'unsupported_operation' ? -32601 : -32000, message: kind } }) + '\n');
  });
});
lines.on('close', () => { void close(); });
process.on('SIGTERM', () => { void close().finally(() => process.exit(0)); });
process.on('SIGINT', () => { void close().finally(() => process.exit(0)); });
