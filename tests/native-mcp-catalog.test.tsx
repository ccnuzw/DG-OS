import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import NativeMcpMarket from '../src/components/nativeExtensions/NativeMcpMarket';
import NativeMcpProbe, { nativeDraftProbeInput } from '../src/components/nativeExtensions/NativeMcpProbe';
import { nativeCatalogKey, nativeProbeKey, registryDeployments, registryParameters, registryToDraft, useNativeMcpCatalog } from '../src/stores/nativeMcpCatalog';
import { useDevelopment } from '../src/stores/developmentStore';
import type { NativeDraft, NativeSelection } from '../src/types/nativeExtensions';
import type { NativeProbeSnapshot, RegistryEntry, RegistryPage } from '../src/types/nativeMcpCatalog';
import { ModalHost } from '@dgos/ui';
import { NativeManagerLifecycle, type NativeManagerGuard } from '../src/components/nativeExtensions/NativeManagerLifecycle';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ t: (key: string) => key, useLocale: () => ({ locale: 'en-US', t: (key: string) => key }) }));
const selection: NativeSelection = { target: 'Codex', scope: 'ProjectShared', project_id: 'project' };
const entry: RegistryEntry = { server: { name: 'io.example/server', description: 'Published metadata', version: '1.2.3', remotes: [{ type: 'streamable-http', url: 'https://example.com/{region}/mcp', variables: { region: { isRequired: true, choices: ['eu', 'us'], default: 'eu' } }, headers: [{ name: 'Authorization', isRequired: true, isSecret: true }] }] }, _meta: { 'io.modelcontextprotocol.registry/official': { status: 'active', isLatest: true } } };
const page: RegistryPage = { servers: [entry], next_cursor: 'opaque/cursor?value', fetched_at: 123, stale: false, error: null, source: 'https://registry.modelcontextprotocol.io/v0/servers' };
const draft: NativeDraft = { name: 'test', transport: 'Stdio', program: 'server', args: '  path with spaces  \n\nlast', cwd: '', url: '', bearer_env: '', forwarded_env: 'NATIVE_TOKEN', header_env: '', slug: '', source: '', modes: {} };
const snapshot: NativeProbeSnapshot = { probe_id: 'original', selection, config: { name: 'test', transport: 'Stdio', program: 'server', args: ['path with spaces'], url: null, cwd: '/workspace', credential_references: ['env:NATIVE_TOKEN'], redacted: false, config_digest: 'native-preimage' }, digest: 'exact-approved-digest', executable: '/trusted/server', cwd: '/workspace', root: '/workspace', platform_identity: 'cli-hash', executable_identity: 'server-hash', expires_at: 4102444800, state: 'ready', phase: 'approval', cleanup: 'not_started', error: null, observed_at: null, protocol_version: null, tools: [], schema_source: 'independent_host_probe', may_download_execute: true, probe_args: ['path with spaces'], cleanup_plan: null };
beforeEach(() => {
  invoke.mockReset();
  useDevelopment.setState({ epoch: 0, workspaceOwner: 'fixture', projects: [{ id: 'project', owner: 'fixture', name: 'Project', path: '/workspace', directory_identity: 'root-a', archived: false, created_at: 1, last_opened_at: null }], tasks: [], sessions: [] });
  useNativeMcpCatalog.setState({ catalogs: {}, probes: {}, busy: {} });
});
afterEach(cleanup);

