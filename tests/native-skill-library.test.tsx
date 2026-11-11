import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { NativeSkillInspector, NativeSkillLibrary } from '../src/components/nativeExtensions/NativeSkillLibrary';
import { nativeLibraryKey, useNativeSkillLibrary } from '../src/stores/nativeSkillLibrary';
import { useDevelopment } from '../src/stores/developmentStore';
import type { NativeSelection, NativeSkill } from '../src/types/nativeExtensions';
import type { NativeLibrarySkill, NativeSkillSource } from '../src/types/nativeSkillLibrary';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }));
vi.mock('../src/i18n', () => ({ t: (key: string) => key, useLocale: () => ({ locale: 'en-US', t: (key: string) => key }) }));
const selection: NativeSelection = { target: 'Codex', scope: 'ProjectShared', project_id: 'project-a', task_id: 'task-worktree' };
const declarations = { declared: true, tools: [{ program: 'fixture-tool', version_requirement: '>=1' }], packages: ['unverified-package'], required_tools: [] };
const librarySkill: NativeLibrarySkill = { path: 'skills/first', name: 'first', description: 'First fixture skill', content_hash: 'a'.repeat(64), file_count: 2, script_count: 1, executable_files: 1, dynamic_commands: false, dependencies: declarations };
const repository = { owner: 'example', repo: 'skills', requested_ref: 'main', skill_subpath: null, normalized_source: 'https://github.com/example/skills/tree/main' };
const source: NativeSkillSource = { id: 'source-id', revision: 1, repository, indexed_at: 100, commit: 'b'.repeat(40), error: null, skills: [librarySkill], rejected: [] };
const skill: NativeSkill = { instance: { id: 'installed-instance', revision: 'installed-revision' }, target: 'Codex', scope: 'ProjectShared', slug: 'first', owner: 'Loose', metadata: { name: 'first', description: 'First fixture skill', license: null, dependencies: declarations, dynamic_commands: false }, resources: [{ path: 'SKILL.md', hash: 'c'.repeat(64), bytes: 80, executable: false }], source: { source_hash: 'a'.repeat(64), commit: 'b'.repeat(40), origin: { repository, source_id: source.id, skill_path: librarySkill.path } }, source_modified: false, can_update: true };
const plan = { plan_id: 'plan-id', executable: '/trusted/bin/fixture-tool', digest: 'd'.repeat(64), argv: ['--version'], cwd: '/trusted/worktree', expires_at: 9999999999, deadline_seconds: 5, environment: 'native_host_cli_resolution; no credential forwarding', requirement: '>=1' };
beforeEach(() => {
  invoke.mockReset();
  useDevelopment.setState(state => ({ epoch: state.epoch + 1 }));
  useNativeSkillLibrary.getState().reset();
  invoke.mockImplementation(async (command: string) => {
    if (command === 'list_native_skill_sources') return [source];
    if (command === 'inspect_native_skill') return { skill, target_path: '/trusted/worktree/.agents/skills/first' };
    if (command === 'inspect_native_skill_dependencies') return { declarations, tools: [{ declaration: declarations.tools[0], availability: 'FoundUnprobed', candidate: plan.executable, executable: true }], unknown: true };
    if (command === 'preview_native_skill_version') return plan;
    throw 'native_extension_Unavailable';
  });
});
afterEach(cleanup);

it('reads only the saved source cache on mount and saves a source without indexing or installing', async () => {
  invoke.mockImplementation(async (command: string) => command === 'save_native_skill_source' ? source : []);
  const onPreview = vi.fn();
  render(<NativeSkillLibrary selection={selection} tab="sources" disabled={false} onPreview={onPreview} />);
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('list_native_skill_sources', { input: selection }));
  expect(invoke.mock.calls.every(([command]) => command === 'list_native_skill_sources')).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.addSource' }));
  fireEvent.change(screen.getByLabelText('nativeSkillLibrary.repositoryUrl'), { target: { value: repository.normalized_source } });
  fireEvent.submit(screen.getByLabelText('nativeSkillLibrary.repositoryUrl').closest('form')!);
  await screen.findByText('nativeSkillLibrary.sourceSaved');
  expect(invoke).toHaveBeenCalledWith('save_native_skill_source', { input: { selection, id: null, expected_revision: null, input: repository.normalized_source } });
  expect(invoke.mock.calls.some(([command]) => ['refresh_native_skill_source', 'preview_native_skill', 'commit_native_operation', 'run_native_skill_version'].includes(command))).toBe(false);
  expect(onPreview).not.toHaveBeenCalled();
});

