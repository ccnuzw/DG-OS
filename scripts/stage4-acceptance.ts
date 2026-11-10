#!/usr/bin/env bun
// Run both engines with DGOS_STAGE4_{CODEX,CLAUDE}_{URL,KEY,MODEL}, or select --engine claude|codex.
// Restart with the same --engine selection and --resume /tmp/dgos-stage4-<id>.
// Requires Bun, Rust/Cargo, node, selected CLIs and the Claude bridge when selected.
// --desktop runs the visible production WebView acceptance instead of the Rust round-trip.
// --saved-config --desktop uses only NextGpt/gpt-6-astra and Kiro/claude-sonnet-4-6
// metadata from the read-only production DB and scoped Keychain credentials.
// Example: bun scripts/stage4-acceptance.ts --saved-config --desktop --engine both
// Without --saved-config, provider values must be supplied explicitly.
// Keys exist only in private memory/child environment; never on disk/argv/logs.
import { mkdir, mkdtemp, readFile, writeFile, access } from 'node:fs/promises';
import { homedir, tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { randomUUID } from 'node:crypto';
import { readSavedAcceptanceProvider, runIsolated, SavedProviderError } from './real-codex-acceptance';
const fail = (message: string): never => { throw new Error(message); };
const text = (value: unknown): string | undefined => typeof value === 'string' && value.trim() ? value.trim() : undefined;
const args = process.argv.slice(2);
const engineIndex = args.indexOf('--engine');
const selectedEngine = engineIndex >= 0 ? args[engineIndex + 1] : 'both';
if (!['both', 'claude', 'codex'].includes(selectedEngine ?? '')) fail('invalid_engine');
if (engineIndex >= 0) args.splice(engineIndex, 2);
const engines = selectedEngine === 'both' ? ['codex', 'claude'] : [selectedEngine];
const savedConfigIndex = args.indexOf('--saved-config');
const savedConfig = savedConfigIndex >= 0;
if (savedConfig) args.splice(savedConfigIndex, 1);
const desktop = args[0] === '--desktop';
if (desktop) args.shift();
if (desktop && selectedEngine !== 'both') fail('desktop_engine_selection_not_supported');
if (!(args.length === 0 || (args.length === 2 && args[0] === '--resume'))) fail('usage: bun scripts/stage4-acceptance.ts [--saved-config] [--engine claude|codex|both] [--desktop | --resume <artifact-dir>]');
const resume = args.length !== 0;
const root = resume ? resolve(args[1]) : await mkdtemp(join(tmpdir(), 'dgos-stage4-'));
const repository = process.cwd();
let failureClass = 'bootstrap_failed';
let childEnv: Record<string, string | undefined> | undefined;
const configured: Record<string, { key?: string; url?: string; model?: string; authStyle?: string }> = {};
try {
  await access(join(repository, 'crates/dgos-agents/Cargo.toml'));
  if (resume) await access(join(root, 'stage4-isolated-root'));
  if (resume) for (const name of ['config', 'cache', 'data', 'tmp']) await mkdir(join(root, name), { mode: 0o700, recursive: true });
  else {
    await writeFile(join(root, 'stage4-isolated-root'), 'DG-OS temporary acceptance artifact\n', { mode: 0o600 });
    for (const name of ['codex-home', 'claude-home', 'evidence', 'project-codex', 'project-claude', 'home', 'config', 'cache', 'data', 'tmp']) { const path = join(root, name); await mkdir(path, { mode: 0o700 }); }
    await writeFile(join(root, 'fixture-observations.jsonl'), '', { mode: 0o600 });
  }
  failureClass = savedConfig ? 'saved_provider_configuration_unavailable' : 'explicit_provider_configuration_incomplete';
  for (const engine of engines) {
    if (savedConfig) {
      const route = await readSavedAcceptanceProvider(engine as 'codex' | 'claude');
      configured[engine] = { key: route.key, url: route.provider.base_url, model: route.model, authStyle: route.provider.auth_style as string | undefined };
      console.log(`stage4 route engine=${engine} provider=${route.provider.name} model=${route.model} protocol=${engine === 'codex' ? 'responses_candidate' : 'messages_candidate'} credential=available production_database=read_only`);
    } else {
      const prefix = `DGOS_STAGE4_${engine.toUpperCase()}`;
      configured[engine] = { key: text(process.env[`${prefix}_KEY`]), url: text(process.env[`${prefix}_URL`]), model: text(process.env[`${prefix}_MODEL`]), authStyle: text(process.env[`${prefix}_AUTH_STYLE`]) };
    }
  }
  for (const engine of engines) {
    const provider = configured[engine as keyof typeof configured];
    if (!provider.key || !provider.url || !provider.model) fail(failureClass);
    const url = new URL(provider.url);
    if (url.username || url.password || url.search || url.hash || !['http:', 'https:'].includes(url.protocol)) fail('provider_endpoint_invalid');
  }
  if (desktop && resume) fail('desktop_resume_not_supported');
  const hostHome = homedir();
  failureClass = 'required_executable_missing';
  const env: Record<string, string | undefined> = savedConfig ? {
    // Do not inherit provider keys, proxies, CLI accounts, auth helpers or fixture flags.
    PATH: [...new Set(['cargo', 'node', ...engines].map(name => {
      const binary = Bun.which(name);
      if (!binary) fail('required_executable_missing');
      return dirname(binary);
    }).concat([dirname(process.execPath), '/usr/bin', '/bin', '/usr/sbin', '/sbin']))].join(':'),
    LANG: 'en_US.UTF-8', RUST_BACKTRACE: '0',
  } : { ...process.env };
  childEnv = env;
  Object.assign(env, {
    CARGO_HOME: process.env.CARGO_HOME ?? join(hostHome, '.cargo'),
    RUSTUP_HOME: process.env.RUSTUP_HOME ?? join(hostHome, '.rustup'),
    HOME: join(root, 'home'), CODEX_HOME: join(root, 'codex-home'), CLAUDE_CONFIG_DIR: join(root, 'claude-home'),
    XDG_CONFIG_HOME: join(root, 'config'), XDG_CACHE_HOME: join(root, 'cache'), XDG_DATA_HOME: join(root, 'data'),
    TMPDIR: `${join(root, 'tmp')}/`, TMP: join(root, 'tmp'), TEMP: join(root, 'tmp'),
    DGOS_CLAUDE_BRIDGE: process.env.DGOS_CLAUDE_BRIDGE ?? join(repository, 'tools/agent-bridge/bin/dgos-claude-bridge'), RUST_LOG: 'off',
    DGOS_STAGE4_ENGINE: selectedEngine,
  });
  for (const name of ['CARGO_TARGET_DIR', 'SDKROOT', 'MACOSX_DEPLOYMENT_TARGET']) if (process.env[name]) env[name] = process.env[name];
  for (const engine of engines) {
    const provider = configured[engine];
    const prefix = `DGOS_STAGE4_${engine.toUpperCase()}`;
    env[`${prefix}_URL`] = provider.url; env[`${prefix}_MODEL`] = provider.model; env[`${prefix}_KEY`] = provider.key;
    // Preserve only this supported, non-secret protocol metadata from saved config.
    if (provider.authStyle != null) env[`${prefix}_AUTH_STYLE`] = provider.authStyle;
  }
  for (const name of ['OPENAI_API_KEY', 'OPENAI_AUTH_TOKEN', 'OPENAI_BASE_URL', 'ANTHROPIC_AUTH_TOKEN', 'ANTHROPIC_API_KEY', 'ANTHROPIC_BASE_URL', 'CLAUDE_CODE_OAUTH_TOKEN', 'CLAUDE_CODE_API_KEY', 'DGOS_PROVIDER_KEY', 'DGOS_MCP_BEARER']) delete env[name];
  for (const engine of ['codex', 'claude']) {
    if (engines.includes(engine)) continue;
    for (const field of ['URL', 'MODEL', 'KEY']) delete env[`DGOS_STAGE4_${engine.toUpperCase()}_${field}`];
  }
  if (engines.includes('claude')) {
    failureClass = 'bridge_binary_missing_run_npm_run_agents_build';
    await access(env.DGOS_CLAUDE_BRIDGE!);
  }
  if (!resume) {
    for (const engine of engines) {
      const id = `stage4-${engine}-proof`;
      const markers = { body: `BODY_${randomUUID()}`, resource: `RESOURCE_${randomUUID()}` };
      const description = 'Read the installed native Stage4 proof resource and invoke the configured native MCP fixture.';
      const readProcedure = engine === 'codex'
        ? 'The harness selects this installed skill with a structured native skill input. Use the native commandExecution tool (for example cat) to read this whole SKILL.md and references/proof.txt on every round, even if already in context. Codex has no separate Skill or Read tool; native commandExecution file reads are allowed, but must never replace MCP dispatch.'
        : 'Use the native Skill mechanism for this skill, then use a native file-read tool to read this whole SKILL.md and references/proof.txt on every round, even if already in context.';
      const approvalProcedure = engine === 'codex'
        ? 'Complete any actual native approval request. If the CLI does not expose MCP tool approval, still invoke the configured native MCP tool to diagnose dispatch; missing approval remains a blocked acceptance criterion, not a reason to skip the diagnostic call.'
        : 'Complete the native approval flow when requested.';
      const body = `---\nname: ${id}\ndescription: ${description}\n---\n# Stage4 native proof procedure\n\n${markers.body}\n\n${readProcedure} The reference contains a public fixture value, not a credential. Invoke the configured native MCP server stage4 with exactly {"challenge": "the current user challenge", "resource": "the reference value"}. ${approvalProcedure} Write the fresh returned receipt verbatim to the requested round-N.txt. Never invent a receipt or replace native MCP with HTTP, shell, or another tool.\n`;
      const manifest = JSON.stringify({ schema_version: '1.0', id, name: id, description, version: '1.0.0', identity: 'stage4_local_fixture', entry: 'SKILL.md', license: 'MIT' });
      await writeFile(join(root, `${engine}-skill.zip`), zipStore([['SKILL.md', body], ['skill.json', manifest], ['LICENSE', 'MIT\n'], ['references/proof.txt', `${markers.resource}\n`]]), { mode: 0o600 });
      await writeFile(join(root, `${engine}-markers.json`), JSON.stringify(markers), { mode: 0o600 });
    }
    await writeFile(join(root, 'fixture.mjs'), fixtureSource(), { mode: 0o600 });
  }
  if (engines.includes('codex')) {
    failureClass = 'codex_native_protocol_unavailable';
    const protocolRoot = join(root, 'codex-protocol');
    const protocolEnv = { PATH: env.PATH, HOME: env.HOME, CODEX_HOME: env.CODEX_HOME };
    const exported = await Bun.spawn(['codex', 'app-server', 'generate-json-schema', '--experimental', '--out', protocolRoot], { env: protocolEnv, stdout: 'ignore', stderr: 'ignore' }).exited;
    if (exported !== 0) fail(failureClass);
    const schema = JSON.parse(await readFile(join(protocolRoot, 'ServerRequest.json'), 'utf8'));
    const methods: unknown[] | undefined = schema.oneOf?.flatMap((variant: { properties?: { method?: { enum?: unknown[] } } }) => variant.properties?.method?.enum ?? []);
    if (!Array.isArray(methods) || methods.length === 0 || !methods.every(method => typeof method === 'string')) fail(failureClass);
    // These actual installed protocol requests do not grant permission to an MCP
    // tool. In particular, sandbox permissions and elicitation are not substitutes.
    const nonToolPermissionMethods: Record<string, true> = { 'item/commandExecution/requestApproval': true, 'item/fileChange/requestApproval': true, 'item/tool/requestUserInput': true, 'mcpServer/elicitation/request': true, 'item/permissions/requestApproval': true, 'item/tool/call': true, 'account/chatgptAuthTokens/refresh': true, 'attestation/generate': true, 'currentTime/read': true, 'applyPatchApproval': true, 'execCommandApproval': true };
    const permissions = JSON.parse(await readFile(join(protocolRoot, 'PermissionsRequestApprovalParams.json'), 'utf8'));
    const permissionFields = Object.keys(permissions.definitions?.RequestPermissionProfile?.properties ?? {});
    const unsupported = methods.every(method => typeof method === 'string' && Object.hasOwn(nonToolPermissionMethods, method)) && permissionFields.length > 0 && permissionFields.every(field => field === 'fileSystem' || field === 'network');
    const versionProcess = Bun.spawn(['codex', '--version'], { env: protocolEnv, stdout: 'pipe', stderr: 'ignore' });
    const version = (await new Response(versionProcess.stdout).text()).trim();
    if (await versionProcess.exited !== 0) fail(failureClass);
    await writeFile(join(root, 'evidence/codex-native-protocol.json'), JSON.stringify({ source: 'installed_cli_generated_schema', version, server_request_methods: methods, permission_profile_fields: permissionFields, mcp_tool_approval: unsupported ? 'unsupported' : 'unverified', schema_directory: protocolRoot }), { mode: 0o600 });
  }
  console.log(`stage4 evidence=${root}`);
  failureClass = 'cargo_or_acceptance_failed';
  if (desktop) { env.DGOS_STAGE4_FIXTURE_ROOT = root; env.DGOS_PROJECT_ACCEPTANCE_ROOT = join(root, 'desktop-ui'); env.DGOS_PROJECT_ACCEPTANCE_PHASE = 'stage4-ui'; }
  const command = desktop ? ['cargo', 'test', '--offline', '-p', 'dgos', '--features', 'webview-smoke', '--test', 'webview_smoke_agents', '--', '--nocapture'] : ['cargo', 'run', '--offline', '-p', 'dgos-agents', '--example', 'stage4_acceptance', '--', '--root', root, ...(resume ? ['--resume'] : [])];
  const code = savedConfig
    ? await runIsolated(command, repository, env as Record<string, string>)
    : await Bun.spawn(command, { cwd: repository, env, stdin: 'inherit', stdout: 'inherit', stderr: 'inherit' }).exited;
  process.exitCode = code;
  if (code === 0 && !resume && !desktop) console.log(`restart: bun scripts/stage4-acceptance.ts ${savedConfig ? '--saved-config ' : ''}--engine ${selectedEngine} --resume ${root}`);
} catch (error) {
  if (error instanceof SavedProviderError) failureClass = error.code;
  // Never stringify native configuration errors, command objects, or provider keys.
  console.error(`stage4 bootstrap failed: ${failureClass}; artifact=${root}`);
  process.exitCode = 1;
}
finally {
  for (const provider of Object.values(configured)) provider.key = undefined;
  if (childEnv) for (const name of Object.keys(childEnv)) if (name.endsWith('_KEY')) delete childEnv[name];
}

function crc32(bytes: Uint8Array): number {
  let crc = 0xffffffff;
  for (const byte of bytes) { crc ^= byte; for (let i = 0; i < 8; i++) crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1)); }
  return (crc ^ 0xffffffff) >>> 0;
}
// A small uncompressed ZIP keeps this temporary bootstrap dependency-free. Package
// bytes are still validated and installed by the production NativeExtensions API.
function zipStore(files: [string, string][]): Buffer {
  const parts: Buffer[] = [], directory: Buffer[] = [];
  let offset = 0;
  for (const [path, text] of files) {
    const name = Buffer.from(path), bytes = Buffer.from(text), crc = crc32(bytes);
    const local = Buffer.alloc(30); local.writeUInt32LE(0x04034b50); local.writeUInt16LE(20, 4); local.writeUInt16LE(0x21, 12); local.writeUInt32LE(crc, 14); local.writeUInt32LE(bytes.length, 18); local.writeUInt32LE(bytes.length, 22); local.writeUInt16LE(name.length, 26);
    parts.push(local, name, bytes);
    const central = Buffer.alloc(46); central.writeUInt32LE(0x02014b50); central.writeUInt16LE(0x0314, 4); central.writeUInt16LE(20, 6); central.writeUInt16LE(0x21, 14); central.writeUInt32LE(crc, 16); central.writeUInt32LE(bytes.length, 20); central.writeUInt32LE(bytes.length, 24); central.writeUInt16LE(name.length, 28); central.writeUInt32LE(0x81a40000, 38); central.writeUInt32LE(offset, 42);
    directory.push(central, name); offset += local.length + name.length + bytes.length;
  }
  const central = Buffer.concat(directory), end = Buffer.alloc(22); end.writeUInt32LE(0x06054b50); end.writeUInt16LE(files.length, 8); end.writeUInt16LE(files.length, 10); end.writeUInt32LE(central.length, 12); end.writeUInt32LE(offset, 16);
  return Buffer.concat([...parts, central, end]);
}

