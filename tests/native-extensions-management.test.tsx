import { cleanup, fireEvent, render, screen, waitFor, act, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import NativeManager from '../src/components/nativeExtensions/NativeManager';
import { useDevelopment } from '../src/stores/developmentStore';
import { useNativeExtensions } from '../src/stores/nativeExtensions';
import type { NativeMcpInstance, NativeReceipt, NativeSkill, NativeSkillPreview } from '../src/types/nativeExtensions';
import { ModalHost } from '@dgos/ui';
import { SurfaceScope } from '../src/workspace/SurfaceContext';
import { createWorkspaceController } from '../src/workspace/controller';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ t: (key: string) => key, useLocale: () => ({ locale: 'en-US', t: (key: string) => key }) }));
vi.mock('../src/components/nativeExtensions/NativeSessionPanel', () => ({ default: () => null }));

const skill: NativeSkill = { instance: { id: 'skill-1', revision: 'rev-1' }, target: 'Codex', scope: 'OsUser', slug: 'sample-skill', owner: 'Loose', metadata: { name: 'Sample skill', description: 'Installed description', license: null, dependencies: { declared: true, tools: [], packages: [], required_tools: [] }, dynamic_commands: false }, resources: [], source: { source_hash: 'installed-hash', commit: 'installed-commit' }, can_update: true };
const receipt: NativeReceipt = { operation_id: 'preview-operation', state: 'Previewed', pending_activation: false, recovery_material_retained: false };
const basePreview = { receipt, target: 'Codex' as const, scope: 'OsUser' as const, expires_at: 4102444800 };
const skillPreview: NativeSkillPreview = { ...basePreview, project_id: null, slug: 'returned-slug', before_hash: 'before-hash', after_hash: 'after-hash', source: { source_hash: 'returned-source-hash', commit: 'returned-commit' }, metadata: { ...skill.metadata, name: 'Authoritative package', description: 'Returned description', license: 'MIT', dynamic_commands: true, dependencies: { declared: true, tools: [{ program: 'python', version_requirement: '>=3' }], packages: ['sample-package'], required_tools: ['sample-tool'] } }, resources: [{ path: 'scripts/inspect.py', hash: 'script-hash', bytes: 42, executable: true }] };
const server: NativeMcpInstance = { native_key: 'alpha', transport: 'Stdio', enabled: true, owner: 'Loose', has_auth_configuration: true, unknown_fields_present: true, authentication: { configured: true, credential_present: null, authenticated: null, connected: null, approved: null, active_session: null, login: 'Unsupported', logout: 'Unsupported', delete_credentials: 'Unsupported' } };

async function choose(label: string, option: string) {
  fireEvent.click(screen.getByRole('combobox', { name: label }));
  fireEvent.click(await screen.findByRole('option', { name: option }));
}
async function createServer() {
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.createServer' }));
  fireEvent.change(screen.getByLabelText('nativeExtensions.name'), { target: { value: 'new-server' } });
  fireEvent.change(screen.getByLabelText('nativeExtensions.program'), { target: { value: 'node' } });
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.previewChange' }));
  await screen.findByRole('button', { name: 'nativeExtensions.confirm' });
}