it('searches explicitly and sends the selected repository subdirectory through the existing preview callback', async () => {
  invoke.mockImplementation(async (command: string) => command === 'search_native_skill_library' ? { sources: [source], results: [librarySkill, { ...librarySkill, name: 'second', path: 'skills/second' }].map(item => ({ source_id: source.id, repository_url: repository.normalized_source, indexed_at: 100, commit: source.commit, stale: true, skill: item })) } : [source]);
  const onPreview = vi.fn();
  render(<NativeSkillLibrary selection={selection} tab="discover" disabled={false} onPreview={onPreview} />);
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('list_native_skill_sources', { input: selection }));
  expect(invoke.mock.calls.some(([command]) => command === 'search_native_skill_library')).toBe(false);
  fireEvent.change(screen.getByLabelText('nativeSkillLibrary.searchQuery'), { target: { value: 'fixture' } });
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.search' }));
  fireEvent.click(await screen.findByRole('button', { name: /^second/ }));
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.previewInstall' }));
  expect(onPreview).toHaveBeenCalledWith({ ...selection, action: 'install', slug: 'second', expected: null, source: { kind: 'repository', source_id: source.id, skill_path: 'skills/second' } });
  expect(invoke.mock.calls.some(([command]) => ['preview_native_skill', 'commit_native_operation', 'run_native_skill_version'].includes(command))).toBe(false);
  expect(screen.getAllByText('nativeSkillLibrary.lastResults').length).toBeGreaterThan(0);
});

it('imports multiple picker packages using only a host grant, never a renderer file path or ZIP bytes', async () => {
  invoke.mockImplementation(async (command: string) => command === 'pick_native_skill_source' ? { handle: 'opaque-host-grant', label: 'picked.zip', expires_at: 9999999999, skills: [librarySkill, { ...librarySkill, name: 'second', path: 'skills/second' }], rejected: [] } : []);
  const onPreview = vi.fn();
  render(<NativeSkillLibrary selection={selection} tab="discover" disabled={false} onPreview={onPreview} />);
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.method.local' }));
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.pickZip' }));
  fireEvent.click(await screen.findByRole('button', { name: /^second/ }));
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.previewInstall' }));
  expect(invoke).toHaveBeenCalledWith('pick_native_skill_source', { input: { selection, kind: 'zip' } });
  expect(onPreview).toHaveBeenCalledWith({ ...selection, action: 'install', slug: 'second', expected: null, source: { kind: 'picker', handle: 'opaque-host-grant', skill_path: 'skills/second' } });
  expect(JSON.stringify(onPreview.mock.calls)).not.toMatch(/"bytes"|"path":/);
});

it('binds content reads to the installed revision and never renders file HTML as active content', async () => {
  invoke.mockImplementation(async (command: string) => command === 'read_native_skill_file' ? { instance: skill.instance, path: 'SKILL.md', hash: 'c'.repeat(64), bytes: 80, text: '<img src=x onerror="forbidden()">', reason: null } : { skill, target_path: '/trusted/worktree/.agents/skills/first' });
  render(<NativeSkillInspector selection={selection} skill={skill} disabled={false} onPreview={vi.fn()} />);
  await screen.findByText('/trusted/worktree/.agents/skills/first');
  fireEvent.click(screen.getByRole('button', { name: 'SKILL.md' }));
  await screen.findByText('<img src=x onerror="forbidden()">');
  expect(document.querySelector('img')).toBeNull();
  expect(invoke).toHaveBeenCalledWith('read_native_skill_file', { input: { skill: { selection, slug: skill.slug, expected: skill.instance }, path: 'SKILL.md' } });
});

