import { StrictMode } from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { Modal } from '@dgos/ui';
import CliSettings from '../src/pages/CliSettings';
import { resetLocaleForTesting, setLocaleForTesting } from '../src/i18n/runtime';

const { confirmDiscard, picker, guard } = vi.hoisted(() => ({ confirmDiscard: vi.fn(), picker: vi.fn(), guard: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: picker }));
vi.mock('../src/workspace/closeCoordinator', () => ({ confirmDiscard }));
vi.mock('../src/workspace/SurfaceContext', () => ({ useSurfaceCloseGuard: guard }));
const call = vi.mocked(invoke);
const tools = ['codex', 'claude', 'gemini', 'node'] as const;
type Tool = typeof tools[number];
interface Probe { status: string; path: string | null; version: string | null }
interface Offer { status: string; current_version: string | null; latest_version: string | null; method: string | null }
interface Detection extends Probe { update: Offer | null }
interface Snapshot { revision: number; paths: Record<Tool, string | null>; detections: Partial<Record<Tool, Detection>> }
const ok = (data: unknown) => ({ success: true, data: structuredClone(data) });
let database: Snapshot;
let check: Offer;
let local: Record<Tool, Probe>;
let selectedNode: Probe;
let updateStatus: string;
let issued: number;
function persist(tool: Tool, probe: Probe) {
  database.paths[tool] = probe.path;
  database.detections[tool] = { ...probe, update: null };
  database.revision += 1;
}
async function host(command: string, args?: unknown): Promise<unknown> {
  const input = (args ?? {}) as { tool: Tool; path: string | null; paths: Snapshot['paths']; expectedRevision: number };
  switch (command) {
    case 'get_cli_preferences': return ok(database);
    case 'probe_cli': persist(input.tool, local[input.tool]); return ok(local[input.tool]);
    case 'check_cli_update': {
      const offer = { ...check, current_version: database.detections[input.tool]?.version ?? null };
      const record = database.detections[input.tool];
      if (record) { record.update = offer; database.revision += 1; }
      return ok({ ...offer, ticket: offer.status === 'available' ? `ticket-${++issued}` : null });
    }
    case 'probe_node_directory': return ok(selectedNode);
    case 'set_cli_preferences':
      if (database.revision !== input.expectedRevision) return ok({ saved: false, snapshot: database });
      database.paths = { ...input.paths };
      persist('node', selectedNode);
      return ok({ saved: true, snapshot: database });
    case 'update_cli': {
      const probe = updateStatus === 'updated' ? { status: 'available', path: '/detected/codex', version: 'codex-cli 1.1.0' } : null;
      if (probe) persist('codex', probe);
      return ok({ status: updateStatus, probe });
    }
    default: throw Error(`Unexpected fixture command: ${command}`);
  }
}
beforeEach(() => {
  database = { revision: 2, paths: { codex: '/old/codex', claude: null, gemini: null, node: null }, detections: {} };
  local = {
    codex: { status: 'available', path: '/detected/codex', version: 'codex-cli 1.0.0' },
    claude: { status: 'not_installed', path: null, version: null },
    gemini: { status: 'not_installed', path: null, version: null },
    node: { status: 'available', path: '/detected/node', version: 'v22.0.0' },
  };
  selectedNode = { status: 'available', path: '/chosen/node/bin/node', version: 'v24.1.0' };
  check = { status: 'available', current_version: 'codex-cli 1.0.0', latest_version: '1.1.0', method: 'npm' };
  updateStatus = 'updated'; issued = 0;
  call.mockReset(); call.mockImplementation(host); picker.mockReset(); confirmDiscard.mockReset(); guard.mockReset(); setLocaleForTesting('zh-CN');
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); resetLocaleForTesting(); });
async function mount() {
  const view = render(<CliSettings />);
  await waitFor(() => expect(screen.getByRole('button', { name: '自动检测' }).hasAttribute('disabled')).toBe(false));
  return view;
}
async function detect() {
  fireEvent.click(screen.getByRole('button', { name: '自动检测' }));
  await waitFor(() => expect(screen.getByRole('button', { name: '自动检测' }).hasAttribute('disabled')).toBe(false));
}

