// Settings category navigation, preserved Provider drafts, resource redirects and management behavior.
// jsdom uses mock IPC; renderer/focus evidence is recorded separately in implementation status.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { Modal, ToastViewport, clearToasts } from '@dgos/ui';
import App from '../src/App';
import { useProviderStore } from '../src/stores/providerStore';
import { useSkillsStore } from '../src/stores/skillsStore';
import { setLocaleForTesting } from '../src/i18n/runtime';

const host = vi.hoisted(() => ({
  calls: [] as string[],
  args: [] as Array<[string, Record<string, unknown> | undefined]>,
  failSkills: 0,
  handler: null as null | ((command: string, args?: Record<string, unknown>) => unknown),
}));

const app = vi.hoisted(() => ({ failVersion: 0, versionCalls: 0 }));

vi.mock('@tauri-apps/api/core', () => {
  class Channel {
    onmessage: (message: unknown) => void = () => undefined;
  }
  return {
    Channel,
    invoke: async (command: string, args?: Record<string, unknown>) => {
      host.calls.push(command);
      host.args.push([command, args]);
      if (host.handler) return host.handler(command, args);
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
  if (command === 'get_lan_status') {
    return {
      success: true,
      data: {
        enabled: false, state: 'stopped', address: null, preferred_port: 8635, port_fallback: false,
        failure_code: null, session_count: 0, code_expires_at: null,
      },
    };
  }
  if (command === 'list_lan_devices') return { success: true, data: [] };
  if (command === 'list_lan_audit_events') return { success: true, data: { items: [], next_cursor: null } };
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
const select = async (name: string) => { await screen.findByRole('tablist', { name: '系统设置' }); fireEvent.mouseDown(tab(name), { button: 0 }); };

beforeEach(() => {
  setLocaleForTesting('zh-CN');
  host.calls = [];
  host.args = [];
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
    await screen.findByRole('heading', { name: '桌面总览' });
    const shellOnly = new Set(host.calls);
    cleanup();
    host.calls = [];

    renderAt('/settings');
    expect(await screen.findByRole('heading', { name: '系统设置' })).toBeTruthy();
    await screen.findByRole('tablist', { name: '系统设置' });
    expect(settingsTabs().filter(t => t.getAttribute('aria-selected') === 'true')).toEqual([]);
    expect(host.calls.filter(command => !shellOnly.has(command))).toEqual([]);
  });

});

describe('tab navigation (B07-01)', () => {
  it.each([
    ['Provider', '/settings/providers', 'Provider 管理'],
    ['模块', '/settings/modules', '模块管理'],
    ['外观', '/settings/appearance', '外观设置'],
    ['关于', '/settings/about', '关于'],
    ['局域网协同', '/settings/lan', '局域网协同'],
  ])('the %s tab opens %s and its page, and only that tab is selected', async (label, path, heading) => {
    renderAt('/settings');
    await select(label);
    if (path === '/settings/providers') await screen.findByRole('button', { name: '+ 新建 Provider' });
    else expect(await screen.findByRole('heading', { level: 1, name: heading })).toBeTruthy();
    expect(where()).toBe(path);
    expect(settingsTabs().filter(t => t.getAttribute('aria-selected') === 'true').map(t => t.textContent)).toEqual([label]);
  });

  it.each([
    ['/settings/providers', 'Provider', 'Provider 管理'],
    ['/settings/modules', '模块', '模块管理'],
    ['/settings/appearance', '外观', '外观设置'],
    ['/settings/lan', '局域网协同', '局域网协同'],
  ])('a direct load of %s opens on the %s tab', async (path, label, heading) => {
    renderAt(path);
    if (path === '/settings/providers') await screen.findByRole('button', { name: '+ 新建 Provider' });
    else expect(await screen.findByRole('heading', { level: 1, name: heading })).toBeTruthy();
    expect(tab(label).getAttribute('aria-selected')).toBe('true');
  });

  it('moves focus with the arrow keys without switching, and switches on Enter', async () => {
    renderAt('/settings/providers');
    await screen.findByRole('button', { name: '+ 新建 Provider' });
    act(() => tab('Provider').focus());

    fireEvent.keyDown(tab('Provider'), { key: 'ArrowDown' });
    await waitFor(() => expect(document.activeElement).toBe(tab('模块')));
    expect(where()).toBe('/settings/providers');
    expect(tab('Provider').getAttribute('aria-selected')).toBe('true');

    fireEvent.keyDown(tab('局域网协同'), { key: 'Enter' });
    await waitFor(() => expect(where()).toBe('/settings/lan'));
    expect(await screen.findByRole('heading', { level: 1, name: '局域网协同' })).toBeTruthy();
    expect(tab('局域网协同').getAttribute('aria-selected')).toBe('true');
  });
});

describe('local UI state across tabs (B07-16)', () => {
  it('keeps an unsaved Provider form when switching to another tab and back', async () => {
    renderAt('/settings/providers');
    fireEvent.click(await screen.findByRole('button', { name: '+ 创建 Provider' }));
    fireEvent.change(screen.getByLabelText('名称 *'), { target: { value: 'Unsaved draft' } });

    await select('关于');
    await screen.findByRole('heading', { level: 1, name: '关于' });
    expect(screen.queryByRole('heading', { level: 1, name: 'Provider 管理' })).toBeNull();

    await select('Provider');
    expect((screen.getByLabelText('名称 *') as HTMLInputElement).value).toBe('Unsaved draft');
    expect(host.calls.filter(command => command === 'list_providers')).toHaveLength(1);
  });

  it('keeps the Skills search text and does not reload the library', async () => {
    renderAt('/settings/skills');
    const search = await screen.findByPlaceholderText(/搜索/i);
    fireEvent.change(search, { target: { value: 'summary' } });

    fireEvent.click(screen.getByRole('link', { name: 'MCP 管理' }));
    await screen.findByRole('heading', { level: 1, name: 'MCP 管理' });
    fireEvent.click(screen.getByRole('link', { name: '技能中心' }));
    expect((screen.getByPlaceholderText(/搜索/i) as HTMLInputElement).value).toBe('summary');
    expect(host.calls.filter(command => command === 'fetch_skills_registry')).toHaveLength(1);
  });
});

describe('load failure and retry', () => {
  it('Skills inside Settings keeps its own error and retry', async () => {
    host.failSkills = 2;
    renderAt('/settings/skills');
    fireEvent.click(await screen.findByRole('button', { name: '重试加载 Skills' }));
    expect(await screen.findByText('Summary')).toBeTruthy();
    await waitFor(() => expect(screen.queryByRole('button', { name: '重试加载 Skills' })).toBeNull());
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

describe('LAN entry (B07-14, AC15)', () => {
  it('opens the LAN management page: reads the status through the LAN host commands and shows the switch', async () => {
    renderAt('/settings');
    await screen.findByRole('heading', { name: '系统设置' });
    const before = host.calls.length;

    await select('局域网协同');
    expect(await screen.findByRole('heading', { name: '局域网协同' })).toBeTruthy();
    expect(await screen.findByRole('switch', { name: '启用局域网协同' })).toBeTruthy();
    expect(screen.getByText('仅在可信网络中使用：局域网连接未加密')).toBeTruthy();
    expect(screen.queryByText('局域网协同尚未启用')).toBeNull();
    expect(where()).toBe('/settings/lan');

    const lanCalls = host.calls.slice(before).filter(command => command.includes('lan'));
    expect(lanCalls).toContain('get_lan_status');
    expect(lanCalls).toContain('list_lan_devices');
    // The service is stopped, so no access code is requested.
    expect(lanCalls).not.toContain('issue_lan_access_code');
  });
});

// Management uses settings for providers and shared workspaces for Skills/MCP.
describe('management through product navigation', () => {
  const created = {
    id: 'provider-new', name: 'Team gateway', provider_type: 'openai_compatible', base_url: 'https://gateway.example.invalid/v1',
    enabled: true, sort_order: 0, created_at: '2026-11-01T00:00:00Z', updated_at: '2026-11-01T00:00:00Z',
    credential_status: 'stored', config_revision: 0,
  };
  const argsOf = (command: string) => host.args.filter(([name]) => name === command).map(([, args]) => args);

  async function openNewProviderForm() {
    renderAt('/settings');
    await select('Provider');
    fireEvent.click(await screen.findByRole('button', { name: '+ 创建 Provider' }));
    fireEvent.change(screen.getByLabelText('名称 *'), { target: { value: created.name } });
    fireEvent.change(screen.getByLabelText('基础 URL *'), { target: { value: created.base_url } });
    fireEvent.change(screen.getByLabelText('API Key *'), { target: { value: 'synthetic-key' } });
  }

  it('AC03 adds a Provider from the Provider tab and lists it', async () => {
    host.handler = command => (command === 'create_provider' ? { success: true, data: created } : respond(command));
    await openNewProviderForm();

    fireEvent.click(screen.getByRole('button', { name: /创建$/ }));

    expect(await screen.findByText(created.name)).toBeTruthy();
    await waitFor(() => expect(screen.queryByLabelText('API Key *')).toBeNull());
    expect(where()).toBe('/settings/providers');
    const [request] = argsOf('create_provider');
    expect(request).toEqual({ config: expect.objectContaining({ name: created.name, base_url: created.base_url, api_key: 'synthetic-key' }) });
    expect(JSON.stringify(request)).not.toMatch(/user_?id/i);
  });

  it.each([
    ['success', { success: true, data: true }, '连接成功'],
    ['failure', { success: false, error: { code: 1001002, message: 'upstream said 401' } }, '请检查 API Key 和 Provider 权限。'],
  ])('AC04 tests the connection from the Provider tab and shows the %s result', async (_, reply, text) => {
    host.handler = command => (command === 'test_provider_connection' ? reply : respond(command));
    await openNewProviderForm();

    fireEvent.click(screen.getByRole('button', { name: /测试连接/ }));

    expect(await screen.findByText(text)).toBeTruthy();
    expect(argsOf('test_provider_connection')).toEqual([{ config: expect.objectContaining({ base_url: created.base_url }) }]);
    expect(host.calls).not.toContain('create_provider');
  });

  it('AC06 adds an official local skill to My Skills from the Skills tab; legacy entries stay read-only', async () => {
    const legacy = { ...official, skill_id: 'old-calculator', name: 'Old Calculator', identity: 'legacy_registry', source_kind: 'legacy_registry' };
    let installed: unknown[] = [];
    host.handler = command => {
      if (command === 'fetch_skills_registry') return { skills: [official, legacy], registry_url: 'bundled://official', offline: false, warning: null };
      if (command === 'list_installed_skills') return installed;
      if (command === 'check_dependencies') return { satisfied: true, missing: [] };
      if (command === 'install_skill') {
        const manifest = { id: official.skill_id, name: official.name, version: '1.0.0', category: 'writing', tags: [], call_type: 'content', dependencies: {} };
        installed = [{ ...official, id: 'version-1', version: '1.0.0', installed_at: 1, install_path: 'bundled/official', manifest, call_type: 'content', enabled: true }];
        return installed[0];
      }
      return respond(command);
    };
    renderAt('/settings/skills');
    expect(await screen.findByText('Old Calculator')).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: '添加到我的 Skills' }));

    expect(await screen.findByText(/Skill 已添加到我的 Skills/)).toBeTruthy();
    expect(argsOf('install_skill')).toEqual([{ skillId: official.skill_id }]);
    fireEvent.mouseDown(screen.getByRole('tab', { name: /已安装（1/ }), { button: 0 });
    expect(screen.getByRole('switch', { name: '禁用 Summary' }).getAttribute('aria-checked')).toBe('true');
    // Adding stores a local content version; nothing is executed, downloaded or configured.
    expect(host.calls.filter(command => ['call_skill', 'update_skill', 'update_skill_config'].includes(command))).toEqual([]);
    expect(where()).toBe('/skills');
  });

  it('AC07 adds an MCP server from the MCP tab and lists it without connecting', async () => {
    const server = { id: 'mcp-new', name: 'Local files', transport: 'stdio', enabled: false, status: 'configured' };
    host.handler = command => (command === 'create_mcp_server' ? server : respond(command));
    renderAt('/settings/mcp');
    await screen.findByRole('heading', { level: 1, name: 'MCP 管理' });
    fireEvent.click(screen.getByRole('button', { name: '添加 MCP Server' }));
    fireEvent.change(screen.getByRole('textbox', { name: '名称', exact: true }), { target: { value: server.name } });
    fireEvent.change(screen.getByLabelText('可执行文件'), { target: { value: '/usr/local/bin/files-server' } });

    fireEvent.click(screen.getByRole('button', { name: '保存 Server' }));

    expect(await screen.findByRole('button', { name: `选择 ${server.name}` })).toBeTruthy();
    expect(argsOf('create_mcp_server')).toEqual([{ request: { name: server.name, transport: 'stdio', stdio: { executable: '/usr/local/bin/files-server', args: [] } } }]);
    expect(host.calls).not.toContain('connect_mcp_server');
    expect(where()).toBe('/mcp');
  });

  it('AC08 shows each MCP server\'s connection state as text and as a success or error colour from the MCP tab', async () => {
    host.handler = command => (command === 'list_mcp_servers'
      ? [
          { id: 'up', name: 'Up server', transport: 'stdio', enabled: true, status: 'ready' },
          { id: 'down', name: 'Down server', transport: 'http', enabled: true, status: 'error' },
        ]
      : respond(command));
    renderAt('/settings/mcp');

    for (const [name, colour] of [['Up server', 'bg-success'], ['Down server', 'bg-error']]) {
      const row = await screen.findByRole('button', { name: `选择 ${name}` });
      expect(row.querySelector('[aria-hidden="true"]')?.classList.contains(colour)).toBe(true);
    }
  });
});