beforeEach(() => {
  invoke.mockReset();
  useDevelopment.setState({ epoch: 0, loaded: true, projects: [{ id: 'project-a', owner: 'fixture', name: 'Project A', path: '/tmp/project-a', directory_identity: 'a', archived: false, created_at: 1, last_opened_at: null }], tasks: [], sessions: [], selection: { project_id: null, task_id: null, session_id: null }, operations: [] });
  useNativeExtensions.getState().reset();
  invoke.mockImplementation(async (command: string) => {
    if (command === 'discover_native_skills') return { instances: [skill], readonly_entries: [], missing: false, incomplete: false };
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
  it('installs with a populated list and retains authoritative package preview details', async () => {
    render(<NativeManager kind="skills" />);
    await screen.findAllByText('Sample skill');
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.installSkill' }));
    expect(screen.getAllByLabelText('nativeExtensions.source')).toHaveLength(1);
    fireEvent.change(screen.getByLabelText('nativeExtensions.slug'), { target: { value: 'requested-slug' } });
    fireEvent.change(screen.getByLabelText('nativeExtensions.source'), { target: { value: 'owner/repo/tree/main/skill' } });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.previewInstall' }));
    const preview = within(await screen.findByRole('region', { name: 'nativeExtensions.preview' }));
    for (const text of ['Authoritative package', 'Returned description', 'returned-slug', 'before-hash', 'after-hash', 'returned-source-hash', 'returned-commit']) expect(preview.getByText(text)).toBeTruthy();
    expect(preview.getByText(/python >=3, sample-package, sample-tool/)).toBeTruthy();
    expect(preview.getByText('nativeExtensions.scriptsNotRun')).toBeTruthy();
    expect(preview.getByText('nativeExtensions.expires')).toBeTruthy();
    expect(invoke).toHaveBeenCalledWith('preview_native_skill', { input: { target: 'Codex', scope: 'OsUser', project_id: null, action: 'install', slug: 'requested-slug', expected: null, source: { kind: 'github', input: 'owner/repo/tree/main/skill' } } });
    expect(invoke).not.toHaveBeenCalledWith('commit_native_operation', expect.anything());
  });

  it('updates the selected revision with an explicit source and removes without a source', async () => {
    render(<NativeManager kind="skills" />);
    await screen.findAllByText('Sample skill');
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.update' }));
    const source = screen.getByLabelText('nativeExtensions.source');
    expect(source.getAttribute('aria-invalid')).toBe('true');
    expect(source.getAttribute('aria-describedby')).toBeTruthy();
    expect(invoke).not.toHaveBeenCalledWith('preview_native_skill', expect.anything());
    fireEvent.change(source, { target: { value: 'owner/repo/tree/commit/skill' } });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.update' }));
    await screen.findByRole('button', { name: 'nativeExtensions.confirm' });
    expect(invoke).toHaveBeenCalledWith('preview_native_skill', { input: expect.objectContaining({ action: 'update', slug: skill.slug, expected: skill.instance, source: { kind: 'github', input: 'owner/repo/tree/commit/skill' } }) });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.cancel' }));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'nativeExtensions.confirm' })).toBeNull());
    fireEvent.change(source, { target: { value: '' } });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.remove' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('preview_native_skill', { input: expect.objectContaining({ action: 'remove', slug: skill.slug, expected: skill.instance, source: null }) }));
  });

  it('previews exact-instance visibility and shows returned affected scopes without consuming an update draft', async () => {
    invoke.mockImplementation(async (command: string) => command === 'preview_native_skill_visibility'
      ? { preview: basePreview, coverage: 'all-known-scopes', affected: [{ selection: { target: 'Claude', scope: 'ProjectShared', project_id: 'project-a' }, instance: { id: 'other-instance', revision: 'other-revision' }, slug: 'same-native-name' }] }
      : command === 'commit_native_operation' ? { ...receipt, state: 'Committed' }
        : { instances: [skill], readonly_entries: [], missing: false, incomplete: false });
    render(<NativeManager kind="skills" />);
    await screen.findAllByText('Sample skill');
    fireEvent.change(screen.getByLabelText('nativeExtensions.source'), { target: { value: 'keep-this-update-source' } });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.disableSkill' }));
    const preview = within(await screen.findByRole('region', { name: 'nativeExtensions.preview' }));
    expect(preview.getByText(/same-native-name.*Claude.*nativeExtensions.scope.ProjectShared.*project-a.*other-instance.*other-revision/)).toBeTruthy();
    expect(preview.getByText(/all-known-scopes/)).toBeTruthy();
    expect(invoke).toHaveBeenCalledWith('preview_native_skill_visibility', { input: { target: 'Codex', scope: 'OsUser', project_id: null, instance: skill.instance, visible: false } });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.confirm' }));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'nativeExtensions.confirm' })).toBeNull());
    expect((screen.getByLabelText('nativeExtensions.source') as HTMLInputElement).value).toBe('keep-this-update-source');
  });

  it('does not let a late preview mark a newly selected scope busy and restores its receipt on return', async () => {
    const deferred = Promise.withResolvers<unknown>();
    invoke.mockImplementation((command: string) => command === 'preview_native_skill' ? deferred.promise : Promise.resolve({ instances: [skill], readonly_entries: [], missing: false, incomplete: false }));
    render(<NativeManager kind="skills" />);
    await screen.findAllByText('Sample skill');
    fireEvent.change(screen.getByLabelText('nativeExtensions.source'), { target: { value: 'owner/repo' } });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.update' }));
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
    fireEvent.change(screen.getByLabelText('nativeExtensions.program'), { target: { value: 'alpha-command' } });
    fireEvent.click(screen.getByRole('button', { name: /beta/ }));
    expect((screen.getByLabelText('nativeExtensions.program') as HTMLInputElement).value).toBe('');
    fireEvent.click(screen.getByRole('button', { name: /alpha/ }));
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
    await choose('nativeExtensions.target', 'Claude');
    await screen.findByLabelText('nativeExtensions.header_env');
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
    fireEvent.change(screen.getByLabelText('nativeExtensions.name'), { target: { value: 'old-account-server' } });
    fireEvent.change(screen.getByLabelText('nativeExtensions.program'), { target: { value: 'node' } });
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.previewChange' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('preview_native_mcp', expect.anything()));
    act(() => useDevelopment.setState({ epoch: 1 }));
    await act(async () => { deferred.resolve(basePreview); });
    expect(screen.queryByRole('button', { name: 'nativeExtensions.confirm' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.createServer' }));
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
 fireEvent.change(screen.getByLabelText('nativeExtensions.name'), { target: { value: 'keep-native-draft' } });
 view.rerender(<NativeManager kind="mcp" initialSelection={{ ...entry }} initialSessionId="origin-session" />);
 expect(screen.getByRole('combobox', { name: 'nativeExtensions.target' }).textContent).toContain('Codex');
 expect((screen.getByLabelText('nativeExtensions.name') as HTMLInputElement).value).toBe('keep-native-draft');
 expect(invoke.mock.calls.some(([name]) => name === 'preview_native_mcp' || name === 'commit_native_operation' || name === 'native_session_action')).toBe(false);
});

it('discards only the closing surface drafts across its selections and preserves other domains and surfaces', async () => {
 const controller = createWorkspaceController();
 render(<>
  <SurfaceScope id="skills-a" active controller={controller}><div data-testid="skills-a"><NativeManager kind="skills" /></div></SurfaceScope>
  <SurfaceScope id="skills-b" active controller={controller}><div data-testid="skills-b"><NativeManager kind="skills" /></div></SurfaceScope>
  <SurfaceScope id="mcp" active controller={controller}><div data-testid="mcp"><NativeManager kind="mcp" /></div></SurfaceScope>
  <ModalHost />
 </>);
 const first = within(screen.getByTestId('skills-a')), second = within(screen.getByTestId('skills-b')), mcp = within(screen.getByTestId('mcp'));
 await first.findByLabelText('nativeExtensions.source');
 fireEvent.change(first.getByLabelText('nativeExtensions.source'), { target: { value: 'first-os-draft' } });
 fireEvent.click(first.getByRole('combobox', { name: 'nativeExtensions.scope' }));
 fireEvent.click(await screen.findByRole('option', { name: 'nativeExtensions.scope.ProjectShared' }));
 await waitFor(() => expect(invoke).toHaveBeenCalledWith('discover_native_skills', { input: { target: 'Codex', scope: 'ProjectShared', project_id: 'project-a', query: '' } }));
 fireEvent.change(first.getByLabelText('nativeExtensions.source'), { target: { value: 'first-project-draft' } });
 fireEvent.change(second.getByLabelText('nativeExtensions.source'), { target: { value: 'second-surface-draft' } });
 fireEvent.click(mcp.getByRole('button', { name: 'nativeExtensions.createServer' }));
 fireEvent.change(mcp.getByLabelText('nativeExtensions.name'), { target: { value: 'other-domain-draft' } });
 expect(Object.keys(useNativeExtensions.getState().drafts)).toHaveLength(4);
 let leave!: Promise<boolean>;
 act(() => { leave = controller.canLeave('skills-a'); });
 fireEvent.change(mcp.getByLabelText('nativeExtensions.name'), { target: { value: 'newer-other-domain-draft' } });
 fireEvent.click(await screen.findByRole('button', { name: 'shell.discard' }));
 await act(async () => { expect(await leave).toBe(true); });
 expect(Object.keys(useNativeExtensions.getState().drafts)).toHaveLength(2);
 expect(controller.getSurfaceStatuses()['skills-a'].dirty).toBe(false);
 expect(controller.getSurfaceStatuses()['skills-b'].dirty).toBe(true);
 expect(controller.getSurfaceStatuses().mcp.dirty).toBe(true);
 expect((second.getByLabelText('nativeExtensions.source') as HTMLInputElement).value).toBe('second-surface-draft');
 expect((mcp.getByLabelText('nativeExtensions.name') as HTMLInputElement).value).toBe('newer-other-domain-draft');
});

it('does not close over a draft edited while its own discard confirmation is pending', async () => {
 const controller = createWorkspaceController();
 render(<><SurfaceScope id="native" active controller={controller}><NativeManager kind="skills" /></SurfaceScope><ModalHost /></>);
 const source = await screen.findByLabelText('nativeExtensions.source');
 fireEvent.change(source, { target: { value: 'before-confirm' } });
 let leave!: Promise<boolean>;
 act(() => { leave = controller.canLeave('native'); });
 fireEvent.change(source, { target: { value: 'after-confirm' } });
 fireEvent.click(await screen.findByRole('button', { name: 'shell.discard' }));
 await act(async () => { expect(await leave).toBe(false); });
 expect((source as HTMLInputElement).value).toBe('after-confirm');
 expect(controller.getSurfaceStatuses().native.dirty).toBe(true);
});

it('keeps standalone managers of the same domain in separate draft owners', async () => {
 render(<><div data-testid="one"><NativeManager kind="mcp" /></div><div data-testid="two"><NativeManager kind="mcp" /></div></>);
 const first = within(screen.getByTestId('one')), second = within(screen.getByTestId('two'));
 fireEvent.click(first.getByRole('button', { name: 'nativeExtensions.createServer' }));
 fireEvent.click(second.getByRole('button', { name: 'nativeExtensions.createServer' }));
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