it('hydrates persisted versions and upgrade hints without probing on entry or reload', async () => {
  persist('codex', local.codex); database.detections.codex!.update = check;
  const view = await mount();
  expect(screen.getByText('codex-cli 1.0.0')).toBeTruthy();
  expect(screen.getByRole('button', { name: '更新 Codex' })).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: '重新读取配置' }));
  await waitFor(() => expect(call.mock.calls.filter(([command]) => command === 'get_cli_preferences')).toHaveLength(2));
  view.unmount(); await mount();
  expect(screen.getByText('codex-cli 1.0.0')).toBeTruthy();
  expect(call.mock.calls.every(([command]) => command === 'get_cli_preferences')).toBe(true);
});

it('keeps empty, legacy configured and detected paths read-only at all times', async () => {
  await mount();
  for (const tool of ['Codex', 'Claude', 'Gemini', 'Node.js']) expect((screen.getByRole('textbox', { name: tool }) as HTMLInputElement).readOnly).toBe(true);
  const codex = screen.getByRole('textbox', { name: 'Codex' }) as HTMLInputElement;
  fireEvent.change(codex, { target: { value: '/typed/not-allowed' } });
  fireEvent.click(screen.getByRole('button', { name: '重新读取配置' }));
  await waitFor(() => expect(codex.value).toBe('/old/codex'));
  expect(database.paths.codex).toBe('/old/codex');
  expect(screen.getByRole('button', { name: '保存' }).hasAttribute('disabled')).toBe(true);
  await detect();
  expect(codex.readOnly).toBe(true);
  expect(codex.value).toBe('/detected/codex');
});

it('persists manual detection without a Save step and restores labels on re-entry', async () => {
  const view = await mount();
  expect(screen.queryByText('codex-cli 1.0.0')).toBeNull();
  await detect();
  expect(screen.getByText('codex-cli 1.0.0')).toBeTruthy();
  expect(screen.getByRole('button', { name: '更新 Codex' })).toBeTruthy();
  expect(screen.getByRole('button', { name: '保存' }).hasAttribute('disabled')).toBe(true);
  expect(call).toHaveBeenCalledWith('probe_cli', { tool: 'codex', path: null });
  view.unmount(); call.mockClear(); await mount();
  expect(screen.getByText('codex-cli 1.0.0')).toBeTruthy();
  expect((screen.getByRole('textbox', { name: 'Node.js' }) as HTMLInputElement).value).toBe('/detected/node');
  expect(call.mock.calls.every(([command]) => command === 'get_cli_preferences')).toBe(true);
});

it('publishes local results and keeps navigation, focus and Node selection available during a slow update check', async () => {
  const slow = Promise.withResolvers<unknown>();
  call.mockImplementation((command, args) => command === 'check_cli_update' ? slow.promise : host(command, args));
  const view = await mount();
  fireEvent.click(screen.getByRole('button', { name: '自动检测' }));
  expect(await screen.findByText('codex-cli 1.0.0')).toBeTruthy();
  await waitFor(() => expect(screen.getByRole('button', { name: '选择 Node.js 安装文件夹' }).hasAttribute('disabled')).toBe(false));
  const codex = screen.getByRole('textbox', { name: 'Codex' }) as HTMLInputElement;
  codex.focus(); expect(document.activeElement).toBe(codex);
  expect(codex.disabled).toBe(false);
  expect(screen.getByRole('button', { name: '重新读取配置' }).hasAttribute('disabled')).toBe(false);
  expect(await guard.mock.lastCall![0]()).toBe(true);
  picker.mockResolvedValueOnce('/chosen/node');
  fireEvent.click(screen.getByRole('button', { name: '选择 Node.js 安装文件夹' }));
  expect(await screen.findByText('v24.1.0')).toBeTruthy();
  expect(screen.getByRole('button', { name: '保存' }).hasAttribute('disabled')).toBe(false);
  view.unmount();
  await act(async () => { slow.resolve(ok({ ...check, ticket: 'late-ticket' })); });
});

