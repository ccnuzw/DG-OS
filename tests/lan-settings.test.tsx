// V1-FR-007 AC15 / V1-FR-010 (D039, D099, D107): the host-side LAN collaboration page.
//
// jsdom with a mocked Tauri core and the real page. Proven here: status, address and port-fallback text; an access
// code issued on open and on "重新生成"; the null-code message and the expired-code state; saving the preferred port
// (client-side range check, confirmation while running, host error 1010012, re-issue after a running change); device
// disconnect and disconnect-all behind confirmations; the switch-off confirmation; audit paging; that no code or
// token reaches localStorage or sessionStorage; and that polling stops on unmount. NOT proven: layout, copy to the
// real clipboard, and the Rust service behind the commands (FR-010 host tests).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { Modal, ModalHost, ToastViewport, clearToasts } from '@dgos/ui';
import LanSettings from '../src/pages/LanSettings';
import type { LanAuditEvent, LanDevice, LanIssuedCode, LanStatus } from '../src/lan/lanApi';
import { resetLocaleForTesting, setLocaleForTesting } from '../src/i18n/runtime';

const host = vi.hoisted(() => ({
  calls: [] as { command: string; args?: Record<string, unknown> }[],
  status: null as unknown as LanStatus,
  devices: [] as LanDevice[],
  issued: null as LanIssuedCode | null,
  issueQueue: [] as (LanIssuedCode | null)[],
  portError: 0,
  failStatus: 0,
  auditPages: [] as { items: LanAuditEvent[]; next_cursor: string | null }[],
  auditCursors: [] as (string | undefined)[],
}));

vi.mock('@tauri-apps/api/core', () => {
  class Channel {
    onmessage: (message: unknown) => void = () => undefined;
  }
  return {
    Channel,
    invoke: async (command: string, args?: Record<string, any>) => {
      host.calls.push({ command, args });
      switch (command) {
        case 'get_lan_status':
          if (host.failStatus > 0) {
            host.failStatus -= 1;
            return { success: false, error: { code: 1010001, message: 'boom' } };
          }
          return { success: true, data: { ...host.status } };
        case 'list_lan_devices':
          return { success: true, data: host.devices.map((device) => ({ ...device })) };
        case 'issue_lan_access_code':
          return { success: true, data: host.issueQueue.length > 0 ? host.issueQueue.shift() : host.issued };
        case 'set_lan_enabled':
          host.status = {
            ...host.status,
            enabled: args?.enabled,
            state: args?.enabled ? 'running' : 'stopped',
            address: args?.enabled ? '192.168.1.5:8635' : null,
          };
          if (!args?.enabled) host.devices = [];
          return { success: true, data: { ...host.status } };
        case 'set_lan_port':
          if (host.portError) return { success: false, error: { code: host.portError, message: 'bad port' } };
          host.status = { ...host.status, preferred_port: args?.port };
          if (host.status.state === 'running') host.status.address = `192.168.1.5:${args?.port}`;
          return { success: true, data: { ...host.status } };
        case 'disconnect_lan_device': {
          const before = host.devices.length;
          host.devices = host.devices.filter((device) => device.session_id !== args?.sessionId);
          return { success: true, data: host.devices.length < before };
        }
        case 'disconnect_all_lan_devices': {
          const count = host.devices.length;
          host.devices = [];
          return { success: true, data: count };
        }
        case 'list_lan_audit_events': {
          host.auditCursors.push(args?.input?.cursor);
          const page = host.auditPages.shift() ?? { items: [], next_cursor: null };
          return { success: true, data: page };
        }
        default:
          return null;
      }
    },
  };
});

const running = (patch: Partial<LanStatus> = {}): LanStatus => ({
  enabled: true,
  state: 'running',
  address: '192.168.1.5:8635',
  preferred_port: 8635,
  port_fallback: false,
  failure_code: null,
  session_count: 0,
  code_expires_at: null,
  ...patch,
});

const stopped = (patch: Partial<LanStatus> = {}): LanStatus => ({
  enabled: false,
  state: 'stopped',
  address: null,
  preferred_port: 8635,
  port_fallback: false,
  failure_code: null,
  session_count: 0,
  code_expires_at: null,
  ...patch,
});

const nowSeconds = () => Math.floor(Date.now() / 1000);
const issued = (code: string, ttl = 600): LanIssuedCode => ({
  code,
  expires_at: nowSeconds() + ttl,
  pair_url: `http://192.168.1.5:8635/#code=${code.replaceAll('-', '')}`,
});