it('does not execute static dependency checks or version previews, then requires exact explicit approval', async () => {
  invoke.mockImplementation(async (command: string) => {
    if (command === 'inspect_native_skill') return { skill, target_path: '/trusted/worktree/.agents/skills/first' };
    if (command === 'inspect_native_skill_dependencies') return { declarations, tools: [{ declaration: declarations.tools[0], availability: 'FoundUnprobed', candidate: plan.executable, executable: true }], unknown: true };
    if (command === 'preview_native_skill_version') return plan;
    if (command === 'run_native_skill_version') return { status: 'observed', output: 'fixture 1.0.0', exit_code: 0, cleanup_confirmed: true, requirement_satisfied: null };
    throw 'native_extension_Unavailable';
  });
  render(<NativeSkillInspector selection={selection} skill={skill} disabled={false} onPreview={vi.fn()} />);
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.checkDependencies' }));
  await screen.findByText('nativeSkillLibrary.availability.FoundUnprobed', { exact: false });
  expect(invoke.mock.calls.some(([command]) => command === 'run_native_skill_version')).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.previewVersion' }));
  await screen.findByText('["--version"]');
  expect(invoke.mock.calls.some(([command]) => command === 'run_native_skill_version')).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.approveVersion' }));
  await screen.findByText('fixture 1.0.0');
  expect(invoke).toHaveBeenCalledWith('run_native_skill_version', { input: { selection, plan_id: plan.plan_id, approved: true } });
  expect(screen.getAllByText('nativeSkillLibrary.requirementsUnknown').length).toBeGreaterThan(0);
});

it('verifies the same unknown version request without executing it again', async () => {
  invoke.mockImplementation(async (command: string) => {
    if (command === 'inspect_native_skill') return { skill, target_path: '/trusted/worktree/.agents/skills/first' };
    if (command === 'inspect_native_skill_dependencies') return { declarations, tools: [{ declaration: declarations.tools[0], availability: 'FoundUnprobed', candidate: plan.executable, executable: true }], unknown: true };
    if (command === 'preview_native_skill_version') return plan;
    if (command === 'run_native_skill_version') throw 'private-transport-detail';
    if (command === 'inspect_native_skill_version') return { status: 'observed', output: 'fixture 1.0.0', exit_code: 0, cleanup_confirmed: true, requirement_satisfied: null };
    throw 'native_extension_Unavailable';
  });
  render(<NativeSkillInspector selection={selection} skill={skill} disabled={false} onPreview={vi.fn()} />);
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.checkDependencies' }));
  fireEvent.click(await screen.findByRole('button', { name: 'nativeSkillLibrary.previewVersion' }));
  fireEvent.click(await screen.findByRole('button', { name: 'nativeSkillLibrary.approveVersion' }));
  await screen.findByText('nativeSkillLibrary.versionStatus.unknown');
  await waitFor(() => expect(screen.getByRole('button', { name: 'nativeSkillLibrary.inspectVersion' }).hasAttribute('disabled')).toBe(false));
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.inspectVersion' }));
  await screen.findByText('fixture 1.0.0');
  expect(invoke.mock.calls.filter(([command]) => command === 'run_native_skill_version')).toHaveLength(1);
  expect(invoke).toHaveBeenCalledWith('inspect_native_skill_version', { input: { selection, plan_id: plan.plan_id } });
  expect(document.body.textContent).not.toContain('private-transport-detail');
});

it('drops late account results and isolates source drafts by task worktree', async () => {
  const pending = Promise.withResolvers<unknown>();
  invoke.mockImplementation((command: string) => command === 'search_native_skill_library' ? pending.promise : Promise.resolve([source]));
  const left = { input: 'https://github.com/example/one', sourceId: null, revision: null };
  useNativeSkillLibrary.getState().setDraft(selection, left);
  const other = { ...selection, task_id: 'different-worktree' };
  useNativeSkillLibrary.getState().setDraft(other, { ...left, input: 'https://github.com/example/two' });
  expect(useNativeSkillLibrary.getState().drafts[nativeLibraryKey(selection)]).toBe(left);
  const request = useNativeSkillLibrary.getState().search(selection, '', null);
  await act(async () => { useDevelopment.setState(state => ({ epoch: state.epoch + 1 })); pending.resolve({ sources: [source], results: [{ source_id: source.id, skill: librarySkill }] }); });
  expect(await request).toEqual({ ok: false, code: 'native_extension_StaleGeneration' });
  expect(useNativeSkillLibrary.getState().drafts).toEqual({});
});