describe('official Registry deployment boundary', () => {
  it('creates only an unsaved draft with host credential references', () => {
    const deployment = registryDeployments(entry)[0];
    const fields = registryParameters(deployment);
    expect(fields.find(field => field.key === 'header:0')?.reference).toBe(true);
    const result = registryToDraft(entry, deployment, { 'url:region': 'eu', 'header:0': 'NATIVE_AUTHORIZATION' }, selection);
    expect(result).toMatchObject({ transport: 'Http', url: 'https://example.com/eu/mcp', header_env: 'Authorization=NATIVE_AUTHORIZATION', modes: { transport: 'set', url: 'set', header_env: 'set' } });
    expect(invoke).not.toHaveBeenCalled();
  });
  it('keeps repeated argv entries and forwards native Claude env references', () => {
    const published: RegistryEntry = { ...entry, server: { ...entry.server, remotes: [], packages: [{ registryType: 'npm', identifier: '@example/server', version: '4.5.6', transport: { type: 'stdio' }, packageArguments: [{ type: 'named', name: '--path', isRepeated: true, isRequired: true }], environmentVariables: [{ name: 'NATIVE_TOKEN', isRequired: true, isSecret: true }] }] } };
    const result = registryToDraft(published, registryDeployments(published)[0], { 'package:0': '/a path\n/b path', 'env:0': 'NATIVE_TOKEN' }, { ...selection, target: 'Claude' });
    expect(result.program).toBe('npx');
    expect(result.args.split('\n')).toEqual(['--yes', '@example/server@4.5.6', '--path', '/a path', '--path', '/b path']);
    expect(result.forwarded_env).toBe('NATIVE_TOKEN'); expect(result.modes.forwarded_env).toBe('set');
  });
  it('does not accept secret argv or pretend unenforced artifact integrity is verified', () => {
    const published: RegistryEntry = { ...entry, server: { ...entry.server, remotes: [], packages: [{ registryType: 'npm', identifier: 'package', transport: { type: 'stdio' }, packageArguments: [{ type: 'positional', isSecret: true }] }] } };
    expect(() => registryToDraft(published, registryDeployments(published)[0], { 'package:0': 'must-not-be-saved' }, selection)).toThrow('secretArgument');
    published.server.packages![0].fileSha256 = 'a'.repeat(64);
    expect(() => registryToDraft(published, registryDeployments(published)[0], {}, selection)).toThrow('integrityRequired');
    expect(invoke).not.toHaveBeenCalled();
  });
  it('refuses a package registry endpoint it cannot preserve in the generated command', () => {
    const published: RegistryEntry = { ...entry, server: { ...entry.server, remotes: [], packages: [{ registryType: 'oci', registryBaseUrl: 'https://private.example', identifier: 'server', version: '1.0.0', transport: { type: 'stdio' } }] } };
    expect(() => registryToDraft(published, registryDeployments(published)[0], {}, selection)).toThrow('unsupportedDeployment');
    expect(invoke).not.toHaveBeenCalled();
  });
  it('fetches only after explicit action and retains prior results after failure', async () => {
    render(<NativeMcpMarket selection={selection} disabled={false} onConfigure={vi.fn()} />);
    expect(invoke).not.toHaveBeenCalled();
    invoke.mockResolvedValueOnce(page).mockRejectedValueOnce('registry_offline');
    fireEvent.change(screen.getByLabelText('nativeMcpCatalog.searchLabel'), { target: { value: 'server' } });
    fireEvent.click(screen.getByRole('button', { name: 'nativeMcpCatalog.search' }));
    await screen.findByText('Published metadata');
    fireEvent.click(screen.getByRole('button', { name: 'nativeMcpCatalog.search' }));
    await screen.findByRole('alert');
    expect(screen.getByText('Published metadata')).toBeTruthy();
    expect(useNativeMcpCatalog.getState().catalogs[nativeCatalogKey(selection)].page?.stale).toBe(true);
  });
  it('keeps opaque cursor and original search filters on the next page', async () => {
    invoke.mockResolvedValueOnce(page).mockResolvedValueOnce({ ...page, servers: [], next_cursor: null });
    await useNativeMcpCatalog.getState().search(selection, 'server');
    await useNativeMcpCatalog.getState().search(selection, 'server', true);
    expect(invoke).toHaveBeenLastCalledWith('search_native_mcp_registry', { input: { selection, search: 'server', cursor: 'opaque/cursor?value', refresh: true } });
    expect(useNativeMcpCatalog.getState().catalogs[nativeCatalogKey(selection)].page?.servers).toEqual([entry]);
  });
  it('discards a delayed read when the same project id acquires another root identity', async () => {
    let finish!: (value: RegistryPage) => void;
    invoke.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    const request = useNativeMcpCatalog.getState().search(selection, 'server');
    useDevelopment.setState(state => ({ projects: state.projects.map(project => ({ ...project, directory_identity: 'root-b' })) }));
    finish(page); await request;
    expect(useNativeMcpCatalog.getState().catalogs[nativeCatalogKey(selection)]).toBeUndefined();
  });
  it('retains Registry parameters after its child discard consent when the enclosing transition is abandoned', async () => {
    invoke.mockResolvedValue(page);
    await useNativeMcpCatalog.getState().search(selection, 'server');
    const guards = new Set<NativeManagerGuard>();
    const register = (guard: NativeManagerGuard) => { guards.add(guard); return () => { guards.delete(guard); }; };
    render(<><NativeManagerLifecycle.Provider value={register}><NativeMcpMarket selection={selection} disabled={false} onConfigure={vi.fn()} /></NativeManagerLifecycle.Provider><ModalHost /></>);
    fireEvent.click(screen.getByRole('button', { name: /Published metadata/ }));
    fireEvent.click(await screen.findByRole('combobox', { name: 'nativeMcpCatalog.deployment' }));
    fireEvent.click(await screen.findByRole('option', { name: 'streamable-http · https://example.com/{region}/mcp' }));
    const reference = screen.getByLabelText('Authorization *');
    fireEvent.change(reference, { target: { value: 'KEEP_THIS_REFERENCE' } });
    let decision!: boolean | Promise<boolean>;
    act(() => { decision = [...guards][0]('close'); });
    fireEvent.click(await screen.findByRole('button', { name: 'shell.discard' }));
    await act(async () => expect(await decision).toBe(true));
    expect((reference as HTMLInputElement).value).toBe('KEEP_THIS_REFERENCE');
  });
});