const device = (n: number): LanDevice => ({
  session_id: `session-${n}`,
  short_id: `sid${n}`,
  label: `iPhone ${n}`,
  peer_ip: `192.168.1.${10 + n}`,
  created_at: 1_760_000_000 + n,
  last_active_at: 1_760_000_100 + n,
});

const audit = (n: number, patch: Partial<LanAuditEvent> = {}): LanAuditEvent => ({
  id: `audit-${n}`,
  first_at: 1_760_000_000 + n,
  last_at: 1_760_000_000 + n,
  event: 'pair_rejected',
  reason: 'invalid_code',
  peer_ip: '192.168.1.77',
  session_short_id: null,
  error_code: 1010004,
  count: 1,
  ...patch,
});

const count = (command: string) => host.calls.filter((call) => call.command === command).length;
const lastArgs = (command: string) => host.calls.filter((call) => call.command === command).at(-1)?.args;

async function openPage() {
  render(
    <MemoryRouter initialEntries={['/settings/lan']}>
      <LanSettings />
      <ToastViewport />
      <ModalHost />
    </MemoryRouter>,
  );
  await screen.findByRole('heading', { name: '局域网协同' });
  await screen.findByRole('switch', { name: '启用局域网协同' });
}

const dialog = () => screen.findByRole('dialog');
const portInput = () => screen.getByLabelText('首选端口') as HTMLInputElement;

beforeEach(() => {
  setLocaleForTesting('zh-CN');
  host.calls = [];
  host.status = running();
  host.devices = [];
  host.issued = issued('ABCD-EFGH-JKLM');
  host.issueQueue = [];
  host.portError = 0;
  host.failStatus = 0;
  host.auditPages = [];
  host.auditCursors = [];
  vi.spyOn(console, 'error').mockImplementation(() => undefined);
});

afterEach(() => {
  act(() => Modal.closeAll());
  act(() => clearToasts());
  cleanup();
  resetLocaleForTesting();
  vi.restoreAllMocks();
});

describe('status', () => {
  it('shows the state, the address, the actual and preferred port, and the unencrypted-network notice', async () => {
    host.status = running({ session_count: 2 });
    await openPage();

    expect(screen.getByText('仅在可信网络中使用：局域网连接未加密')).toBeTruthy();
    expect(screen.getByText('运行中')).toBeTruthy();
    expect(screen.getByText('http://192.168.1.5:8635')).toBeTruthy();
    expect(screen.getByText('当前端口').nextElementSibling?.textContent).toBe('8635');
    expect(screen.getAllByText('首选端口')[0].nextElementSibling?.textContent).toBe('8635');
    expect(screen.getByRole('switch', { name: '启用局域网协同' }).getAttribute('aria-checked')).toBe('true');
    expect(screen.queryByText(/被其他程序占用/)).toBeNull();
    expect(count('get_lan_status')).toBeGreaterThanOrEqual(1);
    expect(count('list_lan_devices')).toBeGreaterThanOrEqual(1);
  });

  it('explains a port fallback', async () => {
    host.status = running({ address: '192.168.1.5:8637', port_fallback: true });
    await openPage();

    expect(screen.getByText('首选端口 8635 被其他程序占用，当前使用 8637')).toBeTruthy();
    expect(screen.getByText('http://192.168.1.5:8637')).toBeTruthy();
    expect(screen.getByText('当前端口').nextElementSibling?.textContent).toBe('8637');
  });

  it.each([
    [1010001, '服务启动失败'],
    [1010002, '首选端口及其后 19 个端口都被占用'],
    [1010003, '没有可用的局域网连接'],
  ])('shows failure code %i as text and no address or access code', async (failure_code, text) => {
    host.status = stopped({ enabled: true, state: 'failed', failure_code });
    await openPage();

    expect(screen.getByText('失败')).toBeTruthy();
    expect(screen.getByText(text)).toBeTruthy();
    expect(screen.queryByText(/^http:\/\//)).toBeNull();
    expect(count('issue_lan_access_code')).toBe(0);
  });

  it('shows a stopped service without an address', async () => {
    host.status = stopped();
    await openPage();

    expect(screen.getByText('已停止')).toBeTruthy();
    expect(screen.getByRole('switch', { name: '启用局域网协同' }).getAttribute('aria-checked')).toBe('false');
    expect(screen.queryByText('访问地址')).toBeNull();
    expect(count('issue_lan_access_code')).toBe(0);
  });

  it('offers a retry when the first read fails, with one error Toast', async () => {
    host.failStatus = 1;
    render(
      <MemoryRouter>
        <LanSettings />
        <ToastViewport />
      </MemoryRouter>,
    );

    expect(await screen.findByText('无法读取局域网协同状态。')).toBeTruthy();
    expect(await screen.findByText('操作失败（错误码 1010001）')).toBeTruthy();
    expect(screen.queryByRole('switch')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: '重试' }));
    expect(await screen.findByRole('switch', { name: '启用局域网协同' })).toBeTruthy();
    expect(screen.queryByText('无法读取局域网协同状态。')).toBeNull();
  });
});

