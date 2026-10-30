import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { checkReactVersions } from './react-version.mjs';

// Static contract checks supplement compilation; they do not prove WebView isolation.
const files = ['provider_commands', 'skills', 'mcp', 'workbench', 'modules', 'module_host', 'appearance'];
// D046: the module host bridge serves the page's own lifecycle. Its only
// parameters are the Tauri context, the instruction channel, and the
// host-issued attachment id, ticket and receipt. No identity, module,
// version, digest, path, URL or permission parameter may ever appear.
const bridgeAllowed = new Set(['app', 'window', 'on_instruction', 'attachment_id', 'ticket', 'receipt']);
// D051 / ADR-0001: one typed command per capability. Each parameter list is an exact allow-list
// (the host-issued attachment and route plus that capability's own business fields). A command
// named `module_call_*` that is not listed here fails the check, so a generic
// `module_call(capability, payload)` can never be added by accident. Identity, runtime, version,
// digest, path, URL, permission and token parameters are rejected by the allow-list itself.
const callCommands = new Map([
  ['module_call_provider', new Set(['app', 'window', 'attachment_id', 'route', 'call_id', 'provider_id', 'model_id', 'request'])],
]);
// D055: a page-reported crash. Same host-derived identity as the call commands: the runtime comes
// from the route, never from a module id.
const recordCommands = new Map([
  ['report_module_crash', new Set(['app', 'window', 'attachment_id', 'route', 'call_id', 'source', 'error_name', 'stack'])],
]);
// D034: lifecycle commands may take a `module_id` selector and an
// `operation_id`, but never identity, handles, versions, digests, paths or URLs.
// D055: management commands. Lifecycle commands take a `module_id` selector and an `operation_id`;
// only grant/revoke also take a D006 permission string (the host re-reads the manifest). Nothing
// else - no identity, handle, version, digest, path, URL or permission list - may appear.
const lifecycleParams = new Set(['app', 'window', 'module_id', 'operation_id']);
const permissionCommands = new Set(['grant_module_permission', 'revoke_module_permission']);
const permissionParams = new Set([...lifecycleParams, 'permission']);
// D065: appearance preferences. Exact allow-list: the Tauri context and, for the write, the patch
// object. No user id, settings key or value parameter may appear, so the renderer can neither pick
// a user nor reach any other setting.
const appearanceParams = new Map([
  ['get_appearance_preferences', new Set(['app', 'window'])],
  ['set_appearance_preferences', new Set(['app', 'window', 'patch'])],
]);
// FR-001 §6.3: exact parameter lists of the provider commands added or extended by the third-party
// compatible protocol extension. `probe_model` is an optional model name for services without a
// model list endpoint; identity never appears. A new parameter must be added here deliberately.
const providerExactParams = new Map([
  ['list_provider_vendors', ['app', 'window']],
  ['add_manual_model', ['app', 'window', 'provider_id', 'model_id', 'kind']],
  ['remove_manual_model', ['app', 'window', 'provider_id', 'model_id']],
  ['call_provider_stream', ['app', 'window', 'provider_id', 'model_id', 'request', 'on_event']],
  ['cancel_provider_stream', ['app', 'window', 'stream_id']],
  ['test_provider_connection', ['app', 'window', 'config', 'probe_model']],
  ['test_provider_by_id', ['app', 'window', 'provider_id', 'probe_model']],
  ['create_provider', ['app', 'window', 'config', 'probe_model']],
  ['create_provider_with_models', ['app', 'window', 'config', 'models', 'probe_model']],
]);
let commands = 0;
for (const file of files) {
  const source = readFileSync(new URL(`../src-tauri/src/commands/${file}.rs`, import.meta.url), 'utf8');
  const handlers = [...source.matchAll(/#\[tauri::command\]\s+pub async fn (\w+)\(([\s\S]*?)\) -> [\s\S]*?\{([\s\S]*?)(?=\n#\[|\npub struct |\n#\[cfg|$)/g)];
  assert.ok(handlers.length, `${file}: no command handlers checked`);
  for (const [, name, args, body] of handlers) {
    assert.match(args, /window:\s*WebviewWindow/, `${name}: host window context is required`);
    if (file === 'module_host') {
      const names = [...args.matchAll(/\b(\w+)\s*:(?!:)/g)].map((match) => match[1]);
      const allowed = callCommands.get(name) ?? recordCommands.get(name) ?? (name.startsWith('module_call') ? null : bridgeAllowed);
      assert.ok(allowed, `${name}: module call commands must be registered in callCommands (no generic forwarding)`);
      for (const param of names) {
        assert.ok(allowed.has(param), `${name}: bridge command parameter \`${param}\` is not allowed`);
      }
      assert.match(body, /host::module_host_window\(\s*&window\s*\)/, `${name}: missing bridge window gate`);
      if (callCommands.has(name)) {
        // Channel admission must come before the capability runs, and the capability must be the
        // dispatch service that authorizes on every call (D050).
        const admit = body.search(/\.admit(?:_once)?\(/);
        const dispatch = body.indexOf('dispatch');
        assert.ok(admit >= 0 && dispatch > admit, `${name}: channel admission must precede the dispatch service`);
        assert.match(body, /ModuleCapabilityDispatchService/, `${name}: must call through ModuleCapabilityDispatchService`);
      }
      if (recordCommands.has(name)) {
        // The runtime comes from the host-issued route, resolved before anything is recorded (D055).
        const resolve = body.search(/\.resolve_once\(/);
        const record = body.indexOf('ModuleCrashService');
        assert.ok(resolve >= 0 && record > resolve, `${name}: the route must be resolved before the crash service is used`);
      }
    } else if (file === 'appearance') {
      const names = [...args.matchAll(/\b(\w+)\s*:(?!:)/g)].map((match) => match[1]);
      const allowed = appearanceParams.get(name);
      assert.ok(allowed, `${name}: appearance commands must be registered in appearanceParams`);
      for (const param of names) {
        assert.ok(allowed.has(param), `${name}: appearance command parameter \`${param}\` is not allowed`);
      }
      assert.equal(names.length, allowed.size, `${name}: every allowed parameter must be present`);
      assert.match(body, /host::context\(\s*&window,/, `${name}: missing host entry gate`);
    } else if (file === 'modules') {
      const names = [...args.matchAll(/\b(\w+)\s*:(?!:)/g)].map((match) => match[1]);
      const allowed = permissionCommands.has(name) ? permissionParams : lifecycleParams;
      for (const param of names) {
        assert.ok(allowed.has(param), `${name}: module command parameter \`${param}\` is not allowed`);
      }
      assert.match(body, /host::module_session\(\s*&window,/, `${name}: missing host entry gate`);
    } else {
      assert.doesNotMatch(args, /\b(?:user_id|module_id|caller)\s*:/, `${name}: identity belongs to the host`);
      assert.match(body, /host::context\(\s*&window,/, `${name}: missing host entry gate`);
      const exact = providerExactParams.get(name);
      if (file === 'provider_commands' && exact) {
        const names = [...args.matchAll(/\b(\w+)\s*:(?!:)/g)].map((match) => match[1]);
        assert.deepEqual([...names].sort(), [...exact].sort(), `${name}: parameter list must be exactly ${[...exact].join(', ')}`);
      }
      if (name === 'call_provider_stream') {
        // The stream reaches only the Channel the caller passed; a global event would be readable
        // by every page in the realm (D042). Caller and request id come from the host context.
        assert.match(args, /on_event:\s*Channel<ProviderStreamEvent>/, `${name}: events must go to the caller's Channel`);
        assert.doesNotMatch(body, /\.emit(?:_to|_filter)?\(|emit_all/, `${name}: must not use global events`);
        assert.match(body, /execute_stream\(\s*&context,/, `${name}: must run through ProviderCallService with the host context`);
        assert.match(body, /register\(window\.label\(\)\)/, `${name}: the stream must be owned by the calling window`);
      }
      if (name === 'cancel_provider_stream') {
        assert.match(body, /\.cancel\(window\.label\(\),/, `${name}: may cancel only streams of the calling window`);
      }
    }
    commands += 1;
  }
}
// D046: the bridge gate is window label + origin only. It must not inherit
// the caller/capability policy of `context` (Settings/ModulesRead), so a
// change to that policy can neither open nor close the page lifecycle channel.
const hostSource = readFileSync(new URL('../src-tauri/src/host.rs', import.meta.url), 'utf8');
const gate = hostSource.match(/pub fn module_host_window\([^)]*\)[^{]*\{([\s\S]*?)\n\}/);
assert.ok(gate, 'host.rs: module_host_window not found');
assert.doesNotMatch(gate[1], /\bcontext\s*\(|BuiltinCaller|Capability|allows\s*\(|is_settings_route/, 'module_host_window must not depend on caller, capability or route policy');
assert.match(gate[1], /trusted_main_window\(/, 'module_host_window must use the shared main-window/origin predicate');
const registered = readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8')
  .match(/commands::\w+,/g) ?? [];
assert.equal(commands, registered.length, 'all registered IPC handlers must be inspected');
// D060: installed React/ReactDOM must equal the lockfile and satisfy package.json.
const reactProblems = checkReactVersions(fileURLToPath(new URL('..', import.meta.url)));
assert.deepEqual(reactProblems, [], `React version contract failed:\n  ${reactProblems.join('\n  ')}`);
console.log(`Checked ${commands} host IPC contracts (static only); react and react-dom match package-lock.json.`);
