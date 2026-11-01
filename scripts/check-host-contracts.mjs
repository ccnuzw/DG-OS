import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { checkReactVersions } from './react-version.mjs';

// Static contract checks supplement compilation; they do not prove WebView isolation.
const files = ['provider_commands', 'skills', 'mcp', 'workbench', 'chat', 'modules', 'module_host', 'module_capabilities', 'appearance', 'accounts', 'lan'];
// FR-010 / interface contract §14.8: the eight LAN host commands. Exact parameter lists; no user
// id, caller, session token or access code. Every one enters through the Settings caller with
// `Capability::LanManage`, which `allows` grants to Settings only.
const lanParams = new Map([
  ['get_lan_status', ['app', 'window']],
  ['set_lan_enabled', ['app', 'window', 'enabled']],
  ['set_lan_port', ['app', 'window', 'port']],
  ['issue_lan_access_code', ['app', 'window']],
  ['list_lan_devices', ['app', 'window']],
  ['disconnect_lan_device', ['app', 'window', 'session_id']],
  ['disconnect_all_lan_devices', ['app', 'window']],
  ['list_lan_audit_events', ['app', 'window', 'input']],
]);
// FR-008 / interface contract §9.5: the six account commands. Exact parameter lists; no user id,
// session token or account id. `get_account_state` and `sign_out` take only the Tauri context; the
// rest take one typed `input`.
const accountParams = new Map([
  ['get_account_state', ['app', 'window']],
  ['sign_out', ['app', 'window']],
  ['create_account', ['app', 'window', 'input']],
  ['sign_in', ['app', 'window', 'input']],
  ['change_password', ['app', 'window', 'input']],
  ['delete_account', ['app', 'window', 'input']],
]);
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
// D093: the five non-Provider capabilities, one typed command each, in `module_capabilities.rs`.
// Same exact allow-list rule; every one must admit through the channel before the dispatch service.
const capabilityCommands = new Map([
  ['module_storage_get', new Set(['app', 'window', 'attachment_id', 'route', 'call_id', 'key'])],
  ['module_storage_set', new Set(['app', 'window', 'attachment_id', 'route', 'call_id', 'key', 'value'])],
  ['module_storage_delete', new Set(['app', 'window', 'attachment_id', 'route', 'call_id', 'key'])],
  ['module_storage_list', new Set(['app', 'window', 'attachment_id', 'route', 'call_id'])],
  ['module_network_request', new Set(['app', 'window', 'attachment_id', 'route', 'call_id', 'request'])],
  ['module_call_mcp_tool', new Set(['app', 'window', 'attachment_id', 'route', 'call_id', 'server_id', 'tool_name', 'arguments'])],
  ['module_prepare_skill', new Set(['app', 'window', 'attachment_id', 'route', 'call_id', 'skill_id'])],
]);
// D093: the user's answer to a module-initiated MCP confirmation. Settings-route gate; the only
// inputs are the host-issued confirmation id and the decision.
const confirmationCommands = new Map([
  ['resolve_module_mcp_confirmation', new Set(['app', 'window', 'confirmation_id', 'approved'])],
]);
// D055: a page-reported crash. Same host-derived identity as the call commands: the runtime comes
// from the route, never from a module id.
const recordCommands = new Map([
  ['report_module_crash', new Set(['app', 'window', 'attachment_id', 'route', 'call_id', 'source', 'error_name', 'stack'])],
]);
// D113: cancel one in-flight call of a route. Same host-derived identity as the call commands (the
// route binding, never a module id or user), an exact parameter list, and the router alone: no
// dispatch service, no admission, no authorization, because a cancel runs and grants nothing.
const cancelCommands = new Map([
  ['cancel_module_call', new Set(['app', 'window', 'attachment_id', 'route', 'call_id'])],
]);
// D034: lifecycle commands may take a `module_id` selector and an
// `operation_id`, but never identity, handles, versions, digests, paths or URLs.
// D055: management commands. Lifecycle commands take a `module_id` selector and an `operation_id`;
// only grant/revoke also take a D006 permission string (the host re-reads the manifest). Nothing
// else - no identity, handle, version, digest, path, URL or permission list - may appear.
const lifecycleParams = new Set(['app', 'window', 'module_id', 'operation_id']);
const permissionCommands = new Set(['grant_module_permission', 'revoke_module_permission']);
const permissionParams = new Set([...lifecycleParams, 'permission']);
// D093: the permission audit query. Optional module selector plus an opaque cursor and page size;
// the user comes from the host session.
const auditParams = new Set(['app', 'window', 'module_id', 'before', 'limit']);
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
// FR-002 Batch B: source parsing and install confirmation. Identity is host
// derived; the renderer can supply only the source text or opaque operation id.
const skillExactParams = new Map([
  ['parse_skill_source', ['window', 'input']],
  ['preview_skill_install', ['window', 'input', 'state']],
  ['preview_skill_package', ['window', 'bytes', 'state']],
  ['confirm_skill_install', ['window', 'operation_id', 'state']],
  ['cancel_skill_install', ['window', 'operation_id', 'state']],
]);
// FR-006 / interface contract §10.1: exact parameter lists of the chat commands. Each takes one typed
// `input` (deny_unknown_fields) and reads managed state from the AppHandle; identity comes from
// `host::context`, never from a parameter. Streaming commands take the caller's Channel (`on_event`).
const chatExactParams = new Map([
  ['create_chat_session', ['app', 'window', 'input']],
  ['list_chat_sessions', ['app', 'window', 'input']],
  ['rename_chat_session', ['app', 'window', 'input']],
  ['update_chat_session_target', ['app', 'window', 'input']],
  ['delete_chat_session', ['app', 'window', 'input']],
  ['search_chat_sessions', ['app', 'window', 'input']],
  ['list_chat_messages', ['app', 'window', 'input']],
  ['send_chat_message', ['app', 'window', 'input', 'on_event']],
  ['regenerate_chat_message', ['app', 'window', 'input', 'on_event']],
  ['advance_chat_tool_turn', ['app', 'window', 'input']],
  ['cancel_chat_generation', ['app', 'window', 'task_id']],
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
      const allowed = callCommands.get(name) ?? recordCommands.get(name) ?? cancelCommands.get(name) ?? (name.startsWith('module_call') ? null : bridgeAllowed);
      assert.ok(allowed, `${name}: module call commands must be registered in callCommands (no generic forwarding)`);
      for (const param of names) {
        assert.ok(allowed.has(param), `${name}: bridge command parameter \`${param}\` is not allowed`);
      }
      assert.match(body, /host::module_host_window\(\s*&window\s*\)/, `${name}: missing bridge window gate`);
      if (cancelCommands.has(name)) {
        assert.deepEqual([...names].sort(), [...allowed].sort(), `${name}: parameter list must be exactly ${[...allowed].join(', ')}`);
        const gate = body.search(/host::module_host_window\(\s*&window\s*\)/);
        const cancel = body.search(/\.cancel\(\s*&route,\s*&attachment_id,\s*&call_id\s*\)/);
        assert.ok(gate >= 0 && cancel > gate, `${name}: the window gate must precede ModuleCallRouter::cancel`);
        assert.doesNotMatch(body, /dispatch|ModuleCapabilityDispatchService|\.admit(?:_once)?\(|authorize|resolve_once|begin_request/i, `${name}: a cancel must not dispatch, admit or authorize`);
      }
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
    } else if (file === 'module_capabilities') {
      const names = [...args.matchAll(/\b(\w+)\s*:(?!:)/g)].map((match) => match[1]);
      const allowed = capabilityCommands.get(name) ?? confirmationCommands.get(name);
      assert.ok(allowed, `${name}: capability commands must be registered in capabilityCommands (no generic forwarding)`);
      assert.deepEqual([...names].sort(), [...allowed].sort(), `${name}: parameter list must be exactly ${[...allowed].join(', ')}`);
      if (capabilityCommands.has(name)) {
        // `admit` is the window gate plus `ModuleCallRouter::admit_once` (checked below).
        const admitted = body.search(/\badmit\(\s*&app,\s*&window,/);
        const dispatch = body.indexOf('ModuleCapabilityDispatchService');
        assert.ok(admitted >= 0 && dispatch > admitted, `${name}: channel admission must precede the dispatch service`);
        assert.match(body, /\.(?:run_bound|authorize_bound)\(\s*call,/, `${name}: must authorize the admitted call on every call`);
      } else {
        // D093: the dialog and the asking module survive SPA navigation, so the answer is gated on the
        // trusted main window on every route; the user is host-derived and the id is bound to it.
        assert.match(body, /host::module_confirmation_session\(\s*&window\s*\)/, `${name}: missing confirmation window gate`);
        assert.doesNotMatch(body, /host::module_session\(/, `${name}: the confirmation gate must not require the Settings route`);
        assert.match(body, /\.resolve\(\s*session\.user_id\(\),\s*&confirmation_id,/, `${name}: the id must be resolved for the host-derived user`);
      }
    } else if (file === 'accounts') {
      const names = [...args.matchAll(/\b(\w+)\s*:(?!:)/g)].map((match) => match[1]);
      const exact = accountParams.get(name);
      assert.ok(exact, `${name}: account commands must be registered in accountParams`);
      assert.deepEqual([...names].sort(), [...exact].sort(), `${name}: parameter list must be exactly ${exact.join(', ')}`);
      if (exact.includes('input')) {
        assert.match(args, /input:\s*\w+Input\b/, `${name}: input must be a typed *Input struct`);
      }
      // D090: the window gate first; never the session/caller gate, which the sign-in page cannot pass.
      const gateAt = body.search(/host::account_window\(\s*&window\s*\)/);
      assert.ok(gateAt >= 0, `${name}: missing host::account_window gate`);
      assert.ok(gateAt < body.indexOf('accounts(&app)'), `${name}: the window gate must come before the account service`);
      assert.doesNotMatch(body, /host::(?:context|module_session)\(/, `${name}: account commands do not use the session gate`);
    } else if (file === 'lan') {
      const names = [...args.matchAll(/\b(\w+)\s*:(?!:)/g)].map((match) => match[1]);
      const exact = lanParams.get(name);
      assert.ok(exact, `${name}: LAN commands must be registered in lanParams`);
      assert.deepEqual([...names].sort(), [...exact].sort(), `${name}: parameter list must be exactly ${exact.join(', ')}`);
      const gateAt = body.search(/host::context\(\s*&window,\s*BuiltinCaller::Settings,\s*Capability::LanManage\s*\)/);
      assert.ok(gateAt >= 0, `${name}: LAN commands use the Settings caller and LanManage`);
      assert.ok(gateAt < body.indexOf('lan(&app)'), `${name}: the gate must come before the LAN service`);
      if (exact.includes('input')) {
        assert.match(args, /input:\s*\w+Input\b/, `${name}: input must be a typed *Input struct`);
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
      const allowed = permissionCommands.has(name)
        ? permissionParams
        : name === 'list_module_permission_audits'
          ? auditParams
          : lifecycleParams;
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
      if (file === 'chat') {
        const names = [...args.matchAll(/\b(\w+)\s*:(?!:)/g)].map((match) => match[1]);
        const exact = chatExactParams.get(name);
        assert.ok(exact, `${name}: chat commands must be registered in chatExactParams`);
        assert.deepEqual([...names].sort(), [...exact].sort(), `${name}: parameter list must be exactly ${exact.join(', ')}`);
        assert.match(body, /host::context\(\s*&window,\s*BuiltinCaller::Workbench,\s*Capability::TasksRun/, `${name}: chat commands use the Workbench caller and TasksRun`);
        assert.doesNotMatch(body, /\.emit(?:_to|_filter)?\(|emit_all/, `${name}: chat must not use global events`);
        if (exact.includes('on_event')) {
          assert.match(args, /on_event:\s*Channel<ChatEvent>/, `${name}: events must go to the caller's Channel`);
          assert.match(body, /window\.label\(\)/, `${name}: the generation must be owned by the calling window`);
        }
        if (exact.includes('input')) {
          assert.match(args, /input:\s*\w+Input/, `${name}: input must be a typed deny_unknown_fields struct`);
        }
      }
      if (file === 'skills' && skillExactParams.has(name)) {
        const names = [...args.matchAll(/\b(\w+)\s*:(?!:)/g)].map((match) => match[1]);
        const expected = skillExactParams.get(name);
        assert.deepEqual([...names].sort(), [...expected].sort(), `${name}: parameter list must be exactly ${expected.join(', ')}`);
        assert.match(body, /host::context\(\s*&window,\s*BuiltinCaller::Settings,\s*Capability::SkillsManage\s*\)/, `${name}: Skills Batch B commands use the Settings SkillsManage gate`);
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
// D093: the MCP confirmation gate is the bridge window gate plus the host-derived session, with no route policy.
const confirmationGate = hostSource.match(/pub fn module_confirmation_session\([^)]*\)[^{]*\{([\s\S]*?)\n\}/);
assert.ok(confirmationGate, 'host.rs: module_confirmation_session not found');
assert.match(confirmationGate[1], /module_host_window\(window\)\?/, 'module_confirmation_session must use the main-window/origin gate');
assert.doesNotMatch(confirmationGate[1], /\bcontext\s*\(|BuiltinCaller|Capability|allows\s*\(|is_settings_route/, 'module_confirmation_session must not depend on caller, capability or route policy');
// FR-008 / contract §9.5: the account gate is the main-window/origin predicate only.
const accountGate = hostSource.match(/pub fn account_window\([^)]*\)[^{]*\{([\s\S]*?)\n\}/);
assert.ok(accountGate, 'host.rs: account_window not found');
assert.match(accountGate[1], /trusted_main_window\(/, 'account_window must use the shared main-window/origin predicate');
assert.doesNotMatch(accountGate[1], /\bcontext\s*\(|BuiltinCaller|Capability|allows\s*\(|is_settings_route|resolved_session|SessionResolver/, 'account_window must not depend on session, caller, capability or route policy');
// FR-008: identity comes only from the session resolver. `context` resolves it; the pre-account
// constant exists only for tests and smoke fixtures.
const contextGate = hostSource.match(/pub fn context\([^)]*\)[^{]*\{([\s\S]*?)\n\}/);
assert.ok(contextGate, 'host.rs: context not found');
assert.match(contextGate[1], /resolved_session\(window\)\?/, 'context must resolve the user through the session resolver');
assert.doesNotMatch(contextGate[1], /LOCAL_USER|DEFAULT_USER_ID|local_default/, 'context must not return a fixed user');
assert.match(hostSource, /#\[cfg\(any\(test, feature = "webview-smoke"\)\)\]\s*pub const LOCAL_USER/, 'LOCAL_USER must exist only for tests and smoke fixtures');
// Production code never builds the pre-account session directly; only test modules may.
const productionSources = [
  ...['host', 'lib', 'session', 'module_host', 'capabilities'].map((name) => `../src-tauri/src/${name}.rs`),
  ...[...files, 'mod'].map((name) => `../src-tauri/src/commands/${name}.rs`),
  ...['lib', 'accounts', 'module_crashes', 'module_management', 'module_mcp', 'module_network', 'module_operations', 'module_storage']
    .map((name) => `../crates/dgos-application/src/${name}.rs`),
];
for (const path of productionSources) {
  const source = readFileSync(new URL(path, import.meta.url), 'utf8');
  const production = source.split(/\n#\[cfg\(test\)\]/)[0];
  assert.doesNotMatch(production, /local_default\(/, `${path}: production code must resolve the session, not use HostSession::local_default()`);
}
// FR-010 / contract §14.8: `LanManage` is granted to the Settings caller only.
const allowsBody = hostSource.match(/fn allows\([^)]*\)[^{]*\{([\s\S]*?)\n\}/);
assert.ok(allowsBody, 'host.rs: allows not found');
const workbenchArm = allowsBody[1].match(/BuiltinCaller::Workbench => matches!\(([\s\S]*?)\),/);
assert.ok(workbenchArm && !/LanManage/.test(workbenchArm[1]), 'LanManage must not be granted to the Workbench caller');
assert.match(allowsBody[1], /BuiltinCaller::Module => false/, 'module contexts get no host capability');
// FR-010 / contract §14.4, §14.9: the LAN routes are exactly the whitelist; route code reaches chat
// only through `ChatBackend` and never names an account, module, MCP or management service; there
// is no "command name + JSON" forwarding.
const lanSource = (name) => readFileSync(new URL(`../crates/dgos-lan/src/${name}.rs`, import.meta.url), 'utf8');
const lanRouteCode = ['http', 'ws', 'core', 'server', 'service'].map((name) => [name, lanSource(name)]);
for (const [name, source] of lanRouteCode) {
  assert.doesNotMatch(
    source,
    /\b(?:AccountService|SessionPort|dgos_application|dgos_mcp|McpManager|dgos_modules|Module\w*(?:Service|Manager)|ProviderManager|SkillsManager|tauri::|invoke|generate_handler)\b/,
    `dgos-lan/${name}.rs: LAN route code must not reference account, module, MCP or management services`,
  );
  assert.doesNotMatch(source, /accept_destination:\s*Some|tools:\s*vec!\[\s*\S/, `dgos-lan/${name}.rs: remote chat never accepts a destination or selects tools`);
}
const routes = [...lanSource('http').matchAll(/\.route\(\s*"([^"]+)",([\s\S]*?)(?=\n\s*\.(?:route|fallback)\()/g)]
  .map(([, path, methods]) => `${[...methods.matchAll(/\b(get|post|put|delete|patch)\(/g)].map((m) => m[1].toUpperCase()).sort().join('|')} ${path}`)
  .sort();
assert.deepEqual(routes, [
  'GET /api/v1/lan/session',
  'GET /api/v1/lan/stream',
  'GET /api/v1/lan/targets',
  'GET|POST /api/v1/lan/chat/sessions',
  'GET|POST /api/v1/lan/chat/sessions/{session_id}/messages',
  'POST /api/v1/lan/chat/messages/{message_id}/regenerate',
  'POST /api/v1/lan/chat/tasks/{task_id}/cancel',
  'POST /api/v1/lan/logout',
  'POST /api/v1/lan/pair',
].sort(), 'dgos-lan/http.rs: LAN routes must be exactly the D099 whitelist');
assert.doesNotMatch(lanSource('http'), /access-control-|CorsLayer|ACCESS_CONTROL/i, 'dgos-lan/http.rs: no CORS headers');
// The chat adapter uses Provider/Skills managers only through their list/read methods.
const chatPort = lanSource('chat_port');
const managerCalls = [...chatPort.matchAll(/\b(?:providers|skills)\b(?:\(\))?\s*\.\s*(\w+)\(/g)].map((m) => m[1]);
for (const call of managerCalls) {
  assert.ok(['list_providers', 'list_models', 'list_installed_skills', 'read'].includes(call), `dgos-lan/chat_port.rs: manager call \`${call}\` is not a read`);
}
assert.doesNotMatch(chatPort, /\b(?:dgos_mcp|McpManager|send_with_mcp|run_tool_turn|update_target|rename_session|delete_session|toggle|install|create_provider|update_provider|delete_provider)\b/, 'dgos-lan/chat_port.rs: no MCP, tool turns or mutations');
const lanManifest = readFileSync(new URL('../crates/dgos-lan/Cargo.toml', import.meta.url), 'utf8').split('[dev-dependencies]')[0];
assert.doesNotMatch(lanManifest, /dgos-(?:application|mcp|modules)\b/, 'dgos-lan must not depend on the account, MCP or module crates');
// D093: the shared admission helper of the capability commands gates the window and admits once.
const capabilitySource = readFileSync(new URL('../src-tauri/src/commands/module_capabilities.rs', import.meta.url), 'utf8');
const admitHelper = capabilitySource.match(/\nfn admit\([\s\S]*?\n\}/);
assert.ok(admitHelper, 'module_capabilities.rs: admit helper not found');
assert.match(admitHelper[0], /host::module_host_window\(window\)\?/, 'admit must gate the window first');
assert.match(admitHelper[0], /\.admit_once\(route, attachment_id, call_id\)/, 'admit must use ModuleCallRouter::admit_once');
const registered = readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8')
  .match(/commands::\w+,/g) ?? [];
assert.equal(commands, registered.length, 'all registered IPC handlers must be inspected');
// D060: installed React/ReactDOM must equal the lockfile and satisfy package.json.
const reactProblems = checkReactVersions(fileURLToPath(new URL('..', import.meta.url)));
assert.deepEqual(reactProblems, [], `React version contract failed:\n  ${reactProblems.join('\n  ')}`);
console.log(`Checked ${commands} host IPC contracts (static only); react and react-dom match package-lock.json.`);
