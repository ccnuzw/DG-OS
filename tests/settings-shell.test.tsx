// V1-FR-007 AC01, AC02, AC05, AC09, AC14 and B07-01, B07-14～B07-17 (D085): the Settings shell.
//
// jsdom with a mocked Tauri core and the real pages and stores. Proven here: /settings opens on an overview with no
// tab selected and no extra host call; the seven tabs are in order and each opens its page and URL; a deep link opens
// on its tab; arrow keys move focus without switching (manual activation) and Enter switches; Provider and Skills keep
// their unsaved UI state across tab switches; a failed load can be retried (Skills, About); the LAN entry calls no
// host command. NOT proven: layout, focus ring rendering, screen-reader output (real WebView smoke), and the Provider,
// Skills, MCP and module pages' own behaviour, which their own test files cover.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { Modal, ToastViewport, clearToasts } from '@dgos/ui';
import App from '../src/App';
import { useProviderStore } from '../src/stores/providerStore';
import { useSkillsStore } from '../src/stores/skillsStore';

const host = vi.hoisted(() => ({
  calls: [] as string[],
  failSkills: 0,
  handler: null as null | ((command: string) => unknown),
}));

const app = vi.hoisted(() => ({ failVersion: 0, versionCalls: 0 }));

vi.mock('@tauri-apps/api/core', () => {
  class Channel {
    onmessage: (message: unknown) => void = () => undefined;
  }
  return {
    Channel,
    invoke: async (command: string) => {
      host.calls.push(command);
      if (host.handler) return host.handler(command);
      return null;
    },
  };
});

vi.mock('@tauri-apps/api/app', () => ({
  getName: async () => 'DG-OS',
  getVersion: async () => {
    app.versionCalls += 1;
    if (app.failVersion > 0) {
      app.failVersion -= 1;
      throw new Error('ipc unavailable');
    }
    return '0.1.0';
  },
  getTauriVersion: async () => '2.9.0',
}));

const providerInitial = useProviderStore.getState();
const skillsInitial = useSkillsStore.getState();

const official = {
  skill_id: 'dgos.official.writing.summarize', name: 'Summary', latest_version: '1.0.0', category: 'writing',
  description: 'Evidence-based summary', author: 'DGOS Team', tags: [], identity: 'dgos_official',
  source_kind: 'dgos_official', license_spdx: 'Apache-2.0', compatible_apps: [],
};

function respond(command: string): unknown {
  if (command === 'list_providers') return { success: true, data: [] };
  if (command === 'list_provider_vendors') return { success: true, data: [] };
  if (command === 'list_mcp_servers') return [];
  if (command === 'list_modules') return { success: true, data: [] };
  if (command === 'fetch_skills_registry' || command === 'list_installed_skills') {
    if (host.failSkills > 0) {
      host.failSkills -= 1;
      throw new Error('skills unavailable');
    }
    return command === 'fetch_skills_registry'
      ? { skills: [official], registry_url: 'bundled://official', offline: false, warning: null }
      : [];
  }
  if (command === 'check_skill_updates') return [];
  return null;
}

function Where() {
  return <output data-testid="where">{useLocation().pathname}</output>;
}

function renderAt(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <App />
      <Where />
      <ToastViewport />
    </MemoryRouter>,
  );
}
const settingsTabs = () => within(screen.getByRole('tablist', { name: '系统设置' })).getAllByRole('tab');
const tab = (name: string) => within(screen.getByRole('tablist', { name: '系统设置' })).getByRole('tab', { name });
const where = () => screen.getByTestId('where').textContent;
const select = (name: string) => fireEvent.mouseDown(tab(name), { button: 0 });

beforeEach(() => {
  host.calls = [];
  host.failSkills = 0;
  host.handler = respond;
  app.failVersion = 0;
  app.versionCalls = 0;
  useProviderStore.setState(providerInitial, true);
  useSkillsStore.setState(skillsInitial, true);
  vi.spyOn(console, 'error').mockImplementation(() => undefined);
});

afterEach(() => {
  act(() => Modal.closeAll());
  act(() => clearToasts());
  cleanup();
  vi.restoreAllMocks();
});

describe('default entry (B07-15)', () => {
  it('opens on an overview with no tab selected and makes no host call of its own', async () => {
    renderAt('/');
    await screen.findByText('Welcome to DG-OS');
    const shellOnly = new Set(host.calls);
    cleanup();
    host.calls = [];

    renderAt('/settings');
    expect(await screen.findByRole('heading', { name: '系统设置' })).toBeTruthy();
    expect(settingsTabs().filter(t => t.getAttribute('aria-selected') === 'true')).toEqual([]);
    expect(host.calls.filter(command => !shellOnly.has(command))).toEqual([]);
  });

  it('lists the seven sections in order', () => {
    renderAt('/settings');
    expect(settingsTabs().map(t => t.textContent)).toEqual([
      'Provider', 'Skills', 'MCP', '模块', '外观', '关于', '局域网协同',
    ]);
  });
});

