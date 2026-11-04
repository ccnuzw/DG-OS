import { query, getSessionMessages, type SDKUserMessage, type PermissionResult, type Query } from '@anthropic-ai/claude-agent-sdk';
import { z } from 'zod';
import { createInterface } from 'node:readline';
import { randomUUID, createHash } from 'node:crypto';
import { readFile, readdir } from 'node:fs/promises';
import { join } from 'node:path';

const emit = (method: string, params: unknown) => process.stdout.write(JSON.stringify({ method, params }) + '\n');
const reply = (id: unknown, result: unknown) => process.stdout.write(JSON.stringify({ id, result }) + '\n');
let agent: Query | undefined;
let sessionId: string = randomUUID();
let running = false;
let closed = false;
let wake: (() => void) | undefined;
const queue: SDKUserMessage[] = [];
const receipts = new Map<string, string>();
const permissions = new Map<string, { tool: string; input: Record<string, unknown>; resolve: (result: PermissionResult) => void }>();
const record = z.record(z.string(), z.unknown());
const commandMetadata = z.object({ name: z.string(), description: z.string().optional(), argumentHint: z.string().optional() });
const initMetadata = z.object({ slash_commands: z.array(z.string()).optional(), skills: z.array(z.string()).optional(), mcp_servers: z.array(z.object({ name: z.string(), status: z.string(), source: z.string().optional() })).optional(), plugins: z.array(z.object({ name: z.string(), path: z.string(), version: z.string().optional() })).optional() });
// This schema is only accepted on the Rust host's private stdio pipe; never echo it.
const capabilitiesSchema = z.object({ managed: z.boolean(), plugin_path: z.string().min(1).nullable().optional(), mcp_url: z.string().url().nullable().optional(), mcp_bearer: z.string().min(1).nullable().optional(), provider_env_keys: z.array(z.string()).optional() }).superRefine((value, context) => {
  if (value.managed && (!value.mcp_url || !value.mcp_bearer)) context.addIssue({ code: 'custom', message: 'mcp_unavailable' });
});
const nativeMessage = z.object({ type: z.string(), subtype: z.string().optional(), session_id: z.string().optional(), message: z.object({ content: z.unknown() }).optional(), event: z.object({ delta: z.object({ type: z.string(), text: z.string().optional() }).passthrough().optional() }).passthrough().optional(), uuid: z.string().optional(), parent_tool_use_id: z.string().nullable().optional(), is_error: z.boolean().optional(), result: z.string().optional(), total_cost_usd: z.number().optional(), usage: z.unknown().optional(), model: z.string().optional(), tools: z.array(z.unknown()).optional(), tool_name: z.string().optional(), tool_use_id: z.string().optional(), elapsed_time_seconds: z.number().optional() });
const resultTermination = z.object({ terminal_reason: z.string().optional() });
const nativeDetails = z.object({ tool_use_result: z.unknown().optional(), error: z.unknown().optional(), errors: z.array(z.string()).optional() });
const commandSchema = z.discriminatedUnion('method', [
  z.object({ id: z.number(), method: z.literal('open'), params: z.object({ workspace: z.string(), executable: z.string(), resume_id: z.string().nullable(), model: z.string().nullable(), capabilities: capabilitiesSchema.optional() }) }),
  z.object({ id: z.number(), method: z.literal('send'), params: z.object({ request_id: z.string().uuid(), text: z.string() }) }),
  z.object({ id: z.number(), method: z.literal('respond'), params: z.object({ request_id: z.string(), response: z.object({ behavior: z.enum(['allow', 'deny']), answers: z.record(z.string(), z.string()).optional() }) }) }),
  z.object({ id: z.number(), method: z.enum(['interrupt', 'close']), params: record }),
]);
type BridgeCommand = z.infer<typeof commandSchema>;
async function* input() {
  while (!closed) {
    if (!queue.length) await new Promise<void>(resolve => { wake = resolve; });
    while (queue.length) yield queue.shift()!;
  }
}
function project(value: unknown) {
  const parsed = nativeMessage.safeParse(value);
  if (!parsed.success) return;
  const message = parsed.data;
  if (message.type === 'assistant' || message.type === 'user') {
    const content = message.message?.content;
    const details = nativeDetails.safeParse(value);
    emit('message', { type: message.type, content, uuid: message.uuid, parent_tool_use_id: message.parent_tool_use_id, ...(details.success ? details.data : {}) });
  } else if (message.type === 'stream_event') {
    const delta = message.event?.delta;
    if (delta?.type === 'text_delta') emit('text_delta', { text: delta.text });
  } else if (message.type === 'result') {
    running = false;
    const termination = resultTermination.safeParse(value);
    const details = nativeDetails.safeParse(value);
    emit('result', { subtype: message.subtype, is_error: message.is_error, result: message.result, total_cost_usd: message.total_cost_usd, usage: message.usage, terminal_reason: termination.success ? termination.data.terminal_reason : undefined, ...(details.success ? details.data : {}) });
  } else if (message.type === 'system' && message.subtype === 'init') {
    if (message.session_id) sessionId = message.session_id;
    const metadata = initMetadata.safeParse(value);
    emit('ready', { session_id: sessionId, model: message.model, tools: message.tools, ...(metadata.success ? metadata.data : {}) });
  } else if (message.type === 'tool_progress') {
    emit('tool_progress', { tool_name: message.tool_name, tool_use_id: message.tool_use_id, elapsed_time_seconds: message.elapsed_time_seconds });
  }
}
async function close() {
  closed = true; wake?.();
  for (const permission of permissions.values()) permission.resolve({ behavior: 'deny', message: 'Session closed' });
  permissions.clear(); agent?.close();
}
async function handle(command: BridgeCommand) {
  if (command.method === 'open') {
    const params = command.params;
    if (agent) throw Error('already_open');
    const capabilities = params.capabilities;
    // Snapshot only host-selected keys; native settings still supply all unrelated values.
    const providerEnv: Record<string, string> = {};
    for (const key of capabilities?.provider_env_keys ?? []) {
      if (!key.startsWith('ANTHROPIC_') && !key.startsWith('CLAUDE_CODE_USE_')) throw Error('provider_unavailable');
      const value = process.env[key];
      if (value === undefined) throw Error('provider_unavailable');
      providerEnv[key] = value;
    }
    let selectedSkills: string[] = [];
    if (capabilities?.plugin_path) {
      try {
        const manifest = z.object({ name: z.string().min(1) }).parse(JSON.parse(await readFile(join(capabilities.plugin_path, '.claude-plugin', 'plugin.json'), 'utf8')));
        const directories = await readdir(join(capabilities.plugin_path, 'skills'), { withFileTypes: true });
        for (const directory of directories) {
          if (!directory.isDirectory()) throw Error('skill_load_failed');
          // Match the host package validator's supported scalar frontmatter format.
          const lines = (await readFile(join(capabilities.plugin_path, 'skills', directory.name, 'SKILL.md'), 'utf8')).split(/\r?\n/);
          if (lines[0]?.replace(/^\uFEFF/, '') !== '---') throw Error('skill_load_failed');
          let name: string | undefined;
          let closed = false;
          for (const line of lines.slice(1)) {
            if (line === '---') { closed = true; break; }
            const colon = line.indexOf(':');
            if (colon < 0 || line.slice(0, colon).trim() !== 'name') continue;
            if (name !== undefined) throw Error('skill_load_failed');
            name = line.slice(colon + 1).trim().replace(/^['"]+|['"]+$/g, '');
          }
          if (!closed || !name?.trim()) throw Error('skill_load_failed');
          selectedSkills.push(`${manifest.name}:${name}`);
        }
        if (!selectedSkills.length) throw Error('skill_load_failed');
      } catch { throw Error('skill_load_failed'); }
    }
    sessionId = params.resume_id || sessionId;
    if (params.resume_id) {
      const history = await getSessionMessages(params.resume_id, { dir: params.workspace });
      for (const message of history.slice(-2000)) project(message);
    }
    agent = query({ prompt: input(), options: {
      cwd: params.workspace,
      pathToClaudeCodeExecutable: params.executable,
      systemPrompt: { type: 'preset', preset: 'claude_code' },
      settingSources: ['user', 'project', 'local'],
      permissionMode: 'default',
      includePartialMessages: true,
      ...(capabilities?.plugin_path ? { plugins: [{ type: 'local' as const, path: capabilities.plugin_path, skipMcpDiscovery: true }] } : {}),
      ...(capabilities?.managed ? { strictMcpConfig: true } : {}),
      ...(params.model ? { model: params.model } : {}),
      ...(params.resume_id ? { resume: params.resume_id } : { sessionId }),
      canUseTool: async (tool, original, options) => {
        const id = options.requestId || randomUUID();
        return await new Promise<PermissionResult>(resolve => {
          let settled = false;
          const settle = (result: PermissionResult) => { if (settled) return; settled = true; permissions.delete(id); options.signal.removeEventListener('abort', abort); emit('permission_resolved', { id }); resolve(result); };
          const abort = () => settle({ behavior: 'deny', message: 'Request cancelled' });
          if (options.signal.aborted) { abort(); return; }
          permissions.get(id)?.resolve({ behavior: 'deny', message: 'Request replaced' });
          permissions.set(id, { tool, input: original, resolve: settle });
          options.signal.addEventListener('abort', abort, { once: true });
          emit('permission', { id, tool, input: original });
        });
      },
    }});
    void (async () => {
      try { for await (const message of agent!) project(message); }
      catch (error) { emit('failure', { kind: 'engine_failed', message: capabilities?.managed ? 'engine_failed' : error instanceof Error ? error.message : String(error) }); }
      finally { running = false; emit('closed', {}); }
    })();
    try {
      const initialized = await agent.initializationResult();
      if (selectedSkills.some(name => !initialized.commands.some(command => command.name === name))) throw Error('skill_load_failed');
      if (Object.keys(providerEnv).length) {
        // settings.env normally overwrites inherited env. Apply its flag-tier overlay via
        // private control stdio, rather than options.settings (which puts secrets in argv).
        try { await agent.applyFlagSettings({ env: providerEnv }); }
        catch { throw Error('provider_load_failed'); }
      }
      if (capabilities?.managed) {
        try {
          const configured = await agent.setMcpServers({ dgos_project: { type: 'http', url: capabilities.mcp_url!, headers: { Authorization: `Bearer ${capabilities.mcp_bearer!}` } } });
          if (!configured.added.includes('dgos_project')) throw Error('mcp_unavailable');
        } catch { throw Error('mcp_unavailable'); }
        try { await agent.reconnectMcpServer('dgos_project'); }
        catch { throw Error('mcp_unavailable'); }
      }
      const mcp = capabilities?.managed ? await agent.mcpServerStatus() : undefined;
      if (mcp && !mcp.some(server => server.name === 'dgos_project' && server.status === 'connected')) throw Error('mcp_unavailable');
      emit('capabilities/initialized', { commands: z.array(commandMetadata).parse(initialized.commands), skills: selectedSkills, mcp_servers: mcp?.map(server => ({ name: server.name, status: server.status })), plugins_applied: initialized.plugins_applied, status: 'registered_not_executed' });
    } catch (error) {
      agent.close();
      throw error;
    }
    return { session_id: sessionId };
  }
  if (!agent || closed) throw Error('closed');
  if (command.method === 'send') {
    const params = command.params;
    if (typeof params.text !== 'string' || !params.text.trim() || Buffer.byteLength(params.text) > 65536) throw Error('invalid_prompt');
    const fingerprint = createHash('sha256').update(params.text).digest('hex');
    if (receipts.has(params.request_id)) {
      if (receipts.get(params.request_id) !== fingerprint) throw Error('request_conflict');
      return {};
    }
    if (running) throw Error('busy');
    if (receipts.size >= 1024) throw Error('request_capacity');
    receipts.set(params.request_id, fingerprint); running = true;
    queue.push({ type: 'user', message: { role: 'user', content: params.text }, parent_tool_use_id: null, session_id: sessionId, uuid: randomUUID(), origin: { kind: 'human' } });
    wake?.(); return {};
  }
  if (command.method === 'respond') {
    const params = command.params;
    const pending = permissions.get(params.request_id);
    if (!pending) throw Error('request_expired');
    const response = params.response;
    if (response?.behavior === 'deny') pending.resolve({ behavior: 'deny', message: 'Denied by user' });
    else if (response?.behavior === 'allow') {
      const updatedInput = { ...pending.input };
      if (response.answers) {
        if (pending.tool !== 'AskUserQuestion') throw Error('invalid_response');
        const questions = z.array(z.object({ question: z.string() }).passthrough()).safeParse(pending.input.questions);
        if (!questions.success || Object.keys(response.answers).some(key => !questions.data.some(question => question.question === key))) throw Error('invalid_response');
        updatedInput.answers = response.answers;
      }
      pending.resolve({ behavior: 'allow', updatedInput });
    } else throw Error('invalid_response');
    return {};
  }
  if (command.method === 'interrupt') { await agent.interrupt(); return {}; }
  if (command.method === 'close') { await close(); return {}; }
  throw Error('unsupported_operation');
}
const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
lines.on('line', line => {
  if (Buffer.byteLength(line) > 4 * 1024 * 1024) { void close(); return; }
  let value: unknown;
  try { value = JSON.parse(line); } catch { void close(); return; }
  const parsed = commandSchema.safeParse(value);
  if (!parsed.success) { void close(); return; }
  const message = parsed.data;
  void handle(message).then(result => reply(message.id, result), error => {
    if (message.method === 'open' && message.params.capabilities) {
      const kind = error instanceof Error && ['skill_load_failed', 'provider_load_failed', 'provider_unavailable', 'mcp_unavailable'].includes(error.message) ? error.message : 'capability_load_failed';
      emit('failure', { kind, message: kind });
      reply(message.id, { capability_error: kind });
      return;
    }
    process.stdout.write(JSON.stringify({ id: message.id, error: { code: -32000, message: 'operation_failed' } }) + '\n');
  });
});
lines.on('close', () => { void close(); });
process.on('SIGTERM', () => { void close().finally(() => process.exit(0)); });
process.on('SIGINT', () => { void close().finally(() => process.exit(0)); });