it('lets reload supersede an in-flight detection and ignores its late result', async () => {
  const slow = Promise.withResolvers<unknown>();
  call.mockImplementation((command, args) => command === 'probe_cli' ? slow.promise : host(command, args));
  await mount(); fireEvent.click(screen.getByRole('button', { name: '自动检测' }));
  fireEvent.click(screen.getByRole('button', { name: '重新读取配置' }));
  await waitFor(() => expect(screen.getByRole('button', { name: '自动检测' }).hasAttribute('disabled')).toBe(false));
  await act(async () => { slow.resolve(ok({ status: 'available', path: '/late/cli', version: '99.0.0' })); });
  expect(screen.queryByText('99.0.0')).toBeNull();
  expect((screen.getByRole('textbox', { name: 'Codex' }) as HTMLInputElement).value).toBe('/old/codex');
});

it('preserves the newer persisted read when StrictMode replays hydration', async () => {
  const old = Promise.withResolvers<unknown>();
  call.mockReturnValueOnce(old.promise);
  render(<StrictMode><CliSettings /></StrictMode>);
  await waitFor(() => expect((screen.getByRole('textbox', { name: 'Codex' }) as HTMLInputElement).disabled).toBe(false));
  await act(async () => { old.resolve(ok({ ...database, revision: 1, paths: { ...database.paths, codex: '/stale/codex' } })); });
  expect((screen.getByRole('textbox', { name: 'Codex' }) as HTMLInputElement).value).toBe('/old/codex');
});

it('removes an obsolete persisted upgrade hint and version when manual detection finds the CLI gone', async () => {
  persist('codex', local.codex); database.detections.codex!.update = check;
  await mount();
  local.codex = { status: 'not_installed', path: null, version: null };
  await detect();
  expect(screen.queryByText('codex-cli 1.0.0')).toBeNull();
  expect(screen.queryByRole('button', { name: '更新 Codex' })).toBeNull();
  expect((screen.getByRole('textbox', { name: 'Codex' }) as HTMLInputElement).value).toBe('');
});

it.each(['zh-CN', 'en-US'] as const)('shows no up-to-date notification or upgrade action in %s', async locale => {
  check.status = 'up_to_date'; persist('codex', local.codex); database.detections.codex!.update = check;
  setLocaleForTesting(locale);
  render(<CliSettings />);
  expect(await screen.findByText('codex-cli 1.0.0')).toBeTruthy();
  expect(screen.queryByRole('button', { name: /^(更新 Codex|Update Codex)$/ })).toBeNull();
  expect(document.querySelector('.cli-tool-codex .cli-tool-feedback')).toBeNull();
});

it('isolates probe failures to their row without exposing host errors or losing other detected tools', async () => {
  call.mockImplementation((command, args) => command === 'probe_cli' && (args as { tool: Tool }).tool === 'gemini' ? Promise.reject(Error('sensitive host output')) : host(command, args));
  await mount(); await detect();
  expect(screen.getByRole('textbox', { name: 'Gemini' }).getAttribute('aria-invalid')).toBe('true');
  expect(screen.getByRole('textbox', { name: 'Codex' }).getAttribute('aria-invalid')).toBeNull();
  expect(screen.getByText('codex-cli 1.0.0')).toBeTruthy();
  expect(screen.queryByText('sensitive host output')).toBeNull();
});

it.each(['check_failed', 'transport-error'])('keeps %s separate from the valid local version', async failure => {
  if (failure === 'transport-error') call.mockImplementation((command, args) => command === 'check_cli_update' ? Promise.reject(Error('sensitive registry details')) : host(command, args));
  else check.status = 'check_failed';
  await mount(); await detect();
  expect(screen.getByText('codex-cli 1.0.0')).toBeTruthy();
  expect(screen.queryByRole('button', { name: '更新 Codex' })).toBeNull();
  expect(screen.getByRole('textbox', { name: 'Codex' }).getAttribute('aria-invalid')).toBeNull();
  expect(document.querySelector('.cli-tool-codex .cli-tool-feedback')).not.toBeNull();
});

