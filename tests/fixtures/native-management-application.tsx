import React, { useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';
import { Modal, ModalHost } from '@dgos/ui';
import NativeManager, { type NativeManagerHandle } from '../../src/components/nativeExtensions/NativeManager';
import NativePluginsDrawer from '../../src/components/nativeExtensions/NativePluginsDrawer';
import { useDevelopment } from '../../src/stores/developmentStore';
import '../../src/index.css';
import type { PreviewNativeVisibility, NativeSelection, NativeSkillRuleState } from '../../src/types/nativeExtensions';
import type { NativeMcpConfigProjection, NativeProbeInput, NativeProbeSnapshot } from '../../src/types/nativeMcpCatalog';

const skill = { instance: { id: 'fixture-skill-1', revision: 'fixture-revision' }, target: 'Codex', scope: 'OsUser', slug: 'sample-skill', owner: 'Loose', metadata: { name: 'Sample native skill', description: 'Isolated renderer fixture', license: null, dependencies: { declared: true, tools: [{ program: 'node', version_requirement: null }], packages: [], required_tools: [] }, dynamic_commands: false }, resources: [{ path: 'SKILL.md', hash: 'sample-hash', bytes: 128, executable: false }], source: { source_hash: 'sample-source-hash', commit: null, origin: null }, source_modified: false, can_update: true };
function ManagementSurface() {
  const [kind, setKind] = useState<'skills' | 'mcp' | null>(null);
  const manager = useRef<NativeManagerHandle>(null);
  const selection: NativeSelection = { target: 'Codex', scope: 'OsUser', project_id: null };
  return <div className="grid min-h-screen grid-cols-1 gap-4 p-4 md:grid-cols-[minmax(12rem,20rem)_minmax(0,1fr)]">
    <NativePluginsDrawer selection={selection} onManage={setKind} />
    <section className="min-w-0"><h1>Native management — isolated renderer fixture</h1><p>Production components with explicit IPC doubles. No filesystem, CLI, network or probe execution.</p><textarea className="w-full" aria-label="Conversation draft" defaultValue="Keep this draft while managing extensions" /></section>
    <Modal open={kind !== null} onOpenChange={open => { if (!open) void manager.current?.requestClose(); }} title={kind === 'skills' ? 'CLI Skills' : 'CLI MCP'} footer={<></>}>
      {kind && <NativeManager ref={manager} kind={kind} initialSelection={selection} onClose={() => setKind(null)} />}
    </Modal><ModalHost />
  </div>;
}
export function mountNativeManagement(host: HTMLElement) {
  useDevelopment.setState({ loaded: true, projects: [{ id: 'fixture-project-a', owner: 'fixture', name: 'Alpha Project', path: '/opaque/alpha', directory_identity: 'opaque-a', archived: false, created_at: 1, last_opened_at: null }, { id: 'fixture-project-b', owner: 'fixture', name: 'Beta Project', path: '/opaque/beta', directory_identity: 'opaque-b', archived: false, created_at: 1, last_opened_at: null }] });
  const savedRules = new Map<string, Map<string, NativeSkillRuleState>>();
  const previews = new Map<string, PreviewNativeVisibility>();
  let nextOperation = 0;
  const projection: NativeMcpConfigProjection = { name: 'sample-server', transport: 'Stdio', program: '/fixture/node', args: ['/fixture/server.js'], url: null, cwd: null, credential_references: [], redacted: false, config_digest: 'fixture-digest' };
  const registryServer = { name: 'io.example/fixture', description: 'Renderer-only MCP catalog entry', version: '1.0.0', remotes: [{ type: 'streamable-http', url: 'https://fixture.invalid/mcp' }] };
  const registryEntry = { server: registryServer, _meta: { 'io.modelcontextprotocol.registry/official': { status: 'active', isLatest: true } } };
  const probes = new Map<string, NativeProbeSnapshot>();
  const layerKey = (selection: NativeSelection) => JSON.stringify([selection.target, selection.scope, selection.project_id, selection.task_id ?? null]);
  mockIPC((command, args) => {
    const input = (args as { input: Record<string, unknown> }).input;
    if (command === 'discover_native_skills') return { instances: input.target === 'Codex' ? [skill] : [], readonly_entries: [], missing: false, incomplete: false };
    if (command === 'discover_native_skill_rules') {
      const selection = input as unknown as NativeSelection;
      return { rules: [...(savedRules.get(layerKey(selection)) ?? new Map())].map(([name, state]) => ({ name, state })), coverage: 'selected_settings_layer' };
    }
    if (command === 'get_native_skill') return skill;
    if (command === 'discover_native_mcp') return { instances: [{ native_key: 'sample-server', transport: 'Stdio', enabled: true, has_auth_configuration: false, unknown_fields_present: false, owner: 'Loose', authentication: { configured: false, credential_present: false, authenticated: false, connected: false, approved: null, active_session: null, login: 'Unsupported', logout: 'Unsupported', delete_credentials: 'Unsupported' } }], incomplete: false };
    if (command === 'list_native_sessions') return [];
    if (command === 'list_native_skill_sources') return [];
    if (command === 'search_native_skill_library') return { sources: [], results: [] };
    if (command === 'inspect_native_skill') return { skill, target_path: '/fixture/.agents/skills/sample-skill' };
    if (command === 'read_native_skill_file') return { instance: skill.instance, path: input.path, hash: 'fixture-hash', bytes: 70, text: '# Sample native skill\n\nRenderer-only content. No script is executed.', reason: null };
    if (command === 'inspect_native_skill_dependencies') return { declarations: skill.metadata.dependencies, tools: [{ declaration: skill.metadata.dependencies.tools[0], availability: 'FoundUnprobed', candidate: '/fixture/node', executable: true }], unknown: true };
    if (command === 'preview_native_skill_version') return { plan_id: 'fixture-version', executable: '/fixture/node', digest: 'fixture-binary', argv: ['--version'], cwd: '/fixture', expires_at: Math.floor(Date.now() / 1000) + 120, deadline_seconds: 5, environment: 'host_startup', requirement: null };
    if (command === 'run_native_skill_version' || command === 'inspect_native_skill_version') return { status: 'observed', output: 'fixture version 1', exit_code: 0, cleanup_confirmed: true, requirement_satisfied: null };
    if (command === 'project_native_mcp_config') return projection;
    if (command === 'search_native_mcp_registry') return { servers: [registryEntry], next_cursor: null, fetched_at: Math.floor(Date.now() / 1000), stale: false, error: null, source: 'https://registry.modelcontextprotocol.io/v0/servers' };
    if (command === 'resolve_native_mcp_registry') return registryEntry;
    if (command === 'preview_native_mcp_probe') {
      const request = input as unknown as NativeProbeInput, draft = request.source.kind === 'draft' ? request.source.draft : null;
      const config: NativeMcpConfigProjection = draft ? { name: draft.name, transport: draft.transport as NativeMcpConfigProjection['transport'], program: draft.transport === 'Stdio' ? draft.program : null, args: draft.transport === 'Stdio' ? draft.args : [], url: draft.transport === 'Stdio' ? null : draft.url, cwd: draft.cwd || null, credential_references: [...draft.forwarded_env.map(name => `env:${name}`), ...draft.header_env.map(([name, ref]) => `header:${name}=env:${ref}`), ...(draft.bearer_env ? [`env:${draft.bearer_env}`] : [])], redacted: false, config_digest: `fixture-draft-${++nextOperation}` } : projection;
      const probe: NativeProbeSnapshot = { probe_id: `fixture-probe-${++nextOperation}`, selection: request.selection, config, digest: `fixture-plan-${nextOperation}`, probe_args: config.args, cleanup_plan: null, executable: config.program, cwd: config.cwd ?? '/fixture', root: '/fixture', platform_identity: 'fixture-platform', executable_identity: config.program ? 'fixture-binary' : null, expires_at: Math.floor(Date.now() / 1000) + 120, state: 'ready', phase: 'approval', cleanup: 'not_started', error: null, observed_at: null, protocol_version: null, tools: [], schema_source: 'independent_host_probe', may_download_execute: config.transport === 'Stdio' };
      probes.set(probe.probe_id, probe); return probe;
    }
    if (['run_native_mcp_probe', 'cancel_native_mcp_probe', 'inspect_native_mcp_probe'].includes(command)) {
      const probe = probes.get(String(input.probe_id));
      if (!probe) throw 'native_extension_NotFound';
      if (command === 'run_native_mcp_probe') { probe.state = 'succeeded'; probe.phase = 'finished'; probe.cleanup = 'confirmed'; probe.observed_at = Math.floor(Date.now() / 1000); probe.protocol_version = '2025-03-26'; probe.tools = [{ name: 'fixture_lookup', description: 'Read-only tool definition fixture', inputSchema: { type: 'object', properties: { query: { type: 'string' } }, required: ['query'] } }]; }
      if (command === 'cancel_native_mcp_probe') { probe.state = 'cancelled'; probe.cleanup = 'confirmed'; }
      return probe;
    }
    if (command === 'preview_native_skill_visibility') {
      // Renderer-only fixture input uses the production TypeScript IPC contract.
      const request = input as unknown as PreviewNativeVisibility;
      const id = `fixture-visibility-${++nextOperation}`;
      previews.set(id, request);
      const intent = request.intent;
      const rule = intent.kind === 'claude_name' ? { evidence_version: 1, name: intent.name, state: intent.state, previous: savedRules.get(layerKey(request))?.get(intent.name) ?? null, scope: request.scope, current_and_future: true, plugins_excluded: true, higher_precedence: 'not_evaluated', config_change_hook_possible: true } : null;
      return { preview: { receipt: { operation_id: id, state: 'Previewed', pending_activation: false, recovery_material_retained: false }, target: request.target, scope: request.scope, target_path: '/fixture/settings.json', expires_at: Math.floor(Date.now() / 1000) + 600, rule }, affected: intent.kind === 'codex_path' ? [{ selection: request, instance: intent.instance, slug: skill.slug }] : [], coverage: intent.kind === 'codex_path' ? 'exact_path' : 'current_observations_only', observation_coverage: intent.kind === 'codex_path' ? 'not_applicable' : 'not_captured', rule };
    }
    if (command === 'preview_native_skill' || command === 'preview_native_mcp') return { receipt: { operation_id: 'fixture-operation-1', state: 'Previewed', pending_activation: false, recovery_material_retained: false }, target: input.target, scope: input.scope, project_id: input.project_id, task_id: input.task_id ?? null, target_path: command === 'preview_native_mcp' ? (input.target === 'Claude' ? '/fixture/.claude.json' : '/fixture/.codex/config.toml') : '/fixture/.agents/skills/sample-skill', expires_at: Math.floor(Date.now() / 1000) + 600, slug: input.slug ?? 'sample-skill', before_hash: 'before-fixture', after_hash: 'after-fixture', source: null, resources: [skill.resources[0]], metadata: skill.metadata };
    if (command === 'commit_native_operation' || command === 'cancel_native_operation' || command === 'inspect_native_operation') {
      const request = previews.get(String(input.operation_id));
      if (command === 'commit_native_operation' && request?.intent.kind === 'claude_name') {
        const key = layerKey(request), rules = savedRules.get(key) ?? new Map<string, NativeSkillRuleState>();
        if (request.intent.state === 'inherit') rules.delete(request.intent.name);
        else rules.set(request.intent.name, request.intent.state);
        savedRules.set(key, rules);
      }
      return { operation_id: input.operation_id, state: command === 'inspect_native_operation' ? 'Unknown' : command === 'cancel_native_operation' ? 'Cancelled' : 'Committed', pending_activation: command === 'commit_native_operation', recovery_material_retained: false };
    }
    throw new Error('Unexpected fixture command: ' + command);
  });
  const root = createRoot(host);
  root.render(<MemoryRouter><ManagementSurface /></MemoryRouter>);
  return () => { root.unmount(); clearMocks(); };
}
