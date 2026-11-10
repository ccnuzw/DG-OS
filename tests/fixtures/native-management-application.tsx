import React from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';
import { ModalHost } from '@dgos/ui';
import NativeManager from '../../src/components/nativeExtensions/NativeManager';
import { useDevelopment } from '../../src/stores/developmentStore';
import '../../src/index.css';
import type { PreviewNativeVisibility, NativeSelection, NativeSkillRuleState } from '../../src/types/nativeExtensions';

const skill = { instance: { id: 'fixture-skill-1', revision: 'fixture-revision' }, target: 'Codex', scope: 'OsUser', slug: 'sample-skill', owner: 'Loose', metadata: { name: 'Sample native skill', description: 'Isolated renderer fixture', license: null, dependencies: { declared: true, tools: [{ program: 'node', version_requirement: null }], packages: [], required_tools: [] }, dynamic_commands: false }, resources: [{ path: 'SKILL.md', hash: 'sample-hash', bytes: 128, executable: false }], source: { source_hash: 'sample-source-hash', commit: null }, can_update: true };
export function mountNativeManagement(host: HTMLElement) {
  useDevelopment.setState({ loaded: true, projects: [{ id: 'fixture-project-a', owner: 'fixture', name: 'Alpha Project', path: '/opaque/alpha', directory_identity: 'opaque-a', archived: false, created_at: 1, last_opened_at: null }, { id: 'fixture-project-b', owner: 'fixture', name: 'Beta Project', path: '/opaque/beta', directory_identity: 'opaque-b', archived: false, created_at: 1, last_opened_at: null }] });
  const savedRules = new Map<string, Map<string, NativeSkillRuleState>>();
  const previews = new Map<string, PreviewNativeVisibility>();
  let nextOperation = 0;
  const layerKey = (selection: NativeSelection) => JSON.stringify([selection.target, selection.scope, selection.project_id]);
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
    if (command === 'preview_native_skill_visibility') {
      // Renderer-only fixture input uses the production TypeScript IPC contract.
      const request = input as unknown as PreviewNativeVisibility;
      const id = `fixture-visibility-${++nextOperation}`;
      previews.set(id, request);
      const intent = request.intent;
      const rule = intent.kind === 'claude_name' ? { evidence_version: 1, name: intent.name, state: intent.state, previous: savedRules.get(layerKey(request))?.get(intent.name) ?? null, scope: request.scope, current_and_future: true, plugins_excluded: true, higher_precedence: 'not_evaluated', config_change_hook_possible: true } : null;
      return { preview: { receipt: { operation_id: id, state: 'Previewed', pending_activation: false, recovery_material_retained: false }, target: request.target, scope: request.scope, expires_at: Math.floor(Date.now() / 1000) + 600, rule }, affected: intent.kind === 'codex_path' ? [{ selection: request, instance: intent.instance, slug: skill.slug }] : [], coverage: intent.kind === 'codex_path' ? 'exact_path' : 'current_observations_only', observation_coverage: intent.kind === 'codex_path' ? 'not_applicable' : 'not_captured', rule };
    }
    if (command.startsWith('preview_native_')) return { receipt: { operation_id: 'fixture-operation-1', state: 'Previewed', pending_activation: false, recovery_material_retained: false }, target: input.target, scope: input.scope, expires_at: Math.floor(Date.now() / 1000) + 600, slug: input.slug ?? 'sample-skill', before_hash: 'before-fixture', after_hash: 'after-fixture', source: null, resources: [skill.resources[0]], metadata: skill.metadata };
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
  root.render(<MemoryRouter><div className="flex min-h-screen flex-col gap-4 p-4"><section><h1>Skills manager fixture</h1><NativeManager kind="skills" /></section><section><h1>MCP manager fixture</h1><NativeManager kind="mcp" /></section></div><ModalHost /></MemoryRouter>);
  return () => { root.unmount(); clearMocks(); };
}
