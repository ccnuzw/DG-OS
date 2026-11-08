#!/usr/bin/env bun
// Run from DG-OS root with DGOS_STAGE4_{CODEX,CLAUDE}_{URL,KEY,MODEL}: bun scripts/stage4-acceptance.ts
// Restart: bun scripts/stage4-acceptance.ts --resume /tmp/dgos-stage4-<id>
// Requires Bun, Rust/Cargo, node, codex, claude and npm run agents:build.
// --desktop runs the visible production WebView acceptance instead of the Rust round-trip.
// Provider values must be supplied explicitly; no native configuration is read.
// Keys exist only in this process and the Rust child environment; never on disk/argv.
import { mkdir, mkdtemp, readFile, writeFile, access } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { randomUUID } from 'node:crypto';

const fail = (message: string): never => { throw new Error(message); };
const text = (value: unknown): string | undefined => typeof value === 'string' && value.trim() ? value.trim() : undefined;
const args = process.argv.slice(2);
const desktop = args[0] === '--desktop';
if (desktop) args.shift();
if (!(args.length === 0 || (args.length === 2 && args[0] === '--resume'))) fail('usage: bun scripts/stage4-acceptance.ts [--desktop | --resume <artifact-dir>]');
const resume = args.length !== 0;
const root = resume ? resolve(args[1]) : await mkdtemp(join(tmpdir(), 'dgos-stage4-'));
const repository = process.cwd();
let failureClass = 'bootstrap_failed';
try {
  await access(join(repository, 'crates/dgos-agents/Cargo.toml'));
  if (resume) await access(join(root, 'stage4-isolated-root'));
  else {
    await writeFile(join(root, 'stage4-isolated-root'), 'DG-OS temporary acceptance artifact\n', { mode: 0o600 });
    for (const name of ['codex-home', 'claude-home', 'evidence', 'project-codex', 'project-claude', 'home']) { const path = join(root, name); await mkdir(path, { mode: 0o700 }); }
    await writeFile(join(root, 'fixture-observations.jsonl'), '', { mode: 0o600 });
  }
  failureClass = 'explicit_provider_configuration_incomplete';
  const codexKey = text(process.env.DGOS_STAGE4_CODEX_KEY);
  const codexUrl = text(process.env.DGOS_STAGE4_CODEX_URL);
  const codexModel = text(process.env.DGOS_STAGE4_CODEX_MODEL);
  const claudeKey = text(process.env.DGOS_STAGE4_CLAUDE_KEY);
  const claudeUrl = text(process.env.DGOS_STAGE4_CLAUDE_URL);
  const claudeModel = text(process.env.DGOS_STAGE4_CLAUDE_MODEL);
  if (!codexKey || !codexUrl || !codexModel || !claudeKey || !claudeUrl || !claudeModel) fail(failureClass);
  if (desktop && resume) fail('desktop_resume_not_supported');
  for (const raw of [codexUrl, claudeUrl]) { const url = new URL(raw); if (url.username || url.password || url.search || url.hash || !['http:', 'https:'].includes(url.protocol)) fail('provider_endpoint_invalid'); }
  const env: Record<string, string | undefined> = { ...process.env,
    HOME: join(root, 'home'), CODEX_HOME: join(root, 'codex-home'), CLAUDE_CONFIG_DIR: join(root, 'claude-home'),
    DGOS_STAGE4_CODEX_URL: codexUrl, DGOS_STAGE4_CODEX_MODEL: codexModel, DGOS_STAGE4_CODEX_KEY: codexKey,
    DGOS_STAGE4_CLAUDE_URL: claudeUrl, DGOS_STAGE4_CLAUDE_MODEL: claudeModel, DGOS_STAGE4_CLAUDE_KEY: claudeKey,
    DGOS_CLAUDE_BRIDGE: process.env.DGOS_CLAUDE_BRIDGE ?? join(repository, 'tools/agent-bridge/bin/dgos-claude-bridge'), RUST_LOG: 'off',
  };
  for (const name of ['OPENAI_API_KEY', 'OPENAI_AUTH_TOKEN', 'OPENAI_BASE_URL', 'ANTHROPIC_AUTH_TOKEN', 'ANTHROPIC_API_KEY', 'ANTHROPIC_BASE_URL', 'CLAUDE_CODE_OAUTH_TOKEN', 'CLAUDE_CODE_API_KEY', 'DGOS_PROVIDER_KEY', 'DGOS_MCP_BEARER']) delete env[name];
  failureClass = 'bridge_binary_missing_run_npm_run_agents_build';
  await access(env.DGOS_CLAUDE_BRIDGE!);
  if (!resume) {
    for (const engine of ['codex', 'claude']) {
      const id = `stage4-${engine}-proof`;
      const markers = { body: `BODY_${randomUUID()}`, resource: `RESOURCE_${randomUUID()}` };
      const description = 'Read the installed native Stage4 proof resource and invoke the configured native MCP fixture.';
      const body = `---\nname: ${id}\ndescription: ${description}\n---\n# Stage4 native proof procedure\n\n${markers.body}\n\nUse the native Skill mechanism for this skill, then use a native file-read tool to read this whole SKILL.md and references/proof.txt on every round, even if already in context. The reference contains a public fixture value, not a credential. Invoke the configured native MCP server stage4 with exactly {"challenge": "the current user challenge", "resource": "the reference value"}. Complete the native approval flow when requested. Write the fresh returned receipt verbatim to the requested round-N.txt. Never invent a receipt or replace native MCP with HTTP, shell, or another tool.\n`;
      const manifest = JSON.stringify({ schema_version: '1.0', id, name: id, description, version: '1.0.0', identity: 'stage4_local_fixture', entry: 'SKILL.md', license: 'MIT' });
      await writeFile(join(root, `${engine}-skill.zip`), zipStore([['SKILL.md', body], ['skill.json', manifest], ['LICENSE', 'MIT\n'], ['references/proof.txt', `${markers.resource}\n`]]), { mode: 0o600 });
      await writeFile(join(root, `${engine}-markers.json`), JSON.stringify(markers), { mode: 0o600 });
    }
    await writeFile(join(root, 'fixture.mjs'), fixtureSource(), { mode: 0o600 });
  }
  console.log(`stage4 evidence=${root}`);
  failureClass = 'cargo_or_acceptance_failed';
  if (desktop) { env.DGOS_STAGE4_FIXTURE_ROOT = root; env.DGOS_PROJECT_ACCEPTANCE_ROOT = join(root, 'desktop-ui'); env.DGOS_PROJECT_ACCEPTANCE_PHASE = 'stage4-ui'; }
  const command = desktop ? ['cargo', 'test', '--offline', '-p', 'dgos', '--features', 'webview-smoke', '--test', 'webview_smoke_agents', '--', '--nocapture'] : ['cargo', 'run', '--offline', '-p', 'dgos-agents', '--example', 'stage4_acceptance', '--', '--root', root, ...(resume ? ['--resume'] : [])];
  const child = Bun.spawn(command, { cwd: repository, env, stdin: 'inherit', stdout: 'inherit', stderr: 'inherit' });
  const code = await child.exited;
  process.exitCode = code;
  if (code === 0 && !resume && !desktop) console.log(`restart: bun scripts/stage4-acceptance.ts --resume ${root}`);
} catch {
  // Never stringify native configuration errors, command objects, or provider keys.
  console.error(`stage4 bootstrap failed: ${failureClass}; artifact=${root}`);
  process.exitCode = 1;
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
