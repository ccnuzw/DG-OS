import { createRef } from 'react';
import { cleanup, fireEvent, render, screen, waitFor, act, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import NativeManager, { type NativeManagerHandle } from '../src/components/nativeExtensions/NativeManager';
import { useDevelopment } from '../src/stores/developmentStore';
import { useNativeExtensions } from '../src/stores/nativeExtensions';
import NativePluginsDrawer from '../src/components/nativeExtensions/NativePluginsDrawer';
import { nativeLibraryKey, useNativeSkillLibrary } from '../src/stores/nativeSkillLibrary';
import type { NativeDraft, NativeMcpInstance, NativeReceipt, NativeSelection, NativeSkill, NativeSkillPreview, NativeSkillRuleIntentState, NativeVisibilityPreview, PreviewNativeSkill } from '../src/types/nativeExtensions';
import { ModalHost } from '@dgos/ui';
import { SurfaceScope } from '../src/workspace/SurfaceContext';
import { createWorkspaceController } from '../src/workspace/controller';
import { useNativeManagerGuard } from '../src/components/nativeExtensions/NativeManagerLifecycle';
const { invoke, childState } = vi.hoisted(() => ({ invoke: vi.fn(), childState: { blocked: false } }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ t: (key: string) => key, useLocale: () => ({ locale: 'en-US', t: (key: string) => key }) }));
vi.mock('../src/components/nativeExtensions/NativeSessionPanel', () => ({ default: () => null }));
// Child workflows have their own behavior coverage. These boundary controls send
// real transaction inputs through the manager's production store and lifecycle.
vi.mock('../src/components/nativeExtensions/NativeSkillLibrary', () => {
  return {
    NativeSkillVersionRecovery: () => null,
    NativeSkillLibrary: ({ selection, tab, disabled, onPreview }: { selection: NativeSelection; tab: string; disabled: boolean; onPreview(input: PreviewNativeSkill): void }) => {
      useNativeManagerGuard(() => !childState.blocked);
      return <button disabled={disabled} onClick={() => onPreview({ ...selection, action: 'install', slug: 'requested-slug', expected: null, source: { kind: 'github', input: 'owner/repo/tree/main/skill' } })}>library.{tab}.preview</button>;
    },
    NativeSkillInspector: ({ selection, skill, disabled, onPreview }: { selection: NativeSelection; skill: NativeSkill; disabled: boolean; onPreview(input: PreviewNativeSkill): void }) => {
      useNativeManagerGuard(() => !childState.blocked);
      return <button disabled={disabled || !skill.can_update || skill.owner !== 'Loose'} onClick={() => onPreview({ ...selection, action: 'update', slug: skill.slug, expected: skill.instance, source: { kind: 'github', input: 'owner/repo/tree/commit/skill' } })}>inspector.previewUpdate</button>;
    },
  };
});
vi.mock('../src/components/nativeExtensions/NativeMcpMarket', () => ({ default: ({ disabled, onConfigure }: { disabled: boolean; onConfigure(draft: NativeDraft): boolean | Promise<boolean> }) => <button disabled={disabled} onClick={() => void onConfigure({ slug: '', source: '', name: 'registry-server', transport: 'Stdio', program: 'npx', args: '--yes\n@fixture/server', cwd: '', url: '', bearer_env: '', forwarded_env: 'API_KEY', header_env: '', modes: { program: 'set', args: 'set', forwarded_env: 'set' } })}>market.configure</button> }));
vi.mock('../src/components/nativeExtensions/NativeMcpProbe', () => {
  return { NativeMcpProbeRecovery: () => null, default: () => { useNativeManagerGuard(() => !childState.blocked); return null; } };
});

const skill: NativeSkill = { instance: { id: 'skill-1', revision: 'rev-1' }, target: 'Codex', scope: 'OsUser', slug: 'sample-skill', owner: 'Loose', metadata: { name: 'Sample skill', description: 'Installed description', license: null, dependencies: { declared: true, tools: [], packages: [], required_tools: [] }, dynamic_commands: false }, resources: [], source: { source_hash: 'installed-hash', commit: 'installed-commit' }, source_modified: false, can_update: true };
const receipt: NativeReceipt = { operation_id: 'preview-operation', state: 'Previewed', pending_activation: false, recovery_material_retained: false };
const basePreview = { receipt, target: 'Codex' as const, scope: 'OsUser' as const, target_path: '/authorized/config.toml', expires_at: 4102444800 };
const skillPreview: NativeSkillPreview = { ...basePreview, project_id: null, target_path: '/authorized/skills/returned-slug', slug: 'returned-slug', before_hash: 'before-hash', after_hash: 'after-hash', source: { source_hash: 'returned-source-hash', commit: 'returned-commit' }, metadata: { ...skill.metadata, name: 'Authoritative package', description: 'Returned description', license: 'MIT', dynamic_commands: true, dependencies: { declared: true, tools: [{ program: 'python', version_requirement: '>=3' }], packages: ['sample-package'], required_tools: ['sample-tool'] } }, impact: null, resources: [{ path: 'scripts/inspect.py', hash: 'script-hash', bytes: 42, executable: true }] };
const server: NativeMcpInstance = { native_key: 'alpha', transport: 'Stdio', enabled: true, owner: 'Loose', has_auth_configuration: true, unknown_fields_present: true, authentication: { configured: true, credential_present: null, authenticated: null, connected: null, approved: null, active_session: null, login: 'Unsupported', logout: 'Unsupported', delete_credentials: 'Unsupported' } };

async function choose(label: string, option: string) {
  fireEvent.click(screen.getByRole('combobox', { name: label }));
  fireEvent.click(await screen.findByRole('option', { name: option }));
  await waitFor(() => expect(screen.getByRole('combobox', { name: label }).textContent).toContain(option));
}
async function createServer() {
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.createServer' }));
  await screen.findByLabelText('nativeExtensions.name');
  fireEvent.change(screen.getByLabelText('nativeExtensions.name'), { target: { value: 'new-server' } });
  fireEvent.change(screen.getByLabelText('nativeExtensions.program'), { target: { value: 'node' } });
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.previewChange' }));
  await screen.findByRole('button', { name: 'nativeExtensions.confirm' });
}

beforeEach(() => {
  invoke.mockReset();
  useDevelopment.setState({ epoch: 0, loaded: true, projects: [{ id: 'project-a', owner: 'fixture', name: 'Project A', path: '/tmp/project-a', directory_identity: 'a', archived: false, created_at: 1, last_opened_at: null }], tasks: [], sessions: [], selection: { project_id: null, task_id: null, session_id: null }, operations: [] });
  childState.blocked = false;
  useNativeSkillLibrary.getState().reset();
  useNativeExtensions.getState().reset();
  invoke.mockImplementation(async (command: string) => {
    if (command === 'discover_native_skills') return { instances: [skill], readonly_entries: [], missing: false, incomplete: false };
    if (command === 'discover_native_skill_rules') return { rules: [], coverage: 'selected_settings_layer' };
    if (command === 'preview_native_skill') return skillPreview;
    if (command === 'preview_native_mcp') return basePreview;
    if (command === 'commit_native_operation') return { ...receipt, state: 'Committed', pending_activation: true };
    if (command === 'cancel_native_operation') return { ...receipt, state: 'Cancelled' };
    if (command === 'inspect_native_operation') return receipt;
    return { instances: [], incomplete: false };
  });
});
afterEach(cleanup);