describe('tab navigation (B07-01)', () => {
  it.each([
    ['Provider', '/settings/providers', 'Provider Management'],
    ['Skills', '/settings/skills', 'DGOS Skills Center'],
    ['MCP', '/settings/mcp', 'MCP Management'],
    ['模块', '/settings/modules', 'Module management'],
    ['外观', '/settings/appearance', '外观设置'],
    ['关于', '/settings/about', '关于'],
    ['局域网协同', '/settings/lan', '局域网协同'],
  ])('the %s tab opens %s and its page, and only that tab is selected', async (label, path, heading) => {
    renderAt('/settings');
    select(label);
    expect(await screen.findByRole('heading', { level: 1, name: heading })).toBeTruthy();
    expect(where()).toBe(path);
    expect(settingsTabs().filter(t => t.getAttribute('aria-selected') === 'true').map(t => t.textContent)).toEqual([label]);
  });

  it.each([
    ['/settings/providers', 'Provider', 'Provider Management'],
    ['/settings/modules', '模块', 'Module management'],
    ['/settings/modules/dgos.beta', '模块', 'dgos.beta'],
    ['/settings/appearance', '外观', '外观设置'],
    ['/settings/lan', '局域网协同', '局域网协同'],
  ])('a direct load of %s opens on the %s tab', async (path, label, heading) => {
    renderAt(path);
    expect(await screen.findByRole('heading', { level: 1, name: heading })).toBeTruthy();
    expect(tab(label).getAttribute('aria-selected')).toBe('true');
  });

  it('moves focus with the arrow keys without switching, and switches on Enter', async () => {
    renderAt('/settings/providers');
    await screen.findByRole('heading', { level: 1, name: 'Provider Management' });
    act(() => tab('Provider').focus());

    fireEvent.keyDown(tab('Provider'), { key: 'ArrowRight' });
    await waitFor(() => expect(document.activeElement).toBe(tab('Skills')));
    expect(where()).toBe('/settings/providers');
    fireEvent.keyDown(tab('Skills'), { key: 'End' });
    await waitFor(() => expect(document.activeElement).toBe(tab('局域网协同')));
    fireEvent.keyDown(tab('局域网协同'), { key: 'ArrowLeft' });
    await waitFor(() => expect(document.activeElement).toBe(tab('关于')));
    expect(where()).toBe('/settings/providers');
    expect(tab('Provider').getAttribute('aria-selected')).toBe('true');

    fireEvent.keyDown(tab('关于'), { key: 'Enter' });
    expect(where()).toBe('/settings/about');
    expect(await screen.findByRole('heading', { level: 1, name: '关于' })).toBeTruthy();
    expect(tab('关于').getAttribute('aria-selected')).toBe('true');
  });
});

describe('local UI state across tabs (B07-16)', () => {
  it('keeps an unsaved Provider form when switching to another tab and back', async () => {
    renderAt('/settings/providers');
    fireEvent.click(await screen.findByRole('button', { name: '+ Create Provider' }));
    fireEvent.change(screen.getByLabelText('Name *'), { target: { value: 'Unsaved draft' } });

    select('关于');
    await screen.findByRole('heading', { level: 1, name: '关于' });
    expect(screen.queryByRole('heading', { level: 1, name: 'Provider Management' })).toBeNull();

    select('Provider');
    expect((screen.getByLabelText('Name *') as HTMLInputElement).value).toBe('Unsaved draft');
    expect(host.calls.filter(command => command === 'list_providers')).toHaveLength(1);
  });

  it('keeps the Skills search text and does not reload the library', async () => {
    renderAt('/settings/skills');
    const search = await screen.findByPlaceholderText(/Search/i);
    fireEvent.change(search, { target: { value: 'summary' } });

    select('MCP');
    await screen.findByRole('heading', { level: 1, name: 'MCP Management' });
    select('Skills');
    expect((screen.getByPlaceholderText(/Search/i) as HTMLInputElement).value).toBe('summary');
    expect(host.calls.filter(command => command === 'fetch_skills_registry')).toHaveLength(1);
  });
});

describe('load failure and retry', () => {
  it('Skills inside Settings keeps its own error and retry', async () => {
    host.failSkills = 2;
    renderAt('/settings/skills');
    fireEvent.click(await screen.findByRole('button', { name: 'Retry loading skills' }));
    expect(await screen.findByText('Summary')).toBeTruthy();
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Retry loading skills' })).toBeNull());
  });

  it('About shows the versions (AC14)', async () => {
    renderAt('/settings/about');
    expect(await screen.findByText('0.1.0')).toBeTruthy();
    expect(screen.getByText('DG-OS')).toBeTruthy();
    expect(screen.getByText('2.9.0')).toBeTruthy();
    expect(screen.getByText(/系统信息（操作系统与架构）、许可证信息和检查更新在本版本暂不提供/)).toBeTruthy();
  });

  it('About reports a failed read through the unified error Toast and recovers on retry (B07-17)', async () => {
    app.failVersion = 1;
    renderAt('/settings/about');
    expect(await screen.findByText('无法读取版本信息。')).toBeTruthy();
    expect(await screen.findByText('操作失败')).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: '重试' }));
    expect(await screen.findByText('0.1.0')).toBeTruthy();
    expect(screen.queryByText('无法读取版本信息。')).toBeNull();
    expect(app.versionCalls).toBe(2);
  });
});

describe('LAN entry (B07-14)', () => {
  it('says the capability is not enabled, shows no switch and calls no host command', async () => {
    renderAt('/settings');
    await screen.findByRole('heading', { name: '系统设置' });
    const before = host.calls.length;

    select('局域网协同');
    expect(await screen.findByText('局域网协同尚未启用')).toBeTruthy();
    expect(screen.queryByRole('switch')).toBeNull();
    expect(screen.queryByRole('checkbox')).toBeNull();
    expect(host.calls.slice(before)).toEqual([]);
    expect(host.calls.some(command => command.includes('lan'))).toBe(false);
  });
});