describe('access code', () => {
  it('is issued when the page opens, grouped, with its expiry and a copyable pair URL', async () => {
    const code = issued('ABCD-EFGH-JKLM');
    host.issued = code;
    await openPage();

    expect((await screen.findByTestId('lan-code')).textContent).toBe('ABCD-EFGH-JKLM');
    expect(screen.getByTestId('lan-pair-url').textContent).toBe(code.pair_url);
    expect(count('issue_lan_access_code')).toBe(1);
  });

  it('copies the code and the pair URL', async () => {
    const writeText = vi.fn(async () => undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    await openPage();
    await screen.findByTestId('lan-code');

    fireEvent.click(screen.getByRole('button', { name: '复制凭据' }));
    fireEvent.click(screen.getByRole('button', { name: '复制链接' }));
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(2));
    expect(writeText).toHaveBeenNthCalledWith(1, 'ABCD-EFGH-JKLM');
    expect(writeText).toHaveBeenNthCalledWith(2, host.issued!.pair_url);
  });

  it('is replaced by a new one on 重新生成', async () => {
    host.issueQueue = [issued('AAAA-BBBB-CCCC'), issued('DDDD-EEEE-FFFF')];
    await openPage();
    expect((await screen.findByTestId('lan-code')).textContent).toBe('AAAA-BBBB-CCCC');

    fireEvent.click(screen.getByRole('button', { name: '重新生成' }));
    await waitFor(() => expect(screen.getByTestId('lan-code').textContent).toBe('DDDD-EEEE-FFFF'));
    expect(count('issue_lan_access_code')).toBe(2);
    expect(screen.queryByText('AAAA-BBBB-CCCC')).toBeNull();
  });

  it('shows the not-running/not-signed-in message when the host returns null', async () => {
    host.issued = null;
    await openPage();

    expect(await screen.findByText('服务未运行或尚未登录，无法生成访问凭据')).toBeTruthy();
    expect(screen.queryByTestId('lan-code')).toBeNull();
    expect(screen.queryByTestId('lan-pair-url')).toBeNull();
  });

  it('shows the message and no code while the service is stopped, and 重新生成 is disabled', async () => {
    host.status = stopped();
    await openPage();

    expect(screen.getByText('服务未运行或尚未登录，无法生成访问凭据')).toBeTruthy();
    expect((screen.getByRole('button', { name: '重新生成' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('shows an expired code as expired, hides it and regenerates on request', async () => {
    host.issueQueue = [issued('AAAA-BBBB-CCCC', -5), issued('DDDD-EEEE-FFFF')];
    await openPage();

    expect(await screen.findByText('访问凭据已过期，请点击“重新生成”。')).toBeTruthy();
    expect(screen.queryByTestId('lan-code')).toBeNull();
    expect(screen.queryByTestId('lan-pair-url')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: '重新生成' }));
    expect((await screen.findByTestId('lan-code')).textContent).toBe('DDDD-EEEE-FFFF');
  });
});

describe('preferred port', () => {
  it('saves a new port while the service is stopped without a confirmation', async () => {
    host.status = stopped();
    await openPage();
    expect(portInput().value).toBe('8635');

    fireEvent.change(portInput(), { target: { value: '9000' } });
    fireEvent.click(screen.getByRole('button', { name: '保存' }));

    await waitFor(() => expect(lastArgs('set_lan_port')).toEqual({ port: 9000 }));
    expect(screen.queryByRole('dialog')).toBeNull();
    await waitFor(() => expect(screen.getAllByText('首选端口')[0].nextElementSibling?.textContent).toBe('9000'));
    expect(count('issue_lan_access_code')).toBe(0);
  });

  it.each(['80', '1023', '65536', '', '8.5', 'abc'])('rejects %j on the client without calling the host', async (value) => {
    host.status = stopped();
    await openPage();

    fireEvent.change(portInput(), { target: { value } });
    fireEvent.click(screen.getByRole('button', { name: '保存' }));

    expect(await screen.findByText('端口须为 1024～65535 的整数')).toBeTruthy();
    expect(count('set_lan_port')).toBe(0);
  });

  it('shows the host rejection 1010012 as an inline message', async () => {
    host.status = stopped();
    host.portError = 1010012;
    await openPage();

    fireEvent.change(portInput(), { target: { value: '6000' } });
    fireEvent.click(screen.getByRole('button', { name: '保存' }));

    expect(await screen.findByText('端口不可用：须为 1024～65535，且不是浏览器禁止的端口')).toBeTruthy();
    expect(lastArgs('set_lan_port')).toEqual({ port: 6000 });
  });

  it('asks for confirmation when a running service would change port, and does nothing on cancel', async () => {
    await openPage();
    await screen.findByTestId('lan-code');

    fireEvent.change(portInput(), { target: { value: '9000' } });
    fireEvent.click(screen.getByRole('button', { name: '保存' }));
    const confirmDialog = await dialog();
    expect(within(confirmDialog).getByText(/会断开全部设备/)).toBeTruthy();
    expect(count('set_lan_port')).toBe(0);

    fireEvent.click(within(confirmDialog).getByRole('button', { name: '取消' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(count('set_lan_port')).toBe(0);
  });

  it('on confirm saves the port, refreshes the status and issues a new code', async () => {
    host.issueQueue = [issued('AAAA-BBBB-CCCC'), issued('DDDD-EEEE-FFFF')];
    host.devices = [device(1)];
    await openPage();
    expect((await screen.findByTestId('lan-code')).textContent).toBe('AAAA-BBBB-CCCC');

    fireEvent.change(portInput(), { target: { value: '9000' } });
    fireEvent.click(screen.getByRole('button', { name: '保存' }));
    fireEvent.click(within(await dialog()).getByRole('button', { name: '修改并断开' }));

    await waitFor(() => expect(lastArgs('set_lan_port')).toEqual({ port: 9000 }));
    await waitFor(() => expect(screen.getByTestId('lan-code').textContent).toBe('DDDD-EEEE-FFFF'));
    expect(count('issue_lan_access_code')).toBe(2);
    expect(screen.getByText('http://192.168.1.5:9000')).toBeTruthy();
  });

  it('saves without a confirmation when the value equals the port in use', async () => {
    host.status = running({ address: '192.168.1.5:8637', port_fallback: true });
    await openPage();
    await screen.findByTestId('lan-code');

    fireEvent.change(portInput(), { target: { value: '8637' } });
    fireEvent.click(screen.getByRole('button', { name: '保存' }));

    await waitFor(() => expect(lastArgs('set_lan_port')).toEqual({ port: 8637 }));
    expect(screen.queryByRole('dialog')).toBeNull();
  });
});

describe('switch', () => {
  it('confirms before switching off and then calls set_lan_enabled(false)', async () => {
    host.devices = [device(1)];
    await openPage();
    await screen.findByTestId('lan-code');

    fireEvent.click(screen.getByRole('switch', { name: '启用局域网协同' }));
    const confirmDialog = await dialog();
    expect(within(confirmDialog).getByText(/关闭后会断开全部设备/)).toBeTruthy();
    expect(count('set_lan_enabled')).toBe(0);

    fireEvent.click(within(confirmDialog).getByRole('button', { name: '确认关闭' }));
    await waitFor(() => expect(lastArgs('set_lan_enabled')).toEqual({ enabled: false }));
    await waitFor(() => expect(screen.getByText('已停止')).toBeTruthy());
    expect(screen.queryByTestId('lan-code')).toBeNull();
    expect(screen.getByText('当前没有已连接的设备')).toBeTruthy();
  });

  it('leaves the service running when the confirmation is cancelled', async () => {
    await openPage();

    fireEvent.click(screen.getByRole('switch', { name: '启用局域网协同' }));
    fireEvent.click(within(await dialog()).getByRole('button', { name: '取消' }));

    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(count('set_lan_enabled')).toBe(0);
    expect(screen.getByRole('switch', { name: '启用局域网协同' }).getAttribute('aria-checked')).toBe('true');
  });

  it('switches on at once, without a confirmation, and issues a code', async () => {
    host.status = stopped();
    await openPage();

    fireEvent.click(screen.getByRole('switch', { name: '启用局域网协同' }));

    await waitFor(() => expect(lastArgs('set_lan_enabled')).toEqual({ enabled: true }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect((await screen.findByTestId('lan-code')).textContent).toBe('ABCD-EFGH-JKLM');
    expect(screen.getByText('运行中')).toBeTruthy();
  });
});

describe('devices', () => {
  it('lists label, IP, short id and times', async () => {
    host.devices = [device(1), device(2)];
    await openPage();

    const table = await screen.findByRole('table', { name: '已连接设备' });
    const rows = within(table).getAllByRole('row');
    expect(rows).toHaveLength(3);
    const first = within(rows[1]).getAllByRole('cell').map((c) => c.textContent);
    expect(first.slice(0, 3)).toEqual(['iPhone 1', '192.168.1.11', 'sid1']);
  });

  it('disconnects one device after a confirmation and refreshes the list', async () => {
    host.devices = [device(1), device(2)];
    await openPage();
    await screen.findByRole('table', { name: '已连接设备' });

    fireEvent.click(screen.getByRole('button', { name: '断开 iPhone 1' }));
    const confirmDialog = await dialog();
    expect(within(confirmDialog).getByText(/iPhone 1/)).toBeTruthy();
    expect(count('disconnect_lan_device')).toBe(0);

    fireEvent.click(within(confirmDialog).getByRole('button', { name: '断开' }));
    await waitFor(() => expect(lastArgs('disconnect_lan_device')).toEqual({ sessionId: 'session-1' }));
    await waitFor(() => expect(screen.queryByRole('button', { name: '断开 iPhone 1' })).toBeNull());
    expect(screen.getByRole('button', { name: '断开 iPhone 2' })).toBeTruthy();
  });

  it('does not disconnect when the confirmation is cancelled', async () => {
    host.devices = [device(1)];
    await openPage();
    await screen.findByRole('table', { name: '已连接设备' });

    fireEvent.click(screen.getByRole('button', { name: '断开 iPhone 1' }));
    fireEvent.click(within(await dialog()).getByRole('button', { name: '取消' }));

    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(count('disconnect_lan_device')).toBe(0);
    expect(screen.getByRole('button', { name: '断开 iPhone 1' })).toBeTruthy();
  });

  it('disconnects all devices after a confirmation', async () => {
    host.devices = [device(1), device(2)];
    await openPage();
    await screen.findByRole('table', { name: '已连接设备' });

    fireEvent.click(screen.getByRole('button', { name: '断开全部' }));
    const confirmDialog = await dialog();
    expect(count('disconnect_all_lan_devices')).toBe(0);
    fireEvent.click(within(confirmDialog).getByRole('button', { name: '断开全部' }));

    await waitFor(() => expect(count('disconnect_all_lan_devices')).toBe(1));
    expect(await screen.findByText('当前没有已连接的设备')).toBeTruthy();
    expect((screen.getByRole('button', { name: '断开全部' }) as HTMLButtonElement).disabled).toBe(true);
  });
});

describe('audit records', () => {
  it('lists events with Chinese labels and loads more with the cursor', async () => {
    host.auditPages = [
      { items: [audit(2, { count: 3 }), audit(1, { event: 'session_revoked', reason: 'host_disconnect', error_code: null, session_short_id: 'sid1' })], next_cursor: 'c1' },
      { items: [audit(0, { event: 'pair_succeeded', reason: null, error_code: null })], next_cursor: null },
    ];
    await openPage();

    const table = await screen.findByRole('table', { name: '审计记录' });
    expect(within(table).getAllByRole('row')).toHaveLength(3);
    expect(within(table).getByText('配对被拒绝')).toBeTruthy();
    expect(within(table).getByText('访问凭据无效或已使用')).toBeTruthy();
    expect(within(table).getByText('会话被撤销')).toBeTruthy();
    expect(within(table).getByText('主机断开该设备')).toBeTruthy();
    expect(within(table).getByText('sid1')).toBeTruthy();
    expect(within(table).getByText('3')).toBeTruthy();
    expect(host.auditCursors).toEqual([undefined]);

    fireEvent.click(screen.getByRole('button', { name: '加载更多' }));
    await waitFor(() => expect(within(screen.getByRole('table', { name: '审计记录' })).getAllByRole('row')).toHaveLength(4));
    expect(host.auditCursors).toEqual([undefined, 'c1']);
    expect(screen.getByText('配对成功')).toBeTruthy();
    expect(screen.queryByRole('button', { name: '加载更多' })).toBeNull();
  });

  it('says so when there are no records', async () => {
    await openPage();
    expect(await screen.findByText('暂无审计记录')).toBeTruthy();
  });
});

describe('secrets and polling', () => {
  it('never writes the code, pair URL or anything else to localStorage or sessionStorage', async () => {
    const setItem = vi.spyOn(Storage.prototype, 'setItem');
    host.devices = [device(1)];
    await openPage();
    await screen.findByTestId('lan-code');

    fireEvent.click(screen.getByRole('button', { name: '重新生成' }));
    await waitFor(() => expect(count('issue_lan_access_code')).toBe(2));
    fireEvent.change(portInput(), { target: { value: '9000' } });
    fireEvent.click(screen.getByRole('button', { name: '保存' }));
    fireEvent.click(within(await dialog()).getByRole('button', { name: '修改并断开' }));
    await waitFor(() => expect(count('set_lan_port')).toBe(1));
    await waitFor(() => expect(count('issue_lan_access_code')).toBe(3));

    expect(setItem).not.toHaveBeenCalled();
    expect(localStorage.length).toBe(0);
    expect(sessionStorage.length).toBe(0);
  });

  it('clears the code from the page when it unmounts', async () => {
    await openPage();
    await screen.findByTestId('lan-code');
    cleanup();
    expect(screen.queryByText('ABCD-EFGH-JKLM')).toBeNull();
    expect(document.body.textContent).not.toContain('ABCD-EFGH-JKLM');
  });

  it('polls status and devices every 5 s while mounted and stops on unmount', async () => {
    const setIntervalSpy = vi.spyOn(window, 'setInterval');
    const clearIntervalSpy = vi.spyOn(window, 'clearInterval');
    await openPage();
    await screen.findByTestId('lan-code');

    const index = setIntervalSpy.mock.calls.findIndex((call) => call[1] === 5000);
    expect(index).toBeGreaterThanOrEqual(0);
    const tick = setIntervalSpy.mock.calls[index][0] as () => void;
    const timerId = setIntervalSpy.mock.results[index].value;

    const statusBefore = count('get_lan_status');
    const devicesBefore = count('list_lan_devices');
    await act(async () => { tick(); });
    expect(count('get_lan_status')).toBe(statusBefore + 1);
    expect(count('list_lan_devices')).toBe(devicesBefore + 1);
    // The code is issued once per running period, not on every poll.
    expect(count('issue_lan_access_code')).toBe(1);

    cleanup();
    expect(clearIntervalSpy).toHaveBeenCalledWith(timerId);
  });

  it('shows devices that connected between polls', async () => {
    const setIntervalSpy = vi.spyOn(window, 'setInterval');
    await openPage();
    expect(await screen.findByText('当前没有已连接的设备')).toBeTruthy();

    host.devices = [device(3)];
    host.status = running({ session_count: 1 });
    const tick = setIntervalSpy.mock.calls.find((call) => call[1] === 5000)![0] as () => void;
    await act(async () => { tick(); });

    expect(await screen.findByRole('button', { name: '断开 iPhone 3' })).toBeTruthy();
  });
});

it('shows English confirmation copy and preserves device identity in interpolation',async()=>{
  setLocaleForTesting('en-US');host.devices=[device(1)];
  render(<MemoryRouter><LanSettings/><ModalHost/></MemoryRouter>);
  const table=await screen.findByRole('table',{name:'Connected devices'});
  fireEvent.click(await within(table).findByRole('button',{name:'Disconnect iPhone 1'}));
  const prompt=await screen.findByRole('dialog',{name:'Disconnect device?'});
  expect(prompt.textContent).toContain('iPhone 1');expect(prompt.textContent).toContain('192.168.1.11');
  fireEvent.click(within(prompt).getByRole('button',{name:'Cancel'}));
  expect(count('disconnect_lan_device')).toBe(0);
});