describe('native skill workflows', () => {
  it('routes library imports through authoritative preview without installing on selection', async () => {
    render(<NativeManager kind="skills" />);
    await screen.findAllByText('Sample skill');
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.installSkill' }));
    fireEvent.click(await screen.findByRole('button', { name: 'library.discover.preview' }));
    const preview = within(await screen.findByRole('region', { name: 'nativeExtensions.preview' }));
    for (const text of ['Authoritative package', 'Returned description', 'returned-slug', 'before-hash', 'after-hash', 'returned-source-hash', 'returned-commit', '/authorized/skills/returned-slug']) expect(preview.getByText(text)).toBeTruthy();
    expect(preview.getByText(/python >=3, sample-package, sample-tool/)).toBeTruthy();
    expect(preview.getByText('nativeExtensions.scriptsNotRun')).toBeTruthy();
    expect(invoke).toHaveBeenCalledWith('preview_native_skill', { input: { target: 'Codex', scope: 'OsUser', project_id: null, action: 'install', slug: 'requested-slug', expected: null, source: { kind: 'github', input: 'owner/repo/tree/main/skill' } } });
    expect(invoke).not.toHaveBeenCalledWith('commit_native_operation', expect.anything());
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.confirm' }));
    await waitFor(() => expect(screen.getByRole('tab', { name: 'nativeExtensions.tab.installed' }).getAttribute('aria-selected')).toBe('true'));
    expect(invoke).toHaveBeenCalledWith('commit_native_operation', { input: { operation_id: receipt.operation_id } });
  });

  it('shows bounded Claude same-name evidence and refuses to imply complete visibility', async () => {
    invoke.mockImplementation(async (command: string) => command === 'discover_native_skill_rules' ? { rules: [], coverage: 'selected_settings_layer' } : command === 'preview_native_skill'
      ? { ...skillPreview, target: 'Claude', impact: { native_name: 'fixture', previous_name: null, same_name: [{ scope: 'ProjectShared', slug: 'other-folder', native_name: 'fixture', owner: 'Loose' }], coverage: 'authorized_loose_roots', incomplete: true, visibility_writable: false } }
      : { instances: [{ ...skill, target: 'Claude' }], readonly_entries: [], missing: false, incomplete: false });
    render(<NativeManager kind="skills" initialSelection={{ target: 'Claude', scope: 'OsUser', project_id: null }} />);
    await screen.findAllByText('Sample skill');
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.remove' }));
    const preview = within(await screen.findByRole('region', { name: 'nativeExtensions.preview' }));
    expect(preview.getByText('nativeExtensions.looseImpactCoverage')).toBeTruthy();
    expect(preview.getByText('nativeExtensions.visibilityUnavailable')).toBeTruthy();
    expect(preview.getByText(/other-folder.*nativeExtensions.scope.ProjectShared.*nativeExtensions.owner.Loose/)).toBeTruthy();
    expect(preview.getByText(/scripts\/inspect.py/)).toBeTruthy();
    expect(preview.getByRole('status').textContent).toBe('nativeExtensions.incomplete');
    expect(invoke).not.toHaveBeenCalledWith('commit_native_operation', expect.anything());
  });

  it('updates only the selected revision via the inspector and removes without a source', async () => {
    render(<NativeManager kind="skills" />);
    fireEvent.click(await screen.findByRole('button', { name: 'inspector.previewUpdate' }));
    await screen.findByRole('button', { name: 'nativeExtensions.confirm' });
    expect(invoke).toHaveBeenCalledWith('preview_native_skill', { input: expect.objectContaining({ action: 'update', slug: skill.slug, expected: skill.instance, source: { kind: 'github', input: 'owner/repo/tree/commit/skill' } }) });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.cancel' }));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'nativeExtensions.confirm' })).toBeNull());
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.remove' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('preview_native_skill', { input: expect.objectContaining({ action: 'remove', slug: skill.slug, expected: skill.instance, source: null }) }));
  });

  it('previews exact-instance visibility and does not consume a source-library draft', async () => {
    invoke.mockImplementation(async (command: string) => command === 'preview_native_skill_visibility'
      ? { preview: basePreview, rule: null, coverage: 'exact_path', affected: [{ selection: { target: 'Codex', scope: 'ProjectShared', project_id: 'project-a' }, instance: { id: 'other-instance', revision: 'other-revision' }, slug: 'same-native-name' }] }
      : command === 'commit_native_operation' ? { ...receipt, state: 'Committed' }
        : { instances: [skill], readonly_entries: [], missing: false, incomplete: false });
    render(<NativeManager kind="skills" />);
    await screen.findAllByText('Sample skill');
    const selection: NativeSelection = { target: 'Codex', scope: 'OsUser', project_id: null };
    const sourceDraft = { input: 'owner/keep-source', sourceId: null, revision: null };
    act(() => useNativeSkillLibrary.getState().setDraft(selection, sourceDraft));
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.disableSkill' }));
    const preview = within(await screen.findByRole('region', { name: 'nativeExtensions.preview' }));
    expect(preview.getByText(/same-native-name.*Codex.*nativeExtensions.scope.ProjectShared.*project-a.*other-instance.*other-revision/)).toBeTruthy();
    expect(invoke).toHaveBeenCalledWith('preview_native_skill_visibility', { input: { target: 'Codex', scope: 'OsUser', project_id: null, intent: { kind: 'codex_path', instance: skill.instance, visible: false } } });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.confirm' }));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'nativeExtensions.confirm' })).toBeNull());
    expect(useNativeSkillLibrary.getState().drafts[nativeLibraryKey(selection)]).toBe(sourceDraft);
  });

  it('keeps late preview receipts in their original scope and restores them on return', async () => {
    const deferred = Promise.withResolvers<unknown>();
    invoke.mockImplementation((command: string) => command === 'preview_native_skill' ? deferred.promise : Promise.resolve({ instances: [skill], readonly_entries: [], missing: false, incomplete: false }));
    render(<NativeManager kind="skills" />);
    await screen.findAllByText('Sample skill');
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.remove' }));
    await choose('nativeExtensions.scope', 'nativeExtensions.scope.ProjectShared');
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('discover_native_skills', expect.objectContaining({ input: expect.objectContaining({ scope: 'ProjectShared' }) })));
    await act(async () => { deferred.resolve(skillPreview); });
    expect(screen.getByRole('button', { name: 'nativeExtensions.refresh' }).hasAttribute('disabled')).toBe(false);
    expect(screen.queryByRole('button', { name: 'nativeExtensions.confirm' })).toBeNull();
    await choose('nativeExtensions.scope', 'nativeExtensions.scope.OsUser');
    expect(await screen.findByRole('button', { name: 'nativeExtensions.confirm' })).toBeTruthy();
    expect(screen.getByText('returned-commit')).toBeTruthy();
  });
});