it('changes Node only through a verified folder, then saves its path/version together', async () => {
  await mount(); await detect();
  const codex = database.paths.codex;
  picker.mockResolvedValueOnce('/chosen/node');
  fireEvent.click(screen.getByRole('button', { name: '选择 Node.js 安装文件夹' }));
  expect(await screen.findByText('v24.1.0')).toBeTruthy();
  expect(database.paths.node).toBe('/detected/node');
  expect((screen.getByRole('textbox', { name: 'Node.js' }) as HTMLInputElement).readOnly).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: '保存' }));
  await waitFor(() => expect(database.paths.node).toBe('/chosen/node/bin/node'));
  await waitFor(() => expect(screen.getByRole('button', { name: '保存' }).hasAttribute('disabled')).toBe(true));
  expect(database.detections.node?.version).toBe('v24.1.0');
  expect(database.paths.codex).toBe(codex);
});

it.each(['cancel', 'invalid', 'picker-error', 'probe-error'])('keeps a persisted Node path and version after %s', async failure => {
  persist('node', local.node); await mount();
  if (failure === 'picker-error') picker.mockRejectedValueOnce(Error('sensitive picker'));
  else picker.mockResolvedValueOnce(failure === 'cancel' ? null : '/invalid/node');
  if (failure === 'invalid') selectedNode = { status: 'not_installed', path: null, version: null };
  if (failure === 'probe-error') call.mockImplementation((command, args) => command === 'probe_node_directory' ? Promise.reject(Error('sensitive output')) : host(command, args));
  fireEvent.click(screen.getByRole('button', { name: '选择 Node.js 安装文件夹' }));
  await waitFor(() => expect(screen.getByRole('button', { name: '选择 Node.js 安装文件夹' }).hasAttribute('disabled')).toBe(false));
  expect((screen.getByRole('textbox', { name: 'Node.js' }) as HTMLInputElement).value).toBe('/detected/node');
  expect(screen.getByText('v22.0.0')).toBeTruthy();
  expect(screen.getByRole('button', { name: '保存' }).hasAttribute('disabled')).toBe(true);
  expect(screen.queryByText(/sensitive/)).toBeNull();
});

it('preserves a conflicted Node draft until an explicit readback is accepted', async () => {
  await mount(); picker.mockResolvedValueOnce('/chosen/node');
  fireEvent.click(screen.getByRole('button', { name: '选择 Node.js 安装文件夹' }));
  expect(await screen.findByText('v24.1.0')).toBeTruthy();
  database.revision += 1;
  fireEvent.click(screen.getByRole('button', { name: '保存' }));
  await screen.findByRole('alert');
  expect((screen.getByRole('textbox', { name: 'Node.js' }) as HTMLInputElement).value).toBe('/chosen/node/bin/node');
  expect(screen.getByRole('button', { name: '保存' }).hasAttribute('disabled')).toBe(true);
  confirmDiscard.mockResolvedValueOnce(false);
  fireEvent.click(screen.getByRole('button', { name: '重新读取配置' }));
  await waitFor(() => expect(confirmDiscard).toHaveBeenCalled());
  expect(screen.getByText('v24.1.0')).toBeTruthy();
  confirmDiscard.mockResolvedValueOnce(true);
  fireEvent.click(screen.getByRole('button', { name: '重新读取配置' }));
  await waitFor(() => expect(screen.queryByText('v24.1.0')).toBeNull());
});

it('does not acknowledge a Node save until readback succeeds', async () => {
  await mount(); picker.mockResolvedValueOnce('/chosen/node');
  fireEvent.click(screen.getByRole('button', { name: '选择 Node.js 安装文件夹' }));
  await screen.findByText('v24.1.0');
  call.mockImplementation((command, args) => command === 'get_cli_preferences' ? Promise.reject(Error('sensitive readback')) : host(command, args));
  fireEvent.click(screen.getByRole('button', { name: '保存' }));
  await screen.findByRole('alert');
  expect(screen.getByRole('button', { name: '保存' }).hasAttribute('disabled')).toBe(true);
  expect((screen.getByRole('textbox', { name: 'Node.js' }) as HTMLInputElement).value).toBe('/chosen/node/bin/node');
  expect(screen.queryByText(/sensitive/)).toBeNull();
});

