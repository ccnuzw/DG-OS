#!/usr/bin/env bun
// Real provider-bound acceptance, never native-account authentication:
//   bun scripts/real-codex-acceptance.ts --provider NextGpt --model gpt-6-astra --check
//   bun scripts/real-codex-acceptance.ts --provider NextGpt --model gpt-6-astra
// Explicit write/operation opt-ins: --mode coding or --mode operations.
// Default --mode readonly33 retains the original 33-check read-only journey.
// Optional Manager skill/MCP run: append --stage4 (not desktop lifecycle proof).
// Reads only selected provider/model metadata from the production SQLite DB.
// Keys are read through a bounded, piped Keychain subprocess and passed only in
// the isolated child's environment. No credential is written to disk or argv.
// Temporary HOME/config/project/evidence directories are deliberately preserved.
import { Database } from 'bun:sqlite';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, chmod } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { spawn } from 'node:child_process';
import { setTimeout, clearTimeout } from 'node:timers';
import { setTimeout as delay } from 'node:timers/promises';

const fail = (code: string): never => { throw new Error(code); };
export class SavedProviderError extends Error {
  constructor(readonly code: string) { super(code); }
}
// Importing the saved-configuration helpers never reads the DB or Keychain.
if (import.meta.main) {
const usage = 'usage: bun scripts/real-codex-acceptance.ts --provider NextGpt --model gpt-6-astra [--mode readonly33|coding|operations | --check | --stage4]';
const args = process.argv.slice(2);
let providerName: string | undefined;
let modelName: string | undefined;
let mode: 'desktop' | 'check' | 'stage4' = 'desktop';
let desktopMode: 'readonly33' | 'coding' | 'operations' = 'readonly33';
let desktopModeSelected = false;
let failureClass = 'invalid_arguments';
let root: string | undefined;
let secret: string | undefined;
let childEnv: Record<string, string> | undefined;

try {
  for (let index = 0; index < args.length; index++) {
    const argument = args[index];
    if (argument === '--provider' && providerName === undefined) providerName = args[++index];
    else if (argument === '--model' && modelName === undefined) modelName = args[++index];
    else if (argument === '--mode' && mode === 'desktop' && !desktopModeSelected) {
      const selected = args[++index];
      if (selected !== 'readonly33' && selected !== 'coding' && selected !== 'operations') fail('invalid_arguments');
      desktopMode = selected;
      desktopModeSelected = true;
    }
    else if ((argument === '--check' || argument === '--stage4') && mode === 'desktop' && !desktopModeSelected) mode = argument === '--check' ? 'check' : 'stage4';
    else fail('invalid_arguments');
  }
  // These are the only names authorized for this real-credential acceptance.
  if (providerName !== 'NextGpt' || modelName !== 'gpt-6-astra') fail('invalid_arguments');
  if (process.platform !== 'darwin') { failureClass = 'macos_keychain_required'; fail(failureClass); }

  const hostHome = homedir();
  const { provider, endpoint, key } = await readSavedAcceptanceProvider('codex');
  secret = key;

  console.log(`real-codex route provider=${providerName} model=${modelName} origin=${endpoint.origin} protocol=responses_candidate credential=available`);
  if (mode === 'check') {
    console.log('real-codex check=passed model_contacted=false production_database=read_only');
  } else {
    failureClass = 'required_executable_missing';
    // Resolve against the original PATH, before replacing HOME. In particular,
    // never look for Codex inside the isolated HOME or fall back to native auth.
    const codex = Bun.which('codex');
    const cargo = Bun.which('cargo');
    const node = Bun.which('node');
    if (!codex || !cargo || !node) fail(failureClass);
    const repository = resolve(import.meta.dir, '..');
    failureClass = 'isolated_root_creation_failed';
    root = await mkdtemp('/private/tmp/dgos-real-codex-');
    await chmod(root, 0o700);
    for (const name of ['home', 'codex-home', 'claude-home', 'tmp', 'config', 'cache', 'data']) await mkdir(join(root, name), { mode: 0o700 });
    // Allowlist, not a blacklist: no inherited provider keys, proxy credentials,
    // credential helpers, native account env, fixture flags, or arbitrary DGOS flags.
    childEnv = {
      PATH: [...new Set([dirname(codex), dirname(cargo), dirname(node), dirname(process.execPath), '/usr/bin', '/bin', '/usr/sbin', '/sbin'])].join(':'),
      HOME: join(root, 'home'), CODEX_HOME: join(root, 'codex-home'), CLAUDE_CONFIG_DIR: join(root, 'claude-home'),
      XDG_CONFIG_HOME: join(root, 'config'), XDG_CACHE_HOME: join(root, 'cache'), XDG_DATA_HOME: join(root, 'data'),
      TMPDIR: `${join(root, 'tmp')}/`, TMP: join(root, 'tmp'), TEMP: join(root, 'tmp'),
      CARGO_HOME: process.env.CARGO_HOME ?? join(hostHome, '.cargo'),
      RUSTUP_HOME: process.env.RUSTUP_HOME ?? join(hostHome, '.rustup'),
      RUST_LOG: 'off', RUST_BACKTRACE: '0', LANG: 'en_US.UTF-8',
    };
    // Toolchain-only overrides permit the parent to reuse the prepared build.
    for (const name of ['CARGO_TARGET_DIR', 'SDKROOT', 'MACOSX_DEPLOYMENT_TARGET']) {
      if (process.env[name]) childEnv[name] = process.env[name]!;
    }
    let command: string[];
    if (mode === 'stage4') {
      Object.assign(childEnv, { DGOS_STAGE4_CODEX_URL: provider.base_url, DGOS_STAGE4_CODEX_KEY: secret, DGOS_STAGE4_CODEX_MODEL: modelName });
      command = [process.execPath, join(repository, 'scripts/stage4-acceptance.ts'), '--engine', 'codex'];
    } else {
      Object.assign(childEnv, {
        DGOS_CODEX_REAL_LIFECYCLE_SMOKE: '1', DGOS_PROJECT_ACCEPTANCE_PHASE: 'phase6-local',
        DGOS_CODEX_REAL_MODE: desktopMode,
        // Intentionally nonexistent: the acceptance harness owns creating it.
        DGOS_PROJECT_ACCEPTANCE_ROOT: join(root, 'desktop'),
        DGOS_STAGE6_CODEX_URL: provider.base_url, DGOS_STAGE6_CODEX_KEY: secret, DGOS_STAGE6_CODEX_MODEL: modelName,
      });
      command = [cargo, 'test', '--offline', '-p', 'dgos', '--features', 'webview-smoke', '--test', 'webview_smoke_agents', '--', '--nocapture'];
    }
    console.log(`real-codex mode=${mode === 'desktop' ? desktopMode : mode} isolated_root=${root} evidence_kind=real_provider`);
    failureClass = 'acceptance_launch_failed';
    // Own a fresh process group, not a persisted PID or name-based kill target.
    // Never relay raw cargo/model/native output: even key-redacted text can carry
    // project content, account URLs or approval payloads.
    const code = await runIsolated(command, repository, childEnv);
    process.exitCode = code;
    if (code !== 0) console.error(`real-codex acceptance_failed exit=${code} isolated_root=${root}`);
  }
} catch (error) {
  if (error instanceof SavedProviderError) failureClass = error.code;
  // Do not print error objects, subprocess output, SQL rows, command/env or keys.
  console.error(`real-codex failed: ${failureClass}${root ? `; isolated_root=${root}` : ''}`);
  if (failureClass === 'invalid_arguments') console.error(usage);
  process.exitCode = 1;
} finally {
  if (childEnv) for (const name of Object.keys(childEnv)) if (name.endsWith('_KEY')) delete childEnv[name];
  secret = undefined;
}
}