describe('Claude name rule consumers', () => {
  function mockRules(rules: { name: string; state: 'off' | 'name-only' }[] = []) {
    invoke.mockImplementation(async (command: string, args: { input: Record<string, unknown> }) => {
      if (command === 'discover_native_skill_rules') return { rules, coverage: 'selected_settings_layer' };
      if (command === 'discover_native_skills') return { instances: [], readonly_entries: [], missing: false, incomplete: false };
      if (command === 'preview_native_skill_visibility') {
        const intent = args.input.intent as { name: string; state: NativeSkillRuleIntentState };
        const scope = args.input.scope as NativeSelection['scope'];
        return { preview: { ...basePreview, target: 'Claude', scope }, affected: [], coverage: 'current_observations_only', observation_coverage: 'not_captured', rule: { evidence_version: 1, name: intent.name, state: intent.state, previous: rules.find(rule => rule.name === intent.name)?.state ?? null, scope, current_and_future: true, plugins_excluded: true, higher_precedence: 'not_evaluated', config_change_hook_possible: true } } satisfies NativeVisibilityPreview;
      }
      if (command === 'commit_native_operation') return { ...receipt, state: 'Committed', pending_activation: true };
      if (command === 'cancel_native_operation') return { ...receipt, state: 'Cancelled' };
      throw 'unexpected command';
    });
  }

  it.each(['on', 'name-only', 'user-invocable-only', 'off', 'inherit'] as const)('previews explicit absent-resource name with %s semantics and cancels without writing', async state => {
    mockRules();
    render(<NativeManager kind="skills" initialSelection={{ target: 'Claude', scope: 'OsUser', project_id: null }}/>);
    await screen.findByText('nativeExtensions.noNameRules');
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.nameRules' }));
    fireEvent.change(screen.getByLabelText('nativeExtensions.ruleName'), { target: { value: 'future-skill' } });
    await choose('nativeExtensions.ruleState', `nativeExtensions.ruleState.${state}`);
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.previewRule' }));
    const preview = within(await screen.findByRole('region', { name: 'nativeExtensions.preview' }));
    expect(invoke).toHaveBeenCalledWith('preview_native_skill_visibility', { input: { target: 'Claude', scope: 'OsUser', project_id: null, intent: { kind: 'claude_name', name: 'future-skill', state } } });
    for (const key of [`ruleMeaning.${state}`, 'ruleNameImpact', 'rulePluginsExcluded', 'rulePrecedenceWarning', 'ruleHookWarning', 'coverage.current_observations_only']) expect(preview.getByText(`nativeExtensions.${key}`)).toBeTruthy();
    expect(preview.getByText('nativeExtensions.ruleObservationsNotCaptured')).toBeTruthy();
    const observationsLink = preview.getByRole('button', { name: 'nativeExtensions.reviewRuleObservations' });
    fireEvent.click(observationsLink);
    await waitFor(() => expect(document.activeElement?.id).toBe(observationsLink.getAttribute('aria-controls')));
    expect(invoke.mock.calls.some(([command]) => command === 'observe_native_session')).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.cancel' }));
    await screen.findByText('nativeExtensions.state.Cancelled', { selector: 'dd' });
    expect((screen.getByLabelText('nativeExtensions.ruleName') as HTMLInputElement).value).toBe('future-skill');
    expect(invoke).not.toHaveBeenCalledWith('commit_native_operation', expect.anything());
    expect(invoke.mock.calls.some(([command]) => command === 'native_session_action')).toBe(false);
  });

  it.each(['OsUser', 'ProjectShared', 'ProjectLocal'] as const)('restores a saved absent-instance rule in %s without resource authority or implicit refresh', async scope => {
    mockRules([{ name: 'hidden-skill', state: 'off' }]);
    const selection = { target: 'Claude' as const, scope, project_id: scope === 'OsUser' ? null : 'project-a' };
    render(<NativeManager kind="skills" initialSelection={selection}/>);
    if (scope !== 'ProjectLocal') fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.nameRules' }));
    const rules = within(await screen.findByRole('region', { name: 'nativeExtensions.nameRules' }));
    await rules.findByText('hidden-skill');
    expect(invoke).toHaveBeenCalledWith('discover_native_skill_rules', { input: selection });
    fireEvent.click(rules.getByRole('button', { name: 'nativeExtensions.restoreInherit' }));
    const preview = within(await screen.findByRole('region', { name: 'nativeExtensions.preview' }));
    expect(preview.getByText('nativeExtensions.ruleState.off')).toBeTruthy();
    expect(preview.getByText('nativeExtensions.ruleState.inherit')).toBeTruthy();
    expect(invoke).toHaveBeenCalledWith('preview_native_skill_visibility', { input: { ...selection, intent: { kind: 'claude_name', name: 'hidden-skill', state: 'inherit' } } });
    const readCount = invoke.mock.calls.filter(([command]) => command.startsWith('discover_')).length;
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.confirm' }));
    await screen.findByText('nativeExtensions.state.Committed', { selector: 'dd' });
    expect(invoke.mock.calls.filter(([command]) => command.startsWith('discover_'))).toHaveLength(readCount);
    expect(invoke.mock.calls.some(([command]) => command === 'native_session_action')).toBe(false);
    expect(screen.getByText('nativeExtensions.refreshRequired')).toBeTruthy();
    if (scope === 'ProjectLocal') {
      expect(screen.getByRole('button', { name: 'nativeExtensions.installSkill' }).hasAttribute('disabled')).toBe(true);
      expect(invoke).not.toHaveBeenCalledWith('discover_native_skills', expect.anything());
      expect(invoke).not.toHaveBeenCalledWith('preview_native_skill', expect.anything());
    }
    const manager = screen.getByRole('region', { name: 'nativeExtensions.skills' });
    const ruleReadCount = invoke.mock.calls.filter(([command]) => command === 'discover_native_skill_rules').length;
    fireEvent.click(within(manager.querySelector('header')!).getByRole('button', { name: 'nativeExtensions.refresh' }));
    await waitFor(() => expect(invoke.mock.calls.filter(([command]) => command === 'discover_native_skill_rules')).toHaveLength(ruleReadCount + 1));
  });

  it('fills only the native rule name and preserves both source and newer rule drafts', async () => {
    mockRules();
    const rulesMock = invoke.getMockImplementation()!;
    invoke.mockImplementation(async (command: string, args: { input: Record<string, unknown> }) => command === 'discover_native_skills' ? { instances: [{ ...skill, target: 'Claude' }], readonly_entries: [], missing: false, incomplete: false } : rulesMock(command, args));
    const selection: NativeSelection = { target: 'Claude', scope: 'OsUser', project_id: null };
    render(<NativeManager kind="skills" initialSelection={selection}/>);
    await screen.findAllByText('Sample skill');
    const sourceDraft = { input: 'owner/keep-source', sourceId: null, revision: null };
    act(() => useNativeSkillLibrary.getState().setDraft(selection, sourceDraft));
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.useRuleName' }));
    expect((screen.getByLabelText('nativeExtensions.ruleName') as HTMLInputElement).value).toBe(skill.metadata.name);
    expect(screen.queryByRole('button', { name: 'nativeExtensions.enableSkill' })).toBeNull();
    expect(invoke).not.toHaveBeenCalledWith('preview_native_skill_visibility', expect.anything());
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.previewRule' }));
    await screen.findByRole('button', { name: 'nativeExtensions.confirm' });
    fireEvent.change(screen.getByLabelText('nativeExtensions.ruleName'), { target: { value: 'newer-name' } });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.confirm' }));
    await screen.findByText('nativeExtensions.state.Committed', { selector: 'dd' });
    expect(useNativeSkillLibrary.getState().drafts[nativeLibraryKey(selection)]).toBe(sourceDraft);
    expect((screen.getByLabelText('nativeExtensions.ruleName') as HTMLInputElement).value).toBe('newer-name');
  });
});