it('revalidates a persisted update hint before confirmation and consumes the fresh ticket only once', async () => {
  persist('codex', local.codex); database.detections.codex!.update = check;
  await mount();
  const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(() => ({ id: 'confirmation', close: vi.fn() }));
  const button = screen.getByRole('button', { name: '更新 Codex' });
  fireEvent.click(button); fireEvent.click(button);
  await waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
  expect(call.mock.calls.filter(([command]) => command === 'check_cli_update')).toHaveLength(1);
  expect(call.mock.calls.some(([command]) => command === 'update_cli')).toBe(false);
  await act(async () => { confirm.mock.calls[0][0].onCancel?.(); });
  await act(async () => { await confirm.mock.calls[0][0].onConfirm?.(); });
  expect(call.mock.calls.some(([command]) => command === 'update_cli')).toBe(false);
  fireEvent.click(button); await waitFor(() => expect(confirm).toHaveBeenCalledTimes(2));
  await act(async () => { await confirm.mock.calls[1][0].onConfirm?.(); await confirm.mock.calls[1][0].onConfirm?.(); });
  expect(call.mock.calls.filter(([command]) => command === 'update_cli')).toHaveLength(1);
  expect(call).toHaveBeenCalledWith('update_cli', { ticket: 'ticket-2' });
  expect(screen.getByText('codex-cli 1.1.0')).toBeTruthy();
  expect(screen.queryByRole('button', { name: '更新 Codex' })).toBeNull();
});

it('silently removes a cached upgrade offer when explicit revalidation finds it already current', async () => {
  persist('codex', local.codex); database.detections.codex!.update = { ...check };
  await mount(); check.status = 'up_to_date';
  const confirm = vi.spyOn(Modal, 'confirm');
  fireEvent.click(screen.getByRole('button', { name: '更新 Codex' }));
  await waitFor(() => expect(screen.queryByRole('button', { name: '更新 Codex' })).toBeNull());
  expect(confirm).not.toHaveBeenCalled();
  expect(document.querySelector('.cli-tool-codex .cli-tool-feedback')).toBeNull();
});

it.each(['unknown', 'stale', 'failed', 'busy', 'transport-error'])('does not replay an update after %s', async status => {
  persist('codex', local.codex); database.detections.codex!.update = check;
  await mount(); updateStatus = status;
  if (status === 'transport-error') call.mockImplementation((command, args) => command === 'update_cli' ? Promise.reject(Error('sensitive install output')) : host(command, args));
  const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(() => ({ id: 'confirmation', close: vi.fn() }));
  fireEvent.click(screen.getByRole('button', { name: '更新 Codex' }));
  await waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
  await act(async () => { await confirm.mock.calls[0][0].onConfirm?.(); await confirm.mock.calls[0][0].onConfirm?.(); });
  expect(call.mock.calls.filter(([command]) => command === 'update_cli')).toHaveLength(1);
  expect(screen.queryByRole('button', { name: '更新 Codex' })).toBeNull();
  expect(screen.getByText('codex-cli 1.0.0')).toBeTruthy();
  expect(screen.queryByText(/sensitive/)).toBeNull();
});

it('rejects stale directory and confirmation callbacks after leaving the page', async () => {
  persist('codex', local.codex); database.detections.codex!.update = check;
  const view = await mount();
  const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(() => ({ id: 'confirmation', close: vi.fn() }));
  fireEvent.click(screen.getByRole('button', { name: '更新 Codex' }));
  await waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
  view.unmount();
  await act(async () => { await confirm.mock.calls[0][0].onConfirm?.(); });
  expect(call.mock.calls.some(([command]) => command === 'update_cli')).toBe(false);
  const next = await mount(); const directory = Promise.withResolvers<unknown>(); picker.mockReturnValueOnce(directory.promise);
  fireEvent.click(screen.getByRole('button', { name: '选择 Node.js 安装文件夹' })); next.unmount();
  await act(async () => { directory.resolve('/late/node'); });
  expect(call.mock.calls.some(([command]) => command === 'probe_node_directory')).toBe(false);
});