type Provider = {
  id: string; user_id: string; name: string; provider_type: string;
  base_url: string; credential_ref: string | null; auth_style: unknown;
};


export async function readSavedAcceptanceProvider(engine: 'codex' | 'claude') {
  const [providerName, modelName, providerType] = engine === 'codex'
    ? ['NextGpt', 'gpt-6-astra', 'openai_compatible']
    : ['Kiro', 'claude-sonnet-4-6', 'anthropic'];
  let phase = 'macos_keychain_required';
  try {
    if (process.platform !== 'darwin') fail(phase);
    phase = 'provider_metadata_unavailable';
    const database = new Database(join(homedir(), 'Library/Application Support/com.dgos.app/dgos.db'), { readonly: true, strict: true });
    let provider: Provider;
    try {
      // Never read api_key, whole config objects, or whole provider rows.
      const matches = database.query<Provider, [string]>(`SELECT id, user_id, name, provider_type, base_url, credential_ref,
        json_extract(config, '$.auth_style') AS auth_style
        FROM providers WHERE name = ? COLLATE BINARY AND enabled = 1 LIMIT 2`).all(providerName);
      phase = 'enabled_provider_name_must_be_unique';
      if (matches.length !== 1) fail(phase);
      provider = matches[0];
      phase = 'selected_model_unavailable_or_ambiguous';
      const models = database.query<{ model_id: string }, [string, string]>(`SELECT model_id FROM models
        WHERE provider_id = ? AND model_id = ? COLLATE BINARY AND is_available = 1 LIMIT 2`).all(provider.id, modelName);
      if (models.length !== 1) fail(phase);
    } finally { database.close(); }
    phase = 'provider_protocol_incompatible';
    const authStyles = engine === 'codex' ? ['bearer'] : ['bearer', 'x-api-key', 'both'];
    if (provider.provider_type !== providerType || (provider.auth_style != null && !authStyles.includes(provider.auth_style as string))) fail(phase);
    phase = 'provider_endpoint_invalid';
    // Reject unsafe syntax instead of silently normalizing an external route.
    if (!/^https:\/\//i.test(provider.base_url) || /[\s\\\u0000-\u001f\u007f]/.test(provider.base_url)) fail(phase);
    const endpoint = new URL(provider.base_url);
    if (endpoint.protocol !== 'https:' || !endpoint.hostname || endpoint.username || endpoint.password || endpoint.search || endpoint.hash) fail(phase);
    phase = 'provider_credential_reference_missing';
    if (!provider.credential_ref || !provider.user_id || !provider.id) fail(phase);
    // Exactly ProviderCredentialScope::account, using original persisted strings.
    const scope = JSON.stringify([provider.user_id, provider.id, provider.base_url, provider.credential_ref]);
    const account = `provider:${createHash('sha256').update(scope, 'utf8').digest('hex')}`;
    phase = 'keychain_credential_unavailable';
    let key = await keychain('com.dgos.app.providers.dev', account);
    if (key === undefined) key = await keychain('com.dgos.app.providers', account);
    phase = 'keychain_credential_missing';
    if (key === undefined) fail(phase);
    phase = 'keychain_credential_invalid';
    if (!key || key.length > 8192 || /[^\x21-\x7e]/.test(key)) fail(phase);
    return { provider, endpoint, key, model: modelName };
  } catch { throw new SavedProviderError(phase); }
}

async function keychain(service: string, account: string): Promise<string | undefined> {
  // stderr is intentionally discarded: Security diagnostics are not safe logs.
  const child = Bun.spawn(['/usr/bin/security', 'find-generic-password', '-s', service, '-a', account, '-w'], {
    env: { PATH: '/usr/bin:/bin:/usr/sbin:/sbin' }, stdin: 'ignore', stdout: 'pipe', stderr: 'ignore',
  });
  let timedOut = false;
  const timer = setTimeout(() => { timedOut = true; child.kill('SIGKILL'); }, 10_000);
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    const reader = child.stdout.getReader();
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.length;
      if (size > 8194) { child.kill('SIGKILL'); fail('keychain_output_invalid'); }
      chunks.push(value);
    }
    const code = await child.exited;
    if (timedOut) fail('keychain_timeout');
    // security maps errSecItemNotFound (-25300) to exit status 44. Only that
    // result permits retrying the identical scope in the release namespace.
    if (code === 44) return undefined;
    if (code !== 0) fail('keychain_unavailable');
    return Buffer.concat(chunks).toString('utf8').trim();
  } finally {
    clearTimeout(timer);
    if (child.exitCode === null) child.kill('SIGKILL');
    await child.exited;
    for (const chunk of chunks) chunk.fill(0);
  }
}