describe('native MCP field changes', () => {
  it('previews a new server without committing and renders only one editor', async () => {
    invoke.mockImplementation(async (command: string) => command === 'preview_native_mcp' ? basePreview : { instances: [server], incomplete: false });
    render(<NativeManager kind="mcp" />);
    await screen.findByRole('button', { name: /alpha/ });
    await createServer();
    expect(screen.getAllByLabelText('nativeExtensions.program')).toHaveLength(1);
    expect(invoke).toHaveBeenCalledWith('preview_native_mcp', { input: expect.objectContaining({ target: 'Codex', scope: 'OsUser', project_id: null, change: { kind: 'upsert', name: 'new-server', fields: expect.objectContaining({ transport: { kind: 'set', value: 'Stdio' }, program: { kind: 'set', value: 'node' }, args: { kind: 'keep' } }) } }) });
    expect(invoke).not.toHaveBeenCalledWith('commit_native_operation', expect.anything());
  });

  it('keeps hidden fields and offers explicit keep, set and remove while edits remain attached to their server', async () => {
    invoke.mockImplementation(async (command: string) => command === 'preview_native_mcp' ? basePreview : { instances: [server, { ...server, native_key: 'beta' }], incomplete: false });
    render(<NativeManager kind="mcp" />);
    await screen.findByRole('button', { name: /alpha/ });
    fireEvent.click(screen.getByText('nativeExtensions.editConfiguration'));
    fireEvent.change(screen.getByLabelText('nativeExtensions.program'), { target: { value: 'alpha-command' } });
    fireEvent.click(screen.getByRole('button', { name: /beta/ }));
    await waitFor(() => expect((screen.getByLabelText('nativeExtensions.program') as HTMLInputElement).value).toBe(''));
    fireEvent.click(screen.getByRole('button', { name: /alpha/ }));
    await waitFor(() => expect((screen.getByLabelText('nativeExtensions.program') as HTMLInputElement).value).toBe('alpha-command'));
    expect((screen.getByLabelText('nativeExtensions.program') as HTMLInputElement).value).toBe('alpha-command');
    await choose('nativeExtensions.args: nativeExtensions.fieldAction', 'nativeExtensions.field.remove');
    await choose('nativeExtensions.program: nativeExtensions.fieldAction', 'nativeExtensions.field.keep');
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.previewChange' }));
    await screen.findByRole('button', { name: 'nativeExtensions.confirm' });
    expect(invoke).toHaveBeenCalledWith('preview_native_mcp', { input: expect.objectContaining({ change: { kind: 'upsert', name: 'alpha', fields: { program: { kind: 'keep' }, args: { kind: 'remove' }, cwd: { kind: 'keep' }, forwarded_env: { kind: 'keep' } } } }) });
    expect(screen.getByText('nativeExtensions.unknownFieldsKept')).toBeTruthy();
  });

  it('does not turn a disappeared edit into a same-name creation', async () => {
    let discoveries = 0;
    invoke.mockImplementation(async () => ({ instances: ++discoveries === 1 ? [server] : [], incomplete: false }));
    render(<NativeManager kind="mcp" />);
    await screen.findByRole('button', { name: /alpha/ });
    fireEvent.click(screen.getByText('nativeExtensions.editConfiguration'));
    fireEvent.change(screen.getByLabelText('nativeExtensions.program'), { target: { value: 'replacement' } });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.previewChange' }));
    await screen.findByText('nativeExtensions.error.native_extension_NotFound');
    expect(invoke).not.toHaveBeenCalledWith('preview_native_mcp', expect.anything());
  });

  it('previews MCP removal and enabled changes without consuming field drafts', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'preview_native_mcp') return basePreview;
      if (command === 'cancel_native_operation') return { ...receipt, state: 'Cancelled' };
      if (command === 'commit_native_operation') return { ...receipt, state: 'Committed' };
      return { instances: [server], incomplete: false };
    });
    render(<NativeManager kind="mcp" />);
    await screen.findByRole('button', { name: /alpha/ });
    fireEvent.click(screen.getByText('nativeExtensions.editConfiguration'));
    fireEvent.change(screen.getByLabelText('nativeExtensions.program'), { target: { value: 'keep-field-draft' } });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.remove' }));
    await screen.findByRole('button', { name: 'nativeExtensions.confirm' });
    expect(invoke).toHaveBeenCalledWith('preview_native_mcp', { input: { target: 'Codex', scope: 'OsUser', project_id: null, change: { kind: 'remove', name: 'alpha' } } });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.cancel' }));
    await screen.findByText('nativeExtensions.state.Cancelled', { selector: 'dd' });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.visibility' }));
    await screen.findByRole('button', { name: 'nativeExtensions.confirm' });
    expect(invoke).toHaveBeenCalledWith('preview_native_mcp', { input: { target: 'Codex', scope: 'OsUser', project_id: null, change: { kind: 'set_enabled', name: 'alpha', enabled: false } } });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.confirm' }));
    await screen.findByText('nativeExtensions.state.Committed', { selector: 'dd' });
    expect((screen.getByLabelText('nativeExtensions.program') as HTMLInputElement).value).toBe('keep-field-draft');
  });

  it('rejects a new name that appeared after discovery instead of overwriting it', async () => {
    let discoveries = 0;
    invoke.mockImplementation(async () => ({ instances: ++discoveries === 1 ? [] : [{ ...server, native_key: 'new-server' }], incomplete: false }));
    render(<NativeManager kind="mcp" />);
    await screen.findByText('nativeExtensions.empty');
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.createServer' }));
    await screen.findByLabelText('nativeExtensions.name');
    fireEvent.change(screen.getByLabelText('nativeExtensions.name'), { target: { value: 'new-server' } });
    fireEvent.change(screen.getByLabelText('nativeExtensions.program'), { target: { value: 'node' } });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.previewChange' }));
    await screen.findByText('nativeExtensions.error.native_extension_Conflict');
    expect(invoke).not.toHaveBeenCalledWith('preview_native_mcp', expect.anything());
    expect((screen.getByLabelText('nativeExtensions.program') as HTMLInputElement).value).toBe('node');
  });

  it('rejects malformed references with a linked shared-field error', async () => {
    render(<NativeManager kind="mcp" />);
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.createServer' }));
    await screen.findByLabelText('nativeExtensions.name');
    fireEvent.change(screen.getByLabelText('nativeExtensions.name'), { target: { value: 'remote' } });
    await choose('nativeExtensions.transport', 'Http');
    fireEvent.change(screen.getByLabelText('nativeExtensions.url'), { target: { value: 'https://example.test/mcp' } });
    fireEvent.change(screen.getByLabelText('nativeExtensions.header_env'), { target: { value: 'Authorization=TOKEN\nmalformed' } });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.previewChange' }));
    const headers = screen.getByLabelText('nativeExtensions.header_env');
    expect(headers.getAttribute('aria-invalid')).toBe('true');
    expect(document.getElementById(headers.getAttribute('aria-describedby')!)?.textContent).toBe('nativeExtensions.invalidHeaderReference');
    expect(invoke).not.toHaveBeenCalledWith('preview_native_mcp', expect.anything());
    expect(screen.queryByRole('button', { name: 'nativeExtensions.confirm' })).toBeNull();
  });

  it('limits Claude header editing to supported patches and preserves unsupported fields', async () => {
    invoke.mockImplementation(async (command: string) => command === 'preview_native_mcp' ? { ...basePreview, target: 'Claude' } : { instances: [{ ...server, transport: 'Http' }], incomplete: false });
    render(<NativeManager kind="mcp" />);
    await choose('nativeExtensions.target', 'Claude Code');
    await screen.findByLabelText('nativeExtensions.header_env');
    fireEvent.click(screen.getByText('nativeExtensions.editConfiguration'));
    expect(screen.queryByLabelText('nativeExtensions.bearer_env')).toBeNull();
    fireEvent.click(screen.getByRole('combobox', { name: 'nativeExtensions.header_env: nativeExtensions.fieldAction' }));
    expect(screen.queryByRole('option', { name: 'nativeExtensions.field.remove' })).toBeNull();
    fireEvent.click(await screen.findByRole('option', { name: 'nativeExtensions.field.set' }));
    fireEvent.change(screen.getByLabelText('nativeExtensions.header_env'), { target: { value: 'X-Client=CLIENT_ENV' } });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.previewChange' }));
    await screen.findByRole('button', { name: 'nativeExtensions.confirm' });
    expect(invoke).toHaveBeenCalledWith('preview_native_mcp', { input: expect.objectContaining({ target: 'Claude', change: { kind: 'upsert', name: 'alpha', fields: { url: { kind: 'keep' }, header_env: { kind: 'set', value: [['X-Client', 'CLIENT_ENV']] } } } }) });
  });
});

