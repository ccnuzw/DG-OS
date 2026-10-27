import { StrictMode } from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import SkillsManagement from '../src/pages/SkillsManagement';
import { useSkillsStore } from '../src/stores/skillsStore';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const mockedInvoke = vi.mocked(invoke);
const initialState = useSkillsStore.getState();
const skillInfo = {
  skill_id: 'calculator', name: 'Calculator', latest_version: '1.0.0',
  category: 'compute', tags: ['math'],
};
const manifest = {
  ...skillInfo, id: 'calculator', version: '1.0.0', call_type: 'cli',
  dependencies: {}, config_schema: null,
};
const installedSkill = {
  ...skillInfo, id: 'installed-1', version: '1.0.0', call_type: 'cli',
  enabled: true, manifest, installed_at: 1700000000,
};

beforeEach(() => {
  useSkillsStore.setState(initialState, true);
  mockedInvoke.mockImplementation(async (command) => {
    if (command === 'fetch_skills_registry') return { skills: [], registry_url: 'https://skills.dgos.ai/registry.json', offline: true, warning: 'TLS connection failed' };
    return [];
  });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe('Skills registry failure recovery', () => {
  it('deduplicates StrictMode initialization, shows offline empty state and retries successfully', async () => {
    const errors = vi.spyOn(console, 'error');
    render(<StrictMode><SkillsManagement /></StrictMode>);
    await screen.findByText('Skills registry unavailable');
    await screen.findByText(/No registry has been cached yet/);
    await screen.findByText(/No skills are available to browse/);
    expect(screen.getByText('Registry: https://skills.dgos.ai/registry.json')).toBeTruthy();
    expect(screen.queryByText('Offline mode')).toBeNull();
    for (const command of ['fetch_skills_registry', 'list_installed_skills', 'check_skill_updates']) {
      expect(mockedInvoke.mock.calls.filter(([name]) => name === command)).toHaveLength(1);
    }
    mockedInvoke.mockImplementation(async (command) => command === 'fetch_skills_registry'
      ? { skills: [skillInfo], registry_url: 'https://skills.dgos.ai/registry.json', offline: false, warning: null } : []);
    fireEvent.click(screen.getByRole('button', { name: 'Retry connection' }));
    await screen.findByText('Calculator');
    expect(screen.queryByText('Offline mode')).toBeNull();
    expect(screen.queryByText('Skills registry unavailable')).toBeNull();
    expect(useSkillsStore.getState().registryError).toBeNull();
    expect(errors).not.toHaveBeenCalled();
  });

  it('keeps cached skills searchable and installed skills manageable offline', async () => {
    mockedInvoke.mockImplementation(async (command) => {
      if (command === 'fetch_skills_registry') return { skills: [skillInfo], registry_url: 'https://skills.dgos.ai/registry.json', offline: true, warning: 'HTTP 503' };
      if (command === 'list_installed_skills') return [installedSkill];
      if (command === 'search_skills') return [skillInfo];
      if (command === 'get_skill_detail') return { ...skillInfo, manifest };
      return [];
    });
    render(<StrictMode><SkillsManagement /></StrictMode>);
    await screen.findByText('Offline mode');
    await screen.findByRole('button', { name: 'Installed (1)' });
    fireEvent.change(screen.getByRole('textbox', { name: 'Search skills' }), { target: { value: 'Calc' } });
    fireEvent.click(screen.getByRole('button', { name: 'Search', exact: true }));
    await waitFor(() => expect(mockedInvoke).toHaveBeenCalledWith('search_skills', { query: 'Calc' }));
    expect(screen.getByText('Offline mode')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Details' }));
    await screen.findByText('Call Type:');
    expect(screen.getByText('cli')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Close skill details' }));
    fireEvent.click(screen.getByRole('button', { name: 'Installed (1)' }));
    expect(screen.getByText('CLI')).toBeTruthy();
    fireEvent.click(screen.getByRole('checkbox'));
    await waitFor(() => expect(mockedInvoke).toHaveBeenCalledWith('toggle_skill', {
      skillId: 'calculator', userId: 'default-user', enabled: false,
    }));
  });

  it('handles IPC and initialization failures without unhandled rejections or console floods', async () => {
    const errors = vi.spyOn(console, 'error');
    mockedInvoke.mockRejectedValue('database unavailable');
    render(<StrictMode><SkillsManagement /></StrictMode>);
    await screen.findByText('Skills registry unavailable');
    await screen.findByText(/Unable to load installed skills/);
    await screen.findByText(/Unable to check skill updates/);
    expect(screen.queryByText('No skills installed yet.')).toBeNull();
    expect(useSkillsStore.getState().loading).toBe(false);
    expect(useSkillsStore.getState().installedLoading).toBe(false);
    expect(errors).not.toHaveBeenCalled();
  });

  it('distinguishes an empty search result from having no offline cache', async () => {
    mockedInvoke.mockImplementation(async (command) => command === 'fetch_skills_registry'
      ? { skills: [skillInfo], registry_url: 'https://skills.dgos.ai/registry.json', offline: true, warning: 'HTTP 503' }
      : []);
    render(<SkillsManagement />);
    await screen.findByText('Offline mode');
    fireEvent.change(screen.getByRole('textbox', { name: 'Search skills' }), { target: { value: 'missing' } });
    fireEvent.click(screen.getByRole('button', { name: 'Search', exact: true }));
    await screen.findByText('No cached skills match your filters.');
    expect(screen.getByText('Offline mode')).toBeTruthy();
    expect(screen.queryByText(/No registry has been cached yet/)).toBeNull();
  });

  it('contains search and detail action errors in the page', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    mockedInvoke.mockImplementation(async (command) => {
      if (command === 'fetch_skills_registry') return { skills: [skillInfo], registry_url: 'https://skills.dgos.ai/registry.json', offline: false, warning: null };
      if (command === 'search_skills' || command === 'get_skill_detail') throw 'read failed';
      return [];
    });
    render(<SkillsManagement />);
    await screen.findByText('Calculator');
    fireEvent.click(screen.getByRole('button', { name: 'Details' }));
    await screen.findByText('Unable to load skill details: read failed');
    fireEvent.click(screen.getByRole('button', { name: 'Search', exact: true }));
    await screen.findByText('Search failed: read failed');
  });

  it('does not let an installed-list response clear registry loading or start duplicate fetches', async () => {
    let resolveRegistry!: (value: unknown) => void;
    mockedInvoke.mockImplementation((command) => command === 'fetch_skills_registry'
      ? new Promise(resolve => { resolveRegistry = resolve; }) : Promise.resolve([]));
    const first = useSkillsStore.getState().fetchRegistry();
    const second = useSkillsStore.getState().fetchRegistry();
    await useSkillsStore.getState().listInstalledSkills('default-user');
    expect(useSkillsStore.getState().loading).toBe(true);
    expect(first).toBe(second);
    await act(async () => { resolveRegistry({ skills: [], registry_url: 'https://skills.dgos.ai/registry.json', offline: false, warning: null }); await first; });
    expect(useSkillsStore.getState().loading).toBe(false);
  });
});