export async function runIsolated(command: string[], cwd: string, env: Record<string, string>): Promise<number> {
  const child = spawn(command[0], command.slice(1), { cwd, env, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdoutBytes = 0;
  let stderrBytes = 0;
  let interrupted = false;
  let timedOut = false;
  let cleanupFailed = false;
  let force: NodeJS.Timeout | undefined;
  const signalGroup = (signal: NodeJS.Signals) => {
    if (!child.pid) return;
    try { process.kill(-child.pid, signal); } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ESRCH') cleanupFailed = true;
    }
  };
  const stop = () => {
    interrupted = true;
    signalGroup('SIGTERM');
    force ??= setTimeout(() => signalGroup('SIGKILL'), 5_000);
  };
  const deadline = setTimeout(() => { timedOut = true; stop(); }, 30 * 60_000);
  child.stdout.on('data', (data: Buffer) => { stdoutBytes += data.length; });
  child.stderr.on('data', (data: Buffer) => { stderrBytes += data.length; });
  child.stdout.on('error', stop);
  child.stderr.on('error', stop);
  process.on('SIGINT', stop);
  process.on('SIGTERM', stop);
  try {
    const { promise, resolve, reject } = Promise.withResolvers<number>();
    child.once('error', () => reject(new Error('isolated_child_failed')));
    child.once('close', (exitCode, signal) => resolve(exitCode ?? (signal ? 128 : 1)));
    const code = await promise;
    // The harness normally shuts down its owned native sessions. Sweep only our
    // freshly-created group on every exit as protection against panic/failure.
    signalGroup('SIGTERM');
    await delay(250);
    signalGroup('SIGKILL');
    console.log(`real-codex child_exit=${code} interrupted=${interrupted} timed_out=${timedOut} cleanup_failed=${cleanupFailed} stdout_bytes=${stdoutBytes} stderr_bytes=${stderrBytes} raw_output=discarded`);
    return interrupted || timedOut || cleanupFailed ? 1 : code;
  } finally {
    clearTimeout(deadline);
    clearTimeout(force);
    process.off('SIGINT', stop);
    process.off('SIGTERM', stop);
    // Spawn/read errors must not leave cargo, the App or app-server running.
    signalGroup('SIGKILL');
  }
}