describe('native operation lifecycle', () => {
  it('starts a fresh post-commit read and ignores an older in-flight catalog', async () => {
    const api = useNativeExtensions.getState(), selection = { target: 'Codex' as const, scope: 'OsUser' as const, project_id: null };
    const oldRead = Promise.withResolvers<unknown>();
    invoke.mockImplementation(async (command: string) => command === 'discover_native_mcp' ? oldRead.promise : { ...receipt, state: 'Committed' });
    api.setPending('fixture', { kind: 'mcp', selection, mode: 'preview', receipt, preview: basePreview, draft_key: 'fixture', draft: undefined, summary: 'change' });
    const before = api.discoverMcp(selection);
    await api.commit({ operation_id: receipt.operation_id });
    invoke.mockResolvedValueOnce({ instances: [{ ...server, native_key: 'published-server' }], incomplete: false });
    await api.discoverMcp(selection);
    oldRead.resolve({ instances: [{ ...server, native_key: 'old-server' }], incomplete: false });
    await before;
    const current = Object.values(useNativeExtensions.getState().mcp).find(state => state.value)?.value;
    expect(current?.instances.map(item => item.native_key)).toEqual(['published-server']);
  });
  it('blocks another mutation after an unknown commit and only inspects the original operation', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'preview_native_mcp') return basePreview;
      if (command === 'commit_native_operation') throw 'native_extension_Unavailable';
      if (command === 'inspect_native_operation') return { ...receipt, state: 'Committed', pending_activation: true };
      return { instances: [], incomplete: false };
    });
    render(<NativeManager kind="mcp" />);
    await createServer();
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.confirm' }));
    await screen.findByRole('button', { name: 'nativeExtensions.verify' });
    expect(screen.queryByRole('button', { name: 'nativeExtensions.confirm' })).toBeNull();
    expect(screen.getByRole('button', { name: 'nativeExtensions.previewChange' }).hasAttribute('disabled')).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.verify' }));
    await screen.findByText('nativeExtensions.state.Committed', { selector: 'dd' });
    expect(invoke.mock.calls.filter(([command]) => command === 'commit_native_operation')).toHaveLength(1);
    expect(invoke).toHaveBeenCalledWith('inspect_native_operation', { input: { operation_id: receipt.operation_id } });
    expect((screen.getByLabelText('nativeExtensions.program') as HTMLInputElement).value).toBe('');
  });

  it('keeps an unresolved receipt across management views and does not clear drafts on an unknown cancellation', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'preview_native_mcp') return basePreview;
      if (command === 'cancel_native_operation') return { ...receipt, state: 'Unknown', recovery_material_retained: true };
      if (command === 'inspect_native_operation') return { ...receipt, state: 'Cancelled' };
      if (command === 'discover_native_skills') return { instances: [skill], readonly_entries: [], missing: false, incomplete: false };
      return { instances: [], incomplete: false };
    });
    const view = render(<NativeManager kind="mcp" />);
    await createServer();
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.cancel' }));
    await screen.findByRole('button', { name: 'nativeExtensions.verify' });
    view.rerender(<NativeManager kind="skills" />);
    await screen.findAllByText('Sample skill');
    expect(screen.getByRole('button', { name: 'nativeExtensions.remove' }).hasAttribute('disabled')).toBe(true);
    expect(screen.getByText('preview-operation')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.verify' }));
    await screen.findByText('nativeExtensions.state.Cancelled', { selector: 'dd' });
    view.rerender(<NativeManager kind="mcp" />);
    expect((screen.getByLabelText('nativeExtensions.program') as HTMLInputElement).value).toBe('node');
    expect(invoke).toHaveBeenCalledWith('cancel_native_operation', { input: { operation_id: receipt.operation_id } });
    expect(invoke).toHaveBeenCalledWith('inspect_native_operation', { input: { operation_id: receipt.operation_id } });
    expect(invoke).not.toHaveBeenCalledWith('commit_native_operation', expect.anything());
  });

  it('retains cancellation and failure drafts, but clears only the exact committed snapshot', async () => {
    render(<NativeManager kind="mcp" />);
    await createServer();
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.cancel' }));
    await screen.findByText('nativeExtensions.state.Cancelled', { selector: 'dd' });
    expect((screen.getByLabelText('nativeExtensions.program') as HTMLInputElement).value).toBe('node');
    invoke.mockImplementationOnce(async () => { throw 'native_extension_AccessDenied'; });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.previewChange' }));
    await screen.findByText('nativeExtensions.error.native_extension_AccessDenied');
    expect((screen.getByLabelText('nativeExtensions.program') as HTMLInputElement).value).toBe('node');
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.previewChange' }));
    await screen.findByRole('button', { name: 'nativeExtensions.confirm' });
    fireEvent.change(screen.getByLabelText('nativeExtensions.program'), { target: { value: 'newer-edit' } });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.confirm' }));
    await screen.findByText('nativeExtensions.state.Committed', { selector: 'dd' });
    expect((screen.getByLabelText('nativeExtensions.program') as HTMLInputElement).value).toBe('newer-edit');
  });

  it('clears old account drafts and ignores its late preview receipt', async () => {
    const deferred = Promise.withResolvers<unknown>();
    invoke.mockImplementation((command: string) => command === 'preview_native_mcp' ? deferred.promise : Promise.resolve({ instances: [], incomplete: false }));
    render(<NativeManager kind="mcp" />);
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.createServer' }));
    await screen.findByLabelText('nativeExtensions.name');
    fireEvent.change(screen.getByLabelText('nativeExtensions.name'), { target: { value: 'old-account-server' } });
    fireEvent.change(screen.getByLabelText('nativeExtensions.program'), { target: { value: 'node' } });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.previewChange' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('preview_native_mcp', expect.anything()));
    act(() => useDevelopment.setState({ epoch: 1 }));
    await act(async () => { deferred.resolve(basePreview); });
    expect(screen.queryByRole('button', { name: 'nativeExtensions.confirm' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.createServer' }));
    await screen.findByLabelText('nativeExtensions.name');
    expect((screen.getByLabelText('nativeExtensions.name') as HTMLInputElement).value).toBe('');
    expect(screen.queryByText('old-account-server')).toBeNull();
  });
});

it('opens the requested native target and project without resetting a manual selection or its draft on equal entry rerenders', async () => {
 const entry = { target: 'Claude' as const, scope: 'ProjectShared' as const, project_id: 'project-a' };
 const view = render(<NativeManager kind="mcp" initialSelection={entry} initialSessionId="origin-session" />);
 await waitFor(() => expect(invoke).toHaveBeenCalledWith('discover_native_mcp', { input: entry }));
 await choose('nativeExtensions.target', 'Codex');
 fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.createServer' }));
 await screen.findByLabelText('nativeExtensions.name');
 fireEvent.change(screen.getByLabelText('nativeExtensions.name'), { target: { value: 'keep-native-draft' } });
 view.rerender(<NativeManager kind="mcp" initialSelection={{ ...entry }} initialSessionId="origin-session" />);
 expect(screen.getByRole('combobox', { name: 'nativeExtensions.target' }).textContent).toContain('Codex');
 expect((screen.getByLabelText('nativeExtensions.name') as HTMLInputElement).value).toBe('keep-native-draft');
 expect(invoke.mock.calls.some(([name]) => name === 'preview_native_mcp' || name === 'commit_native_operation' || name === 'native_session_action')).toBe(false);
});

it('discards only the closing surface drafts across selections and preserves other domains and surfaces', async () => {
 const controller = createWorkspaceController();
 render(<>
  <SurfaceScope id="mcp-a" active controller={controller}><div data-testid="mcp-a"><NativeManager kind="mcp" /></div></SurfaceScope>
  <SurfaceScope id="mcp-b" active controller={controller}><div data-testid="mcp-b"><NativeManager kind="mcp" /></div></SurfaceScope>
  <SurfaceScope id="skills" active controller={controller}><NativeManager kind="skills" /></SurfaceScope><ModalHost />
 </>);
 const first = within(screen.getByTestId('mcp-a')), second = within(screen.getByTestId('mcp-b'));
 fireEvent.click(first.getByRole('button', { name: 'nativeExtensions.createServer' }));
 fireEvent.change(await first.findByLabelText('nativeExtensions.name'), { target: { value: 'first-os-draft' } });
 fireEvent.click(first.getByRole('combobox', { name: 'nativeExtensions.scope' }));
 fireEvent.click(await screen.findByRole('option', { name: 'nativeExtensions.scope.ProjectShared' }));
 await waitFor(() => expect(invoke).toHaveBeenCalledWith('discover_native_mcp', { input: { target: 'Codex', scope: 'ProjectShared', project_id: 'project-a' } }));
 fireEvent.click(first.getByRole('button', { name: 'nativeExtensions.createServer' }));
 fireEvent.change(await first.findByLabelText('nativeExtensions.name'), { target: { value: 'first-project-draft' } });
 fireEvent.click(second.getByRole('button', { name: 'nativeExtensions.createServer' }));
 fireEvent.change(await second.findByLabelText('nativeExtensions.name'), { target: { value: 'second-surface-draft' } });
 const selection: NativeSelection = { target: 'Codex', scope: 'OsUser', project_id: null };
 const sourceDraft = { input: 'owner/other-domain', sourceId: null, revision: null };
 act(() => useNativeSkillLibrary.getState().setDraft(selection, sourceDraft));
 expect(Object.keys(useNativeExtensions.getState().drafts)).toHaveLength(3);
 let leave!: Promise<boolean>;
 act(() => { leave = controller.canLeave('mcp-a'); });
 fireEvent.change(second.getByLabelText('nativeExtensions.name'), { target: { value: 'newer-other-surface-draft' } });
 fireEvent.click(await screen.findByRole('button', { name: 'shell.discard' }));
 await act(async () => { expect(await leave).toBe(true); });
 expect(Object.keys(useNativeExtensions.getState().drafts)).toHaveLength(1);
 expect(controller.getSurfaceStatuses()['mcp-a'].dirty).toBe(false);
 expect(controller.getSurfaceStatuses()['mcp-b'].dirty).toBe(true);
 expect(controller.getSurfaceStatuses().skills.dirty).toBe(true);
 expect(useNativeSkillLibrary.getState().drafts[nativeLibraryKey(selection)]).toBe(sourceDraft);
 expect((second.getByLabelText('nativeExtensions.name') as HTMLInputElement).value).toBe('newer-other-surface-draft');
});

it('does not close over a draft edited while its discard confirmation is pending', async () => {
 const controller = createWorkspaceController();
 render(<><SurfaceScope id="native" active controller={controller}><NativeManager kind="mcp" /></SurfaceScope><ModalHost /></>);
 fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.createServer' }));
 const name = await screen.findByLabelText('nativeExtensions.name');
 fireEvent.change(name, { target: { value: 'before-confirm' } });
 let leave!: Promise<boolean>;
 act(() => { leave = controller.canLeave('native'); });
 fireEvent.change(name, { target: { value: 'after-confirm' } });
 fireEvent.click(await screen.findByRole('button', { name: 'shell.discard' }));
 await act(async () => { expect(await leave).toBe(false); });
 expect((name as HTMLInputElement).value).toBe('after-confirm');
 expect(controller.getSurfaceStatuses().native.dirty).toBe(true);
});

it('keeps standalone managers of the same domain in separate draft owners', async () => {
 render(<><div data-testid="one"><NativeManager kind="mcp" /></div><div data-testid="two"><NativeManager kind="mcp" /></div></>);
 const first = within(screen.getByTestId('one')), second = within(screen.getByTestId('two'));
 fireEvent.click(first.getByRole('button', { name: 'nativeExtensions.createServer' }));
 fireEvent.click(second.getByRole('button', { name: 'nativeExtensions.createServer' }));
 await first.findByLabelText('nativeExtensions.name'); await second.findByLabelText('nativeExtensions.name');
 fireEvent.change(first.getByLabelText('nativeExtensions.name'), { target: { value: 'only-one' } });
 expect((second.getByLabelText('nativeExtensions.name') as HTMLInputElement).value).toBe('');
 fireEvent.change(second.getByLabelText('nativeExtensions.name'), { target: { value: 'only-two' } });
 expect((first.getByLabelText('nativeExtensions.name') as HTMLInputElement).value).toBe('only-one');
 expect(Object.keys(useNativeExtensions.getState().drafts)).toHaveLength(2);
});

it.each(['within_authorized_root', 'outside_authorized_root', 'unresolved'] as const)('shows linked skills as evidence without creating mutation or file authority (%s)', async status => {
 const entry = { id: 'readonly-link', slug: 'linked-skill', link_path: 'linked-skill', target: status === 'outside_authorized_root' ? '../../outside' : 'source-skill', resolved_target: status === 'within_authorized_root' ? 'source-skill' : null, status };
 invoke.mockImplementation(async (command: string) => {
  if (command === 'discover_native_skills') return { instances: [], readonly_entries: [entry], missing: false, incomplete: false };
  throw 'unexpected command';
 });
 render(<NativeManager kind="skills" />);
 const links = within(await screen.findByRole('region', { name: 'nativeExtensions.readonlyLinks' }));
 expect(links.getAllByText(entry.slug).length).toBeGreaterThan(0);
 expect(links.getAllByText(entry.target).length).toBeGreaterThan(0);
 expect(links.queryByRole('button')).toBeNull();
 expect(links.queryByRole('link')).toBeNull();
 expect(screen.queryByText('nativeExtensions.empty')).toBeNull();
 expect(screen.queryByLabelText('nativeExtensions.source')).toBeNull();
 expect(screen.queryByRole('button', { name: 'nativeExtensions.update' })).toBeNull();
 expect(screen.queryByRole('button', { name: 'nativeExtensions.remove' })).toBeNull();
 expect(screen.queryByRole('button', { name: 'nativeExtensions.disableSkill' })).toBeNull();
 fireEvent.click(links.getAllByText(entry.target)[0]);
 expect(invoke.mock.calls.map(([command]) => command)).toEqual(['discover_native_skills']);
});

describe('native capability manager integration', () => {
 it('moves a market deployment into the manual draft without executing or saving it', async () => {
  render(<NativeManager kind="mcp" />);
  fireEvent.mouseDown(screen.getByRole('tab', { name: 'nativeExtensions.tab.market' }), { button: 0 });
  fireEvent.click(await screen.findByRole('button', { name: 'market.configure' }));
  const program = await screen.findByLabelText('nativeExtensions.program');
  expect((program as HTMLInputElement).value).toBe('npx');
  expect((screen.getByLabelText('nativeExtensions.args') as HTMLTextAreaElement).value).toBe('--yes\n@fixture/server');
  expect((screen.getByLabelText('nativeExtensions.forwarded_env') as HTMLTextAreaElement).value).toBe('API_KEY');
  expect(invoke.mock.calls.every(([command]) => command === 'discover_native_mcp')).toBe(true);
  fireEvent.mouseDown(screen.getByRole('tab', { name: 'nativeExtensions.tab.configured' }), { button: 0 });
  await waitFor(() => expect(screen.getByRole('tab', { name: 'nativeExtensions.tab.configured' }).getAttribute('aria-selected')).toBe('true'));
  fireEvent.mouseDown(screen.getByRole('tab', { name: 'nativeExtensions.tab.manual' }), { button: 0 });
  expect((await screen.findByLabelText('nativeExtensions.program') as HTMLInputElement).value).toBe('npx');
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.previewChange' }));
  await screen.findByRole('button', { name: 'nativeExtensions.confirm' });
  expect(invoke).toHaveBeenCalledWith('preview_native_mcp', { input: expect.objectContaining({ change: { kind: 'upsert', name: 'registry-server', fields: expect.objectContaining({ args: { kind: 'set', value: ['--yes', '@fixture/server'] }, forwarded_env: { kind: 'set', value: ['API_KEY'] } }) } }) });
  expect(invoke).not.toHaveBeenCalledWith('commit_native_operation', expect.anything());
 });

 it('keeps installed search and list scroll while entering and leaving narrow detail', async () => {
  invoke.mockResolvedValue({ instances: [server, { ...server, native_key: 'beta' }], incomplete: false });
  const view = render(<NativeManager kind="mcp" />);
  await screen.findByRole('button', { name: /alpha/ });
  const list = view.container.querySelector('.native-manager-list')!;
  list.scrollTop = 84;
  fireEvent.change(screen.getByLabelText('nativeExtensions.search'), { target: { value: 'beta' } });
  expect(screen.queryByRole('button', { name: /alpha/ })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: /beta/ }));
  await waitFor(() => expect(view.container.querySelector('.native-manager-split')?.getAttribute('data-details')).toBe('true'));
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.backToList' }));
  expect(list.scrollTop).toBe(84);
  expect((screen.getByLabelText('nativeExtensions.search') as HTMLInputElement).value).toBe('beta');
  expect(invoke.mock.calls.filter(([command]) => command === 'discover_native_mcp')).toHaveLength(1);
 });

 it('preserves the actual task workspace identity until a different project is selected', async () => {
  const project = useDevelopment.getState().projects[0];
  useDevelopment.setState({ projects: [project, { ...project, id: 'project-b', name: 'Project B', path: '/tmp/project-b', directory_identity: 'b' }] });
  const entry: NativeSelection = { target: 'Claude', scope: 'ProjectShared', project_id: 'project-a', task_id: 'task-worktree' };
  render(<NativeManager kind="mcp" initialSelection={entry} initialSessionId="origin-session" />);
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('discover_native_mcp', { input: entry }));
  expect(screen.getByText('nativeExtensions.taskWorkspace')).toBeTruthy();
  expect(screen.queryByText('/tmp/project-a')).toBeNull();
  await choose('nativeExtensions.target', 'Codex');
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('discover_native_mcp', { input: { ...entry, target: 'Codex' } }));
  await choose('nativeExtensions.project', 'Project B');
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('discover_native_mcp', { input: { target: 'Codex', scope: 'ProjectShared', project_id: 'project-b' } }));
 });

 it('blocks child operations on close and context switches, then closes cleanly through the public handle', async () => {
  const ref = createRef<NativeManagerHandle>(), onClose = vi.fn();
  render(<NativeManager ref={ref} kind="mcp" onClose={onClose} />);
  childState.blocked = true;
  await act(async () => expect(await ref.current!.requestClose()).toBe(false));
  expect(onClose).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('combobox', { name: 'nativeExtensions.target' }));
  fireEvent.click(await screen.findByRole('option', { name: 'Claude Code' }));
  await act(async () => {});
  expect(screen.getByRole('combobox', { name: 'nativeExtensions.target' }).textContent).toContain('Codex');
  childState.blocked = false;
  await act(async () => expect(await ref.current!.requestClose()).toBe(true));
  expect(onClose).toHaveBeenCalledTimes(1);
 });

 it('confirms before a Registry deployment replaces an existing unsaved manual draft', async () => {
  render(<><NativeManager kind="mcp" /><ModalHost /></>);
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.createServer' }));
  const name = await screen.findByLabelText('nativeExtensions.name');
  fireEvent.change(name, { target: { value: 'keep-manual' } });
  fireEvent.mouseDown(screen.getByRole('tab', { name: 'nativeExtensions.tab.market' }), { button: 0 });
  fireEvent.click(await screen.findByRole('button', { name: 'market.configure' }));
  const confirmation = await screen.findByRole('button', { name: 'shell.discard' });
  expect(Object.values(useNativeExtensions.getState().drafts)[0].name).toBe('keep-manual');
  fireEvent.click(confirmation);
  await waitFor(() => expect((screen.getByLabelText('nativeExtensions.name') as HTMLInputElement).value).toBe('registry-server'));
  expect(invoke.mock.calls.some(([command]) => command === 'preview_native_mcp' || command === 'commit_native_operation')).toBe(false);
 });

 it('collapses the preview before dismissing and blocks unresolved operations without replay', async () => {
  const ref = createRef<NativeManagerHandle>(), onClose = vi.fn();
  render(<NativeManager ref={ref} kind="mcp" onClose={onClose} />);
  await createServer();
  await act(async () => expect(await ref.current!.requestClose()).toBe(false));
  expect(screen.queryByRole('region', { name: 'nativeExtensions.preview' })).toBeNull();
  await act(async () => expect(await ref.current!.requestClose()).toBe(false));
  expect(onClose).not.toHaveBeenCalled();
  expect(screen.getByText('nativeExtensions.closeBlocked')).toBeTruthy();
  expect(invoke).not.toHaveBeenCalledWith('commit_native_operation', expect.anything());
  expect(invoke).not.toHaveBeenCalledWith('cancel_native_operation', expect.anything());
 });

 it('CAS-protects inactive source drafts and only discards owned selection snapshots', async () => {
  const ref = createRef<NativeManagerHandle>(), onClose = vi.fn();
  const selection: NativeSelection = { target: 'Codex', scope: 'OsUser', project_id: null };
  const other: NativeSelection = { target: 'Claude', scope: 'OsUser', project_id: null };
  render(<><NativeManager ref={ref} kind="skills" onClose={onClose} /><ModalHost /></>);
  act(() => { useNativeSkillLibrary.getState().setDraft(selection, { input: 'owner/before', sourceId: null, revision: null }); useNativeSkillLibrary.getState().setDraft(other, { input: 'owner/unrelated', sourceId: null, revision: null }); });
  let closing!: Promise<boolean>;
  act(() => { closing = ref.current!.requestClose(); });
  await screen.findByRole('button', { name: 'shell.discard' });
  act(() => useNativeSkillLibrary.getState().setDraft(selection, { input: 'owner/newer', sourceId: null, revision: null }));
  fireEvent.click(screen.getByRole('button', { name: 'shell.discard' }));
  await act(async () => expect(await closing).toBe(false));
  expect(onClose).not.toHaveBeenCalled();
  act(() => { closing = ref.current!.requestClose(); });
  fireEvent.click(await screen.findByRole('button', { name: 'shell.discard' }));
  await act(async () => expect(await closing).toBe(true));
  expect(useNativeSkillLibrary.getState().drafts[nativeLibraryKey(selection)]).toBeUndefined();
  expect(useNativeSkillLibrary.getState().drafts[nativeLibraryKey(other)].input).toBe('owner/unrelated');
  expect(onClose).toHaveBeenCalledTimes(1);
 });
});