it('releases the previous picker grant on repick and the current grant on unmount', async () => {
  let pickCount = 0;
  invoke.mockImplementation(async (command: string) => command === 'pick_native_skill_source' ? { handle: `host-grant-${++pickCount}`, label: 'picked.zip', expires_at: 9999999999, skills: [librarySkill], rejected: [] } : []);
  const view = render(<NativeSkillLibrary selection={selection} tab="discover" disabled={false} onPreview={vi.fn()} />);
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.method.local' }));
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.pickZip' }));
  await screen.findByRole('button', { name: /^first/ });
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.pickDirectory' }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('release_native_skill_source', { input: { selection, handle: 'host-grant-1' } }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('pick_native_skill_source', { input: { selection, kind: 'directory' } }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'nativeSkillLibrary.pickZip' }).hasAttribute('disabled')).toBe(false));
  view.unmount();
  expect(invoke).toHaveBeenCalledWith('release_native_skill_source', { input: { selection, handle: 'host-grant-2' } });
});

it('prepares a source-associated update then delegates its exact installed revision to the existing preview flow', async () => {
  invoke.mockImplementation(async (command: string) => command === 'prepare_native_skill_update' ? { handle: 'source-update-grant', skill_path: librarySkill.path } : { skill, target_path: '/trusted/worktree/.agents/skills/first' });
  const onPreview = vi.fn();
  render(<NativeSkillInspector selection={selection} skill={skill} disabled={false} onPreview={onPreview} />);
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.previewUpdate' }));
  await waitFor(() => expect(onPreview).toHaveBeenCalledWith({ ...selection, action: 'update', slug: skill.slug, expected: skill.instance, source: { kind: 'picker', handle: 'source-update-grant', skill_path: librarySkill.path } }));
  expect(invoke).toHaveBeenCalledWith('prepare_native_skill_update', { input: { selection, slug: skill.slug, expected: skill.instance } });
  expect(invoke.mock.calls.some(([command]) => ['preview_native_skill', 'commit_native_operation'].includes(command))).toBe(false);
});

