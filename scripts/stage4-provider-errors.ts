#!/usr/bin/env bun
// Exact command, from DG-OS repository root:
//   bun scripts/stage4-provider-errors.ts
// Requirements: Rust/Cargo, Bun, installed real codex/claude, built agent bridge.
// Does not read native settings or credentials. The Rust fixture generates its own
// random credentials in memory. All native configuration is under the temporary root.
import { access, chmod, mkdir, mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const repository = process.cwd();
const args = process.argv.slice(2);
const selectedEngine = args.length === 0 ? 'both' : args.length === 2 && args[0] === '--engine' ? args[1] : '';
if (!['both', 'codex', 'claude'].includes(selectedEngine)) throw new Error('invalid_engine');
const engines = selectedEngine === 'both' ? ['codex', 'claude'] : [selectedEngine];
const root = await mkdtemp(join(tmpdir(), 'dgos-stage4-provider-errors-'));
await chmod(root, 0o700);
let failure = 'bootstrap_failed';
const shell = (value: string) => "'" + value.replaceAll("'", "'\\''") + "'";
try {
  await access(join(repository, 'crates/dgos-agents/examples/stage4_provider_errors.rs'));
  const codex = Bun.which('codex');
  const claude = Bun.which('claude');
  const cargo = Bun.which('cargo');
  const bridge = resolve(process.env.DGOS_CLAUDE_BRIDGE ?? join(repository, 'tools/agent-bridge/bin/dgos-claude-bridge'));
  if (!cargo || engines.includes('codex') && !codex || engines.includes('claude') && !claude) throw Error('native_cli_or_cargo_missing');
  if (engines.includes('claude')) await access(bridge);
  for (const directory of ['home', 'codex-home', 'claude-home', 'skills', 'capability-material', 'cache', 'config', 'data']) {
    await mkdir(join(root, directory), { mode: 0o700 });
  }
  await writeFile(join(root, 'stage4-provider-errors-isolated'), 'DG-OS isolated provider error fixture\n', { mode: 0o600 });
  // The launcher is deliberately a real-CLI pass-through. Only --version can be
  // changed for the explicitly labelled inert recovery scenario. Never fake a turn.
  for (const [engine, executable] of [['codex', codex], ['claude', claude]]) {
    if (!engines.includes(engine!)) continue;
    const marker = join(root, `${engine}-changed-version`);
    const source = `#!/bin/sh\nif [ "$#" -eq 1 ] && [ "$1" = --version ] && [ -f ${shell(marker)} ]; then\n  printf '%s\\n' 'stage4-controlled-version-change'\n  exit 0\nfi\n${engine === 'codex' ? `if [ "$#" -eq 1 ] && [ "$1" = --version ]; then\n  exec ${shell(executable!)} "$@"\nfi\nexec ${shell(executable!)} "$@" -c 'model_providers.dgos_project.request_max_retries=0' -c 'model_providers.dgos_project.stream_max_retries=0'\n` : `export CLAUDE_CODE_MAX_RETRIES=0\nexport ANTHROPIC_MAX_RETRIES=0\nexport MAX_RETRIES=0\nexec ${shell(executable!)} "$@"\n`}`;
    await writeFile(join(root, `${engine}-cli`), source, { mode: 0o700 });
  }
  const description = 'Local throwaway package used only to prove missing skill recovery fails closed.';
  // Each engine owns a distinct package: uninstalling the Codex fixture must not
  // make Claude's preview reuse an already committed, subsequently removed install.
  for (const engine of engines) {
    const id = `stage4-${engine}-provider-errors-restore`;
    await writeFile(join(root, `${engine}-restore-skill.zip`), zipStore([
      ['SKILL.md', `---\nname: ${id}\ndescription: ${description}\n---\n# Restore fixture\nThis local fixture has no tools or executable instructions.\n`],
      ['skill.json', JSON.stringify({ schema_version: '1.0', id, name: id, description, version: '1.0.0', identity: 'stage4_local_fixture', entry: 'SKILL.md', license: 'MIT' })],
      ['LICENSE', 'MIT\n'],
    ]), { mode: 0o600 });
  }
  // Use an allowlist so ambient credentials, cloud selectors, proxy settings and
  // provider configuration cannot change the selected local fixture. Preserve only
  // build/tool discovery variables; the two native HOME/config variables stay isolated.
  const env: Record<string, string> = {};
  for (const key of ['PATH', 'USER', 'LOGNAME', 'SHELL', 'LANG', 'LC_ALL', 'TMPDIR', 'CARGO_HOME', 'RUSTUP_HOME', 'CARGO_TARGET_DIR', 'SDKROOT', 'MACOSX_DEPLOYMENT_TARGET', 'DEVELOPER_DIR']) {
    if (process.env[key]) env[key] = process.env[key]!;
  }
  env.HOME = join(root, 'home');
  env.CODEX_HOME = join(root, 'codex-home');
  env.CLAUDE_CONFIG_DIR = join(root, 'claude-home');
  env.XDG_CACHE_HOME = join(root, 'cache');
  env.XDG_CONFIG_HOME = join(root, 'config');
  env.XDG_DATA_HOME = join(root, 'data');
  env.DGOS_CLAUDE_BRIDGE = bridge;
  env.DGOS_STAGE4_ENGINE = selectedEngine;
  env.RUST_LOG = 'off';
  env.CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC = '1';
  env.DISABLE_TELEMETRY = '1';
  env.DISABLE_ERROR_REPORTING = '1';
  env.CLAUDE_CODE_MAX_RETRIES = '0';
  env.ANTHROPIC_MAX_RETRIES = '0';
  env.MAX_RETRIES = '0';
  // Cargo/rustup normally discover toolchains in the real home. Pin their existing
  // directories without allowing either CLI to see the user's native configuration.
  const originalHome = process.env.HOME;
  if (originalHome) {
    env.CARGO_HOME ??= join(originalHome, '.cargo');
    env.RUSTUP_HOME ??= join(originalHome, '.rustup');
  }
  await writeFile(join(root, 'launch.json'), JSON.stringify({
    command: ['cargo', 'run', '--quiet', '-p', 'dgos-agents', '--example', 'stage4_provider_errors', '--', '--root', root],
    isolated: ['HOME', 'CODEX_HOME', 'CLAUDE_CONFIG_DIR', 'XDG_CACHE_HOME', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME'],
    native_config_read: false,
    fixture_only: true,
    retry_control: { codex: ['request_max_retries=0', 'stream_max_retries=0'], claude: ['CLAUDE_CODE_MAX_RETRIES=0', 'ANTHROPIC_MAX_RETRIES=0', 'MAX_RETRIES=0'], caveat: 'Observed request counts and deadlines report whether this CLI honors the controls; no retry-disable compatibility is assumed.' },
  }, null, 2), { mode: 0o600 });
  failure = 'cargo_or_acceptance_failed';
  const child = Bun.spawn([cargo, 'run', '--quiet', '-p', 'dgos-agents', '--example', 'stage4_provider_errors', '--', '--root', root], { cwd: repository, env, stdin: 'ignore', stdout: 'pipe', stderr: 'pipe' });
  // Drain and discard all native/build output in memory; never save a transcript
  // that could accidentally contain a process-local fixture credential. Rust writes
  // only deliberately selected structural evidence under root.
  const output = new Response(child.stdout).text();
  const errors = new Response(child.stderr).arrayBuffer();
  const code = await child.exited;
  await Promise.all([output, errors]);
  console.log(JSON.stringify({ result: code === 0 ? 'passed' : 'failed', artifact: root }));
  process.exitCode = code === 0 ? 0 : 1;
} catch {
  await writeFile(join(root, 'bootstrap-failure.json'), JSON.stringify({ result: 'failed', reason: failure }), { mode: 0o600 });
  console.log(JSON.stringify({ result: 'failed', artifact: root }));
  process.exitCode = 1;
}

function crc32(bytes: Uint8Array): number {
  let crc = 0xffffffff;
  for (const byte of bytes) { crc ^= byte; for (let i = 0; i < 8; i++) crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1)); }
  return (crc ^ 0xffffffff) >>> 0;
}
// Dependency-free stored ZIP, still validated by production SkillsManager.
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