function fixtureSource(): string { return `import readline from 'node:readline';
import { appendFileSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
const engine = process.argv[2];
const root = process.argv[3];
const resource = JSON.parse(readFileSync(join(root, engine + '-markers.json'), 'utf8')).resource;
const log = join(root, 'fixture-observations.jsonl');
const counts = new Map();
let initialized = false;
function reply(id, result) { process.stdout.write(JSON.stringify({jsonrpc:'2.0',id,result})+'\\n'); }
function observe(value) { appendFileSync(log, JSON.stringify({fixture:true,engine,...value,at:new Date().toISOString()})+'\\n',{mode:0o600}); }
readline.createInterface({input:process.stdin}).on('line', line => {
  const request = JSON.parse(line);
  if (request.method === 'initialize') reply(request.id, {protocolVersion:'2025-03-26',capabilities:{tools:{}},serverInfo:{name:'stage4-local-fixture',version:'1.0.0'}});
  else if (request.method === 'notifications/initialized') initialized = true;
  else if (request.method === 'ping') reply(request.id, {});
  else if (request.method === 'tools/list') {
    if (!initialized) process.exit(4);
    observe({kind:'schema_discovery',method:request.method,tool:'stage4_receipt'});
    reply(request.id, {tools:[{name:'stage4_receipt',description:'Local Stage4 fixture. Request a fresh receipt using the resource value read from the installed proof skill.',inputSchema:{type:'object',required:['challenge','resource'],additionalProperties:false,properties:{challenge:{type:'string',minLength:1},resource:{type:'string',minLength:1}}}}]});
  } else if (request.method === 'tools/call') {
    const args = request.params.arguments;
    if (request.params.name !== 'stage4_receipt' || !args || !args.challenge || args.resource !== resource) { reply(request.id,{isError:true,content:[{type:'text',text:'resource_mismatch'}]}); return; }
    const count = (counts.get(engine) ?? 0) + 1;
    counts.set(engine, count);
    const receipt = 'RECEIPT_' + engine.toUpperCase() + '_' + count + '_' + randomUUID();
    observe({kind:'execution',method:request.method,tool:request.params.name,challenge:args.challenge,resource:args.resource,count,receipt});
    reply(request.id,{isError:false,content:[{type:'text',text:receipt}]});
  }
});
`;
}