describe('native plugin drawer summaries', () => {
 const selection: NativeSelection = { target: 'Codex', scope: 'ProjectShared', project_id: 'project-a', task_id: 'task-worktree' };
 it('reads the actual selection, shows MCP first, and never observes or starts a session', async () => {
  invoke.mockImplementation(async (command: string) => command === 'discover_native_mcp' ? { instances: [server], incomplete: false } : { instances: [skill], readonly_entries: [], missing: false, incomplete: false });
  const onManage = vi.fn();
  render(<NativePluginsDrawer selection={selection} sessionId="session-a" onManage={onManage} />);
  await screen.findByText('alpha'); await screen.findByText('Sample skill');
  const sections = screen.getAllByRole('region');
  expect(sections.map(section => section.getAttribute('aria-label'))).toEqual(['nativeExtensions.configuredServers', 'nativeExtensions.installedSkills']);
  expect(invoke).toHaveBeenCalledWith('discover_native_mcp', { input: selection });
  expect(invoke).toHaveBeenCalledWith('discover_native_skills', { input: { ...selection, query: '' } });
  expect(invoke.mock.calls.map(([command]) => command).sort()).toEqual(['discover_native_mcp', 'discover_native_skills']);
  fireEvent.click(within(sections[0]).getByRole('button', { name: 'nativeExtensions.manage' }));
  expect(onManage).toHaveBeenCalledWith('mcp');
 });

 it('does not turn loading or unreadable data into an empty resource collection', async () => {
  const deferred = Promise.withResolvers<unknown>();
  invoke.mockImplementation(() => deferred.promise);
  render(<NativePluginsDrawer selection={selection} onManage={vi.fn()} />);
  expect(screen.getAllByText('nativeExtensions.loading').length).toBe(2);
  expect(screen.queryByText('nativeExtensions.noConfiguredServers')).toBeNull();
  await act(async () => deferred.reject('native_extension_AccessDenied'));
  expect(screen.getAllByRole('alert')).toHaveLength(2);
  expect(screen.queryByText('nativeExtensions.noConfiguredServers')).toBeNull();
  expect(screen.queryByText('nativeExtensions.noInstalledSkills')).toBeNull();
 });

 it('keeps OS-user resources out of an empty project summary without changing its location', async () => {
  invoke.mockImplementation(async (command: string, args: { input: NativeSelection }) => {
   const user = args.input.scope === 'OsUser';
   return command === 'discover_native_mcp' ? { instances: user ? [server] : [], incomplete: false } : { instances: user ? [skill] : [], readonly_entries: [], missing: !user, incomplete: false };
  });
  const user: NativeSelection = { target: 'Codex', scope: 'OsUser', project_id: null };
  await useNativeExtensions.getState().discoverMcp(user);
  await useNativeExtensions.getState().discoverSkills({ ...user, query: '' });
  invoke.mockClear();
  render(<NativePluginsDrawer selection={selection} onManage={vi.fn()} />);
  await screen.findByText('nativeExtensions.noConfiguredServers');
  await screen.findByText('nativeExtensions.noInstalledSkills');
  expect(screen.getAllByLabelText('nativeExtensions.count').map(node => node.textContent)).toEqual(['0', '0']);
  expect(screen.queryByText('alpha')).toBeNull();
  expect(screen.queryByText('Sample skill')).toBeNull();
  expect(screen.getByText('Codex · nativeExtensions.scope.ProjectShared')).toBeTruthy();
  expect(invoke.mock.calls.every(([, args]) => args.input.scope === 'ProjectShared')).toBe(true);
 });

 it('does not present a cached zero as the current count while refreshing or after a read failure', async () => {
  invoke.mockImplementation(async (command: string) => command === 'discover_native_mcp' ? { instances: [], incomplete: false } : { instances: [], readonly_entries: [], missing: true, incomplete: false });
  const first = render(<NativePluginsDrawer selection={selection} onManage={vi.fn()} />);
  await screen.findByText('nativeExtensions.noConfiguredServers');
  await screen.findByText('nativeExtensions.noInstalledSkills');
  expect(screen.getAllByLabelText('nativeExtensions.count').map(node => node.textContent)).toEqual(['0', '0']);
  first.unmount();
  const deferred = Promise.withResolvers<unknown>();
  invoke.mockImplementation(() => deferred.promise);
  render(<NativePluginsDrawer selection={selection} onManage={vi.fn()} />);
  expect(screen.getAllByLabelText('nativeExtensions.count').map(node => node.textContent)).toEqual(['—', '—']);
  await act(async () => deferred.reject('native_extension_AccessDenied'));
  expect(screen.getAllByRole('alert')).toHaveLength(2);
  expect(screen.getAllByLabelText('nativeExtensions.count').map(node => node.textContent)).toEqual(['—', '—']);
  expect(screen.getAllByText('nativeExtensions.stale')).toHaveLength(2);
  expect(screen.queryByText('nativeExtensions.noConfiguredServers')).toBeNull();
  expect(screen.queryByText('nativeExtensions.noInstalledSkills')).toBeNull();
 });

 it('does not present an incomplete empty snapshot as an exact zero', async () => {
  invoke.mockImplementation(async (command: string) => command === 'discover_native_mcp' ? { instances: [], incomplete: true } : { instances: [], readonly_entries: [], missing: false, incomplete: true });
  render(<NativePluginsDrawer selection={selection} onManage={vi.fn()} />);
  await waitFor(() => expect(screen.getAllByText('nativeExtensions.incomplete')).toHaveLength(2));
  expect(screen.getAllByLabelText('nativeExtensions.count').map(node => node.textContent)).toEqual(['—', '—']);
  expect(screen.queryByText('nativeExtensions.noConfiguredServers')).toBeNull();
  expect(screen.queryByText('nativeExtensions.noInstalledSkills')).toBeNull();
 });

 it('refreshes cached rows on every open and retains stale/error evidence instead of a false empty state', async () => {
  invoke.mockImplementation(async (command: string) => command === 'discover_native_mcp' ? { instances: [server], incomplete: true } : { instances: [skill], readonly_entries: [], missing: false, incomplete: false });
  const first = render(<NativePluginsDrawer selection={selection} onManage={vi.fn()} />);
  await screen.findByText('alpha'); await screen.findByText('Sample skill');
  first.unmount();
  const deferred = Promise.withResolvers<unknown>();
  invoke.mockClear(); invoke.mockImplementation(() => deferred.promise);
  render(<NativePluginsDrawer selection={selection} onManage={vi.fn()} />);
  expect(screen.getByText('alpha')).toBeTruthy(); expect(screen.getByText('Sample skill')).toBeTruthy();
  expect(screen.getAllByText('nativeExtensions.refreshingCached')).toHaveLength(2);
  expect(screen.getByText('nativeExtensions.incomplete')).toBeTruthy();
  expect(screen.queryByText('nativeExtensions.noConfiguredServers')).toBeNull();
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(['discover_native_mcp', 'discover_native_skills']);
  await act(async () => deferred.reject(new Error('transport unavailable')));
  expect(screen.getAllByText('nativeExtensions.stale')).toHaveLength(2);
  expect(screen.getByText('alpha')).toBeTruthy(); expect(screen.getByText('Sample skill')).toBeTruthy();
 });
});
