import { StrictMode } from 'react';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import SkillsManagement from '../src/pages/SkillsManagement';
import { useSkillsStore } from '../src/stores/skillsStore';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const mockedInvoke = vi.mocked(invoke);
const initial = useSkillsStore.getState();
const official = { skill_id: 'dgos.official.writing.summarize', name: 'Summary', latest_version: '1.0.0', category: 'writing',
  description: 'Evidence-based summary', author: 'DGOS Team', tags: ['summary'], identity: 'dgos_official',
  source_kind: 'dgos_official', license_spdx: 'Apache-2.0', compatible_apps: ['ai-workbench'] };
const legacy = { skill_id: 'old-calculator', name: 'Old Calculator', latest_version: '1', category: 'old', tags: [], identity: 'legacy_registry', source_kind: 'legacy_registry', compatible_apps: [] };
const manifest = { id: official.skill_id, name: official.name, version: '1.0.0', category: 'writing', tags: [], call_type: 'content', dependencies: {} };
const detail = { ...official, format: 'skill_md', entry: 'SKILL.md', manifest, content: {
  body: '# Summarize\nDo not invent facts.', content_hash: 'a'.repeat(64), license_text: 'Apache License 2.0 text',
  source_revision: 'content-release-1.0.0', required_tools: [], verification: { status: 'not_evaluated', scope: [] },
} };
let installed: unknown[] = [];
beforeEach(() => {
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
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

it('AC17 shows a local official library independently of legacy content and filters sources', async () => {
  render(<StrictMode><SkillsManagement /></StrictMode>);
  await screen.findByText('Official library is available locally');
  expect(screen.queryByText('Skills registry unavailable')).toBeNull();
  expect(screen.getByText('Bundled content · Apache-2.0')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Legacy: read-only' }).hasAttribute('disabled')).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'DGOS Official', exact: true }));
  expect(screen.queryByText('Old Calculator')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Legacy archive' }));
  expect(screen.queryByText('Summary')).toBeNull();
  expect(screen.getByText('Old Calculator')).toBeTruthy();
  expect(mockedInvoke.mock.calls.filter(([name]) => name === 'fetch_skills_registry')).toHaveLength(1);
});

it('AC18 previews the SKILL.md, provenance, license and unverified quality without executing text', async () => {
  render(<SkillsManagement />);
  await screen.findByText('Summary');
  const buttons = screen.getAllByRole('button', { name: 'Details' });
  buttons[0].focus();
  fireEvent.click(buttons[0]);
  const dialog = await screen.findByRole('dialog');
  expect(within(dialog).getByText(/Do not invent facts/)).toBeTruthy();
  expect(within(dialog).getByText('SKILL.md package')).toBeTruthy();
  expect(within(dialog).queryByText('Call Type:')).toBeNull();
  expect(within(dialog).getByText(/not_evaluated/)).toBeTruthy();
  expect(within(dialog).getByText('Apache License 2.0 text')).toBeTruthy();
  expect(mockedInvoke.mock.calls.some(([name]) => name === 'call_skill')).toBe(false);
  fireEvent.keyDown(document, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(document.activeElement).toBe(buttons[0]);
});

it('AC20 adds an official skill and disables it without uninstalling bundled files', async () => {
  render(<SkillsManagement />);
  const add = await screen.findByRole('button', { name: 'Add to My Skills' });
  fireEvent.click(add);
  await screen.findByText(/Skill added to My Skills/);
  fireEvent.click(screen.getByRole('button', { name: 'Installed (1)' }));
  const toggle = screen.getByRole('checkbox', { name: 'Enable Summary' });
  fireEvent.click(toggle);
  await waitFor(() => expect(mockedInvoke).toHaveBeenCalledWith('toggle_skill', { skillId: official.skill_id, userId: 'default-user', enabled: false }));
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