it('re-authorizes local replacements with a host picker rather than accepting the old path as lasting authority', async () => {
  const localSkill = { ...skill, source: { source_hash: skill.source!.source_hash, commit: null, origin: null } };
  invoke.mockImplementation(async (command: string) => command === 'pick_native_skill_source' ? { handle: 'local-replacement-grant', label: 'selected-directory', expires_at: 9999999999, skills: [librarySkill], rejected: [] } : { skill: localSkill, target_path: '/trusted/worktree/.agents/skills/first' });
  const onPreview = vi.fn();
  render(<NativeSkillInspector selection={selection} skill={localSkill} disabled={false} onPreview={onPreview} />);
  expect(screen.queryByRole('button', { name: 'nativeSkillLibrary.previewUpdate' })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.pickDirectory' }));
  fireEvent.click(await screen.findByRole('button', { name: `nativeSkillLibrary.previewReplacement: ${librarySkill.name} / ${librarySkill.path}` }));
  expect(onPreview).toHaveBeenCalledWith({ ...selection, action: 'update', slug: skill.slug, expected: skill.instance, source: { kind: 'picker', handle: 'local-replacement-grant', skill_path: librarySkill.path } });
  expect(invoke).toHaveBeenCalledWith('pick_native_skill_source', { input: { selection, kind: 'directory' } });
});

it('stops on pre-existing local modifications until explicitly re-reading the exact installed revision', async () => {
  const modified = { ...skill, source_modified: true };
  const reviewed = { ...modified, instance: { ...skill.instance, revision: 'reviewed-local-revision' } };
  let inspections = 0;
  invoke.mockImplementation(async (command: string) => {
    if (command === 'get_native_skill') return reviewed;
    if (command === 'inspect_native_skill') return { skill: ++inspections === 1 ? modified : reviewed, target_path: '/trusted/worktree/.agents/skills/first' };
    if (command === 'prepare_native_skill_update') return { handle: 'reviewed-update', skill_path: librarySkill.path };
    throw 'native_extension_Unavailable';
  });
  const onPreview = vi.fn();
  render(<NativeSkillInspector selection={selection} skill={modified} disabled={false} onPreview={onPreview} />);
  expect(screen.getByRole('button', { name: 'nativeSkillLibrary.previewUpdate' }).hasAttribute('disabled')).toBe(true);
  expect(onPreview).not.toHaveBeenCalled();
  await screen.findByText('/trusted/worktree/.agents/skills/first');
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.reread' }));
  await screen.findByText('nativeSkillLibrary.rereadInstruction');
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.previewUpdate' }));
  await waitFor(() => expect(onPreview).toHaveBeenCalledWith({ ...selection, action: 'update', slug: reviewed.slug, expected: reviewed.instance, source: { kind: 'picker', handle: 'reviewed-update', skill_path: librarySkill.path } }));
  expect(invoke).toHaveBeenCalledWith('prepare_native_skill_update', { input: { selection, slug: reviewed.slug, expected: reviewed.instance } });
});

it('does not let a pre-mutation source list resurrect a removed source or overwrite a saved revision', async () => {
  const pending = Promise.withResolvers<NativeSkillSource[]>();
  const saved = { ...source, revision: 2 };
  invoke.mockImplementation((command: string) => command === 'list_native_skill_sources' ? pending.promise : Promise.resolve(command === 'save_native_skill_source' ? saved : null));
  const api = useNativeSkillLibrary.getState();
  const read = api.loadSources(selection);
  await api.saveSource(selection, { input: repository.normalized_source, sourceId: source.id, revision: source.revision });
  await api.removeSource(selection, saved);
  pending.resolve([source]); await read;
  expect(useNativeSkillLibrary.getState().sources[nativeLibraryKey(selection)].value).toEqual([]);
});

it('releases a picker grant that arrives after the importing view was unmounted', async () => {
  const pending = Promise.withResolvers<unknown>();
  invoke.mockImplementation((command: string) => command === 'pick_native_skill_source' ? pending.promise : Promise.resolve([]));
  const view = render(<NativeSkillLibrary selection={selection} tab="discover" disabled={false} onPreview={vi.fn()} />);
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.method.local' }));
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.pickZip' }));
  view.unmount();
  await act(async () => pending.resolve({ handle: 'late-grant', label: 'picked.zip', expires_at: 9999999999, skills: [librarySkill], rejected: [] }));
  expect(invoke).toHaveBeenCalledWith('release_native_skill_source', { input: { selection, handle: 'late-grant' } });
});

it('retains an unknown version plan across revision remounts and only inspects its original identity', async () => {
  const request = { selection, slug: skill.slug, expected: skill.instance };
  invoke.mockRejectedValueOnce('native_extension_Unavailable');
  await useNativeSkillLibrary.getState().runVersion(request, plan);
  expect(useNativeSkillLibrary.getState().versions[plan.plan_id].result.cleanup_confirmed).toBe(false);
  const changed = { ...skill, instance: { ...skill.instance, revision: 'new-revision' } };
  invoke.mockImplementation(async (command: string) => command === 'inspect_native_skill_version' ? { status: 'observed', output: 'original result', exit_code: 0, cleanup_confirmed: true, requirement_satisfied: null } : { skill: changed, target_path: '/trusted/worktree/.agents/skills/first' });
  render(<NativeSkillInspector selection={selection} skill={changed} disabled={false} onPreview={vi.fn()} />);
  expect(screen.getByText('nativeSkillLibrary.versionStatus.unknown')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'nativeSkillLibrary.checkDependencies' }).hasAttribute('disabled')).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'nativeSkillLibrary.inspectVersion' }));
  await screen.findByText('original result');
  expect(invoke.mock.calls.filter(([command]) => command === 'run_native_skill_version')).toHaveLength(1);
  expect(invoke).toHaveBeenCalledWith('inspect_native_skill_version', { input: { selection, plan_id: plan.plan_id } });
});

it('does not publish a late version result after account revocation', async () => {
  const pending = Promise.withResolvers<unknown>();
  invoke.mockImplementationOnce(() => pending.promise);
  const running = useNativeSkillLibrary.getState().runVersion({ selection, slug: skill.slug, expected: skill.instance }, plan);
  useDevelopment.setState(state => ({ epoch: state.epoch + 1 }));
  pending.resolve({ status: 'observed', output: 'old account output', exit_code: 0, cleanup_confirmed: true, requirement_satisfied: null });
  expect(await running).toEqual({ ok: false, code: 'native_extension_StaleGeneration' });
  expect(useNativeSkillLibrary.getState().versions).toEqual({});
});
