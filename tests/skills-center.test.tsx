import { setLocaleForTesting } from '../src/i18n/runtime';
import { StrictMode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { act, cleanup, fireEvent, render as renderView, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { ModalHost, ToastViewport, clearToasts } from '@dgos/ui';
import { getToastSnapshot } from '../src/ui/feedback/toastStore';
import SkillsManagement from '../src/pages/SkillsManagement';
import { useSkillsStore } from '../src/stores/skillsStore';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const mockedInvoke = vi.mocked(invoke);
const initial = useSkillsStore.getState();
const render = (ui: React.ReactNode) => renderView(<MemoryRouter  future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>{ui}</MemoryRouter>);
const official = { skill_id: 'dgos.official.writing.summarize', name: 'Summary', latest_version: '1.0.0', category: 'writing',
  description: 'Evidence-based summary', author: 'DGOS Team', tags: ['summary'], identity: 'dgos_official',
  source_kind: 'dgos_official', license_spdx: 'Apache-2.0', compatible_apps: ['ai-workbench'], install_path: 'bundled/dgos.official.writing.summarize/1.0.0' };
const legacy = { skill_id: 'old-calculator', name: 'Old Calculator', latest_version: '1', category: 'old', tags: [], identity: 'legacy_registry', source_kind: 'legacy_registry', compatible_apps: [] };
const manifest = { id: official.skill_id, name: official.name, version: '1.0.0', category: 'writing', tags: [], call_type: 'content', dependencies: {} };
const detail = { ...official, format: 'skill_md', entry: 'SKILL.md', manifest, content: {
  body: '# Summarize\nDo not invent facts.', content_hash: 'a'.repeat(64), license_text: 'Apache License 2.0 text',
  source_revision: 'content-release-1.0.0', required_tools: [], verification: { status: 'not_evaluated', scope: [] },
} };
let installed: unknown[] = [];
beforeEach(() => {
  setLocaleForTesting('en-US');
  installed = [];
  useSkillsStore.setState(initial, true);
  mockedInvoke.mockImplementation(async (command, args) => {
    if (command === 'fetch_skills_registry') return { skills: [official, legacy], registry_url: 'bundled://official', offline: false, warning: null };
    if (command === 'list_installed_skills') return installed;
    if (command === 'get_skill_detail') return detail;
    if (command === 'search_skills') return [];
    if (command === 'check_dependencies') return { satisfied: true, missing: [] };
    if (command === 'install_skill') {
      installed = [{ ...official, id: 'version-1', version: '1.0.0', installed_at: 1, manifest, call_type: 'content', enabled: true }];
      return installed[0];
    }
    if (command === 'toggle_skill') {
      installed = [{ ...official, id: 'version-1', version: '1.0.0', installed_at: 1, manifest, call_type: 'content', enabled: (args as {enabled: boolean}).enabled }];
    }
    return [];
  });
});
afterEach(() => { cleanup(); clearToasts(); vi.restoreAllMocks(); });

it('AC17 shows a local official library independently of legacy content and filters sources', async () => {
  render(<StrictMode><SkillsManagement /></StrictMode>);
  await screen.findByText('Official library is available locally');
  expect(screen.queryByText('Skills registry unavailable')).toBeNull();
  expect(screen.queryByRole('button', { name: 'Legacy: read-only' })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'DGOS Official', exact: true }));
  expect(screen.queryByText('Old Calculator')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Legacy archive' }));
  expect(screen.queryByText('Summary')).toBeNull();
  expect(screen.getByText('Old Calculator')).toBeTruthy();
  expect(mockedInvoke.mock.calls.filter(([name]) => name === 'fetch_skills_registry')).toHaveLength(1);
});

it('AC17 keeps category filtering and selection when the interface language changes', async () => {
  setLocaleForTesting('zh-CN');
  render(<SkillsManagement />);
  const filters = await screen.findByRole('group', { name: '分类' });
  const writing = await within(filters).findByRole('button', { name: '写作与内容' });
  fireEvent.click(writing);
  expect(screen.getByText('Summary')).toBeTruthy();
  expect(screen.queryByText('Old Calculator')).toBeNull();
  expect(writing.getAttribute('aria-pressed')).toBe('true');
  await act(async () => { setLocaleForTesting('en-US'); });
  const translated = within(screen.getByRole('group', { name: 'Category' })).getByRole('button', { name: 'Writing & content' });
  expect(translated.getAttribute('aria-pressed')).toBe('true');
  expect(screen.queryByText('Old Calculator')).toBeNull();
  fireEvent.click(within(screen.getByRole('group', { name: 'Category' })).getByRole('button', { name: 'old', exact: true }));
  expect(screen.getByText('Old Calculator')).toBeTruthy();
  expect(screen.queryByText('Summary')).toBeNull();
});

it('AC18 previews the SKILL.md, provenance, license and unverified quality without executing text', async () => {
  render(<SkillsManagement />);
  await screen.findByText('Summary');
  const buttons = screen.getAllByRole('button', { name: 'Details' });
  buttons[0].focus();
  fireEvent.click(buttons[0]);
  const detail = await screen.findByRole('region', { name: 'Details' });
  await within(detail).findByText(/Do not invent facts/);
  expect(within(detail).getByText('SKILL.md package')).toBeTruthy();
  expect(within(detail).getByText(/not_evaluated/)).toBeTruthy();
  fireEvent.click(within(detail).getByText('License text'));
  expect(within(detail).getByText('Apache License 2.0 text')).toBeTruthy();
  expect(mockedInvoke.mock.calls.some(([name]) => name === 'call_skill')).toBe(false);
});

it('AC20 adds an official skill and disables it without uninstalling bundled files', async () => {
  render(<SkillsManagement />);
  const add = await screen.findByRole('button', { name: 'Add to My Skills' });
  fireEvent.click(add);
  await screen.findByText(/Skill added to My Skills/);
  fireEvent.mouseDown(screen.getByRole('tab', { name: 'Installed (1)' }), { button: 0 });
  const toggle = screen.getByRole('switch', { name: 'Disable Summary' });
  expect(toggle.getAttribute('aria-checked')).toBe('true');
  fireEvent.click(toggle);
  await waitFor(() => expect(mockedInvoke).toHaveBeenCalledWith('toggle_skill', { skillId: official.skill_id, enabled: false }));
  await waitFor(() => expect(screen.getByRole('switch', { name: 'Enable Summary' }).getAttribute('aria-checked')).toBe('false'));
  expect(screen.queryByRole('button', { name: 'Uninstall' })).toBeNull();
});

it('AC18 reports a broken content preview and does not show a stale success dialog', async () => {
  const handler = mockedInvoke.getMockImplementation()!;
  mockedInvoke.mockImplementation((command, args) => command === 'get_skill_detail' ? Promise.reject('package hash mismatch') : handler(command, args));
  vi.spyOn(console, 'error').mockImplementation(() => {});
  render(<SkillsManagement />);
  await screen.findByText('Summary');
  fireEvent.click(screen.getAllByRole('button', { name: 'Details' })[0]);
  await screen.findByText('Unable to load skill details: package hash mismatch');
  expect(screen.queryByRole('dialog')).toBeNull();
});

// D111: a background load that fails shows an inline block and never a Toast or the unified error path.
it('D111 shows registry, installed-list and update-check load failures inline and raises no Toast', async () => {
  const logged = vi.spyOn(console, 'error').mockImplementation(() => {});
  mockedInvoke.mockImplementation(async (command) => {
    if (command === 'fetch_skills_registry') throw 'registry host unreachable';
    if (command === 'list_installed_skills') throw 'installed list unreadable';
    if (command === 'check_skill_updates') throw 'update check unreachable';
    return [];
  });
  render(<><SkillsManagement /><ToastViewport /></>);
  const installedAlert = await screen.findByText('Unable to load installed skills: installed list unreadable');
  expect(installedAlert.closest('[role="alert"]')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Retry loading skills' })).toBeTruthy();
  expect(screen.getAllByRole('status').some(node => node.textContent === 'Skills registry unavailable')).toBe(true);
  const state = useSkillsStore.getState();
  expect(state.registryError).toBe('registry host unreachable');
  expect(state.installedError).toBe('installed list unreadable');
  // The update check only runs after a successful registry fetch, so it is exercised on its own as well.
  await act(async () => { await expect(useSkillsStore.getState().checkUpdates()).rejects.toBe('update check unreachable'); });
  expect(await screen.findByText('Unable to check skill updates: update check unreachable')).toBeTruthy();
  expect(getToastSnapshot().visible).toHaveLength(0);
  expect(getToastSnapshot().queued).toHaveLength(0);
  expect(screen.queryAllByRole('alert').every(node => !node.hasAttribute('data-state'))).toBe(true);
  expect(logged).not.toHaveBeenCalledWith('[dgos] error', expect.anything());
});

it('does not confirm an empty registry when its first read fails', async () => {
  const handler = mockedInvoke.getMockImplementation()!;
  mockedInvoke.mockImplementation((command, args) => command === 'fetch_skills_registry' ? Promise.reject('registry unavailable') : handler(command, args));
  render(<SkillsManagement />);
  const browse = screen.getByRole('region', { name: 'Browse Skills' });
  await within(browse).findByText('registry unavailable');
  expect(within(browse).queryByText('No skills found.')).toBeNull();
  expect(within(browse).queryByText('No skills can be browsed until the registry service is available.')).toBeNull();
  expect(screen.getByRole('button', { name: 'Retry connection' })).toBeTruthy();
});

it('keeps the selected detail mounted while the same detail read fails and retries', async () => {
  vi.spyOn(console, 'error').mockImplementation(() => {});
  render(<SkillsManagement />);
  await screen.findByText('Summary');
  fireEvent.click(screen.getAllByRole('button', { name: 'Details' })[0]);
  const preview = await screen.findByText(/Do not invent facts/);
  const handler = mockedInvoke.getMockImplementation()!;
  const failedRead = Promise.withResolvers<unknown>();
  mockedInvoke.mockImplementation((command, args) => command === 'get_skill_detail'
    ? failedRead.promise : handler(command, args));
  fireEvent.click(screen.getAllByRole('button', { name: 'Details' })[0]);
  expect(screen.getByText(/Do not invent facts/)).toBe(preview);
  expect(screen.getByText(/Refreshing/)).toBeTruthy();
  await act(async () => { failedRead.reject('detail unavailable'); });
  const panel = screen.getByRole('region', { name: 'Details' });
  expect(within(panel).getByText('Unable to load skill details: detail unavailable')).toBeTruthy();
  expect(screen.getByText(/Do not invent facts/)).toBe(preview);
  const retryRead = Promise.withResolvers<unknown>();
  mockedInvoke.mockImplementation((command, args) => command === 'get_skill_detail'
    ? retryRead.promise : handler(command, args));
  fireEvent.click(within(panel).getByRole('button', { name: 'Retry' }));
  expect(within(panel).getByText('Unable to load skill details: detail unavailable')).toBeTruthy();
  expect(screen.getByText(/Do not invent facts/)).toBe(preview);
  await act(async () => { retryRead.resolve(detail); });
  expect(within(panel).queryByText('Unable to load skill details: detail unavailable')).toBeNull();
  expect(screen.getByText(/Do not invent facts/)).toBe(preview);
});

it('keeps installed resources through refresh failures and clears the failure only on recovery', async () => {
  installed = [{ ...official, id: 'installed-1', version: '1.0.0', installed_at: 1, manifest, call_type: 'content', enabled: true }];
  render(<SkillsManagement />);
  const tab = await screen.findByRole('tab', { name: 'Installed (1)' });
  fireEvent.mouseDown(tab, { button: 0 });
  const resource = screen.getByText('Summary');
  const handler = mockedInvoke.getMockImplementation()!;
  const failedRead = Promise.withResolvers<unknown>();
  mockedInvoke.mockImplementation((command, args) => command === 'list_installed_skills'
    ? failedRead.promise : handler(command, args));
  let pending!: Promise<void>;
  act(() => { pending = useSkillsStore.getState().listInstalledSkills().catch(() => {}); });
  expect(screen.getByText('Summary')).toBe(resource);
  expect(screen.getByText(/Refreshing/)).toBeTruthy();
  await act(async () => { failedRead.reject('installed unavailable'); await pending; });
  expect(screen.getByText('Summary')).toBe(resource);
  expect(screen.getByRole('alert').textContent).toContain('installed unavailable');
  const retryRead = Promise.withResolvers<unknown>();
  mockedInvoke.mockImplementation((command, args) => command === 'list_installed_skills'
    ? retryRead.promise : handler(command, args));
  fireEvent.click(screen.getByRole('button', { name: 'Retry loading skills' }));
  expect(screen.getByRole('alert').textContent).toContain('installed unavailable');
  expect(screen.getByText('Summary')).toBe(resource);
  await act(async () => { retryRead.resolve(installed); });
  await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
  expect(screen.getByText('Summary')).toBe(resource);
});

it('keeps a confirmed empty registry visible through a failed refresh and retry', async () => {
  const handler = mockedInvoke.getMockImplementation()!;
  mockedInvoke.mockImplementation((command, args) => command === 'fetch_skills_registry'
    ? Promise.resolve({ skills: [], registry_url: 'bundled://official', offline: false, warning: null }) : handler(command, args));
  render(<SkillsManagement />);
  const empty = await screen.findByText('No skills found.');
  const browse = screen.getByRole('region', { name: 'Browse Skills' });
  const failedRead = Promise.withResolvers<unknown>();
  mockedInvoke.mockImplementation((command, args) => command === 'fetch_skills_registry' ? failedRead.promise : handler(command, args));
  let pending!: Promise<void>;
  act(() => { pending = useSkillsStore.getState().fetchRegistry(); });
  await act(async () => { failedRead.reject('registry unavailable'); await pending; });
  expect(screen.getByText('No skills found.')).toBe(empty);
  expect(within(browse).getByRole('alert').textContent).toContain('registry unavailable');
  const retryRead = Promise.withResolvers<unknown>();
  mockedInvoke.mockImplementation((command, args) => command === 'fetch_skills_registry' ? retryRead.promise : handler(command, args));
  fireEvent.click(screen.getByRole('button', { name: 'Retry connection' }));
  expect(screen.getByText('No skills found.')).toBe(empty);
  expect(within(browse).getByRole('alert').textContent).toContain('registry unavailable');
  expect(within(browse).getByText(/Refreshing/)).toBeTruthy();
  await act(async () => { retryRead.resolve({ skills: [], registry_url: 'bundled://official', offline: false, warning: null }); });
  await waitFor(() => expect(within(browse).queryByRole('alert')).toBeNull());
  expect(screen.getByText('No skills found.')).toBe(empty);
});

it('hides the previous skill detail when a new target fails, then retries that target', async () => {
  vi.spyOn(console, 'error').mockImplementation(() => {});
  render(<SkillsManagement />);
  await screen.findByText('Summary');
  fireEvent.click(screen.getAllByRole('button', { name: 'Details' })[0]);
  await screen.findByText(/Do not invent facts/);
  const panel = screen.getByRole('region', { name: 'Details' });
  const handler = mockedInvoke.getMockImplementation()!;
  const failedRead = Promise.withResolvers<unknown>();
  mockedInvoke.mockImplementation((command, args) => command === 'get_skill_detail' ? failedRead.promise : handler(command, args));
  fireEvent.click(screen.getAllByRole('button', { name: 'Details' })[1]);
  expect(within(panel).queryByText(/Do not invent facts/)).toBeNull();
  expect(within(panel).getByRole('status').textContent).toContain('Loading');
  await act(async () => { failedRead.reject('other skill unavailable'); });
  expect(within(panel).getByRole('alert').textContent).toContain('other skill unavailable');
  expect(within(panel).queryByText(/Do not invent facts/)).toBeNull();
  expect(within(panel).queryByText('Select a skill')).toBeNull();
  const retryRead = Promise.withResolvers<unknown>();
  mockedInvoke.mockImplementation((command, args) => command === 'get_skill_detail' ? retryRead.promise : handler(command, args));
  fireEvent.click(within(panel).getByRole('button', { name: 'Retry' }));
  expect(mockedInvoke).toHaveBeenLastCalledWith('get_skill_detail', { skillId: legacy.skill_id });
  expect(within(panel).queryByText(/Do not invent facts/)).toBeNull();
  await act(async () => { retryRead.resolve({ ...detail, ...legacy }); });
  expect(within(panel).getByRole('heading', { name: 'Old Calculator' })).toBeTruthy();
});

it.each(['success', 'failure'] as const)('ignores a superseded detail %s after the latest target succeeds', async outcome => {
  vi.spyOn(console, 'error').mockImplementation(() => {});
  const handler = mockedInvoke.getMockImplementation()!;
  const firstRead = Promise.withResolvers<unknown>();
  const latestRead = Promise.withResolvers<unknown>();
  mockedInvoke.mockImplementation((command, args) => {
    if (command !== 'get_skill_detail') return handler(command, args);
    return args && typeof args === 'object' && 'skillId' in args && args.skillId === official.skill_id
      ? firstRead.promise : latestRead.promise;
  });
  render(<SkillsManagement />);
  await screen.findByText('Summary');
  const buttons = screen.getAllByRole('button', { name: 'Details' });
  fireEvent.click(buttons[0]);
  fireEvent.click(buttons[1]);
  const panel = screen.getByRole('region', { name: 'Details' });
  const latestDetail = { ...detail, ...legacy, content: { ...detail.content, body: 'Latest target content' } };
  await act(async () => { latestRead.resolve(latestDetail); });
  const content = within(panel).getByText('Latest target content');
  await act(async () => {
    if (outcome === 'success') firstRead.resolve(detail);
    else firstRead.reject('superseded failure');
  });
  expect(within(panel).getByText('Latest target content')).toBe(content);
  expect(within(panel).getByRole('heading', { name: 'Old Calculator' })).toBeTruthy();
  expect(within(panel).queryByText(/Do not invent facts/)).toBeNull();
  expect(within(panel).queryByRole('alert')).toBeNull();
  expect(useSkillsStore.getState().selectedSkill?.skill_id).toBe(legacy.skill_id);
});