describe('independent approved probe boundary', () => {
  it('preserves exact unsaved argv and never runs a probe on mount', () => {
    const input = nativeDraftProbeInput(selection, draft);
    expect(input?.source).toMatchObject({ kind: 'draft', draft: { args: ['  path with spaces  ', '', 'last'], forwarded_env: ['NATIVE_TOKEN'] } });
    render(<NativeMcpProbe selection={selection} draft={draft} />);
    expect(invoke).not.toHaveBeenCalled();
  });
  it('pins approval, keeps unknown receipts and inspects without replay', async () => {
    invoke.mockResolvedValueOnce(snapshot).mockRejectedValueOnce('native_extension_Unavailable').mockResolvedValueOnce({ ...snapshot, state: 'running', phase: 'handshake', cleanup: 'pending' });
    const input = nativeDraftProbeInput(selection, draft)!;
    await useNativeMcpCatalog.getState().preview(input);
    const key = nativeProbeKey(input);
    await useNativeMcpCatalog.getState().operation(selection, 'run', key);
    const current = useNativeMcpCatalog.getState().probes[key];
    expect(current.snapshot.state).toBe('unknown');
    expect(invoke).toHaveBeenNthCalledWith(2, 'run_native_mcp_probe', { input: { probe_id: 'original', approved_digest: 'exact-approved-digest' } });
    expect((await useNativeMcpCatalog.getState().operation(selection, 'run', key)).ok).toBe(false);
    await useNativeMcpCatalog.getState().operation(selection, 'inspect', key);
    expect(invoke).toHaveBeenLastCalledWith('inspect_native_mcp_probe', { input: { probe_id: 'original' } });
    expect(invoke.mock.calls.filter(([command]) => command === 'run_native_mcp_probe')).toHaveLength(1);
  });
  it('keeps original inspection available after root drift but never shows it as current approval', async () => {
    const input = nativeDraftProbeInput(selection, draft)!, originalKey = nativeProbeKey(input);
    useNativeMcpCatalog.setState({ probes: { [originalKey]: { snapshot: { ...snapshot, state: 'unknown', cleanup: 'unknown' }, input, error: null } } });
    useDevelopment.setState(state => ({ projects: state.projects.map(project => ({ ...project, path: '/other', directory_identity: 'root-b' })) }));
    invoke.mockResolvedValueOnce({ ...snapshot, state: 'failed', cleanup: 'confirmed', error: 'native_extension_Conflict' });
    render(<NativeMcpProbe selection={selection} draft={draft} />);
    expect(screen.getByText('nativeMcpCatalog.expiredResult')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'nativeMcpCatalog.inspectProbe' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('inspect_native_mcp_probe', { input: { probe_id: 'original' } }));
    expect(invoke.mock.calls.some(([command]) => command === 'run_native_mcp_probe')).toBe(false);
  });
  it('does not carry probe approval across account epochs', async () => {
    let finish!: (value: NativeProbeSnapshot) => void;
    invoke.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    const pending = useNativeMcpCatalog.getState().preview(nativeDraftProbeInput(selection, draft)!);
    await act(async () => { useDevelopment.setState(state => ({ epoch: state.epoch + 1 })); finish(snapshot); await pending; });
    expect(useNativeMcpCatalog.getState().probes).toEqual({});
  });
  it('does not present one configured server result as another server or an unsaved HTTP deployment', async () => {
    const original = { selection, source: { kind: 'existing' as const, server_name: 'configured-server' } };
    invoke.mockResolvedValueOnce({ ...snapshot, state: 'succeeded', cleanup: 'confirmed', tools: [{ name: 'original_tool' }] });
    await useNativeMcpCatalog.getState().preview(original);
    const view = render(<NativeMcpProbe selection={selection} serverName="configured-server" />);
    expect(screen.getByText('original_tool')).toBeTruthy();
    view.rerender(<NativeMcpProbe selection={selection} serverName="other-server" />);
    expect(screen.queryByText('original_tool')).toBeNull();
    expect(screen.queryByText('/trusted/server')).toBeNull();
    view.rerender(<NativeMcpProbe selection={selection} draft={{ ...draft, name: 'registry-deployment', transport: 'Http', url: 'https://new.example/mcp' }} />);
    expect(screen.queryByText('original_tool')).toBeNull();
    expect(screen.queryByRole('button', { name: 'nativeMcpCatalog.inspectProbe' })).toBeNull();
    expect(screen.getByRole('button', { name: 'nativeMcpCatalog.previewProbe' }).hasAttribute('disabled')).toBe(false);
  });
  it('keeps same-target edits explicitly stale without retaining hidden transport fields', async () => {
    const original = nativeDraftProbeInput(selection, draft)!;
    invoke.mockResolvedValueOnce({ ...snapshot, state: 'succeeded', cleanup: 'confirmed', tools: [{ name: 'original_tool' }] });
    await useNativeMcpCatalog.getState().preview(original);
    const changed = { ...draft, transport: 'Http' as const, url: 'https://new.example/mcp', cwd: '/old/cwd', bearer_env: 'HTTP_TOKEN' };
    const input = nativeDraftProbeInput(selection, changed)!;
    expect(input.source).toMatchObject({ kind: 'draft', draft: { program: '', args: [], forwarded_env: [], cwd: '', bearer_env: 'HTTP_TOKEN' } });
    render(<NativeMcpProbe selection={selection} draft={changed} />);
    expect(screen.getByText('nativeMcpCatalog.expiredResult')).toBeTruthy();
    expect(screen.getByText('original_tool')).toBeTruthy();
  });
  it('retains an unknown original probe instead of allocating a replacement after root drift', async () => {
    const input = nativeDraftProbeInput(selection, draft)!, key = nativeProbeKey(input);
    useNativeMcpCatalog.setState({ probes: { [key]: { snapshot: { ...snapshot, state: 'unknown', cleanup: 'unknown' }, input, error: null } } });
    useDevelopment.setState(state => ({ projects: state.projects.map(project => ({ ...project, directory_identity: 'root-b' })) }));
    expect(await useNativeMcpCatalog.getState().preview(input)).toEqual({ ok: false, code: 'native_extension_Busy' });
    expect(invoke).not.toHaveBeenCalled();
    invoke.mockResolvedValueOnce({ ...snapshot, state: 'failed', cleanup: 'confirmed' });
    await useNativeMcpCatalog.getState().operation(selection, 'inspect', key);
    expect(invoke).toHaveBeenCalledWith('inspect_native_mcp_probe', { input: { probe_id: 'original' } });
  });
});
