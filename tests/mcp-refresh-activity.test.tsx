import { StrictMode } from 'react';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import McpManagement from '../src/pages/McpManagement';
import McpConnectionNotice from '../src/components/McpConnectionNotice';
import { useMcpStore, type McpServer, type McpTool, type McpUsageLog } from '../src/stores/mcpStore';
import { useDevelopment } from '../src/stores/developmentStore';
import { setLocaleForTesting } from '../src/i18n/runtime';
import { SurfaceScope } from '../src/workspace/SurfaceContext';
import { createWorkspaceController } from '../src/workspace/controller';
import { t } from '../src/i18n';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const mocked = vi.mocked(invoke);
const initial = useMcpStore.getState();
const servers: McpServer[] = ['a', 'b'].map((id, index) => ({
  id, user_id: 'owner', name: index ? 'Beta' : 'Alpha', transport: index ? 'http' : 'stdio',
  enabled: true, auto_connect: false, status: 'ready', reconnect_attempts: 0,
  reconnect_exhausted: false, created_at: 1, updated_at: 1,
}));
const calls = (command: string) => mocked.mock.calls.filter(([name]) => name === command).length;
async function flush() { await act(async () => { await vi.advanceTimersByTimeAsync(0); }); }
function mount(active = true, globalNotice = false) {
  const controller = createWorkspaceController();
  const tree = (next: boolean, hidden = false) => <MemoryRouter>
    <div hidden={hidden}><SurfaceScope id="mcp" active={next} controller={controller}><McpManagement /></SurfaceScope></div>
    {globalNotice && <McpConnectionNotice />}
  </MemoryRouter>;
  const view = render(tree(active));
  return { ...view, activity: (next: boolean, hidden = false) => view.rerender(tree(next, hidden)) };
}
beforeEach(() => {
  vi.useFakeTimers();
  setLocaleForTesting('en-US');
  useDevelopment.getState().reset();
  useMcpStore.setState(initial, true);
  mocked.mockReset();
  mocked.mockImplementation(async (command, args) => {
    const id = args && typeof args === 'object' && 'serverId' in args ? String(args.serverId) : 'a';
    if (command === 'list_mcp_servers') return servers;
    if (command === 'list_mcp_tools' || command === 'refresh_mcp_tools') return [{ server_id: id, tool_name: `${id}-tool`, description: `${id} capability`, input_schema: {}, schema_hash: 'schema', discovered_at: 1 } satisfies McpTool];
    if (command === 'list_mcp_usage_logs' || command === 'list_mcp_connection_logs') return [];
    if (command === 'toggle_mcp_server') return { ...servers[0], enabled: false };
    throw new Error(`Unexpected command: ${command}`);
  });
});
afterEach(() => { cleanup(); useDevelopment.getState().reset(); vi.restoreAllMocks(); vi.useRealTimers(); });

it('pauses page reads while hidden or covered and resumes all selected snapshots without losing local filters', async () => {
  const view = mount();
  await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' }));
  fireEvent.change(screen.getByLabelText('Search servers'), { target: { value: 'Alpha' } });
  await flush();
  const counts = ['list_mcp_servers', 'list_mcp_tools', 'list_mcp_usage_logs', 'list_mcp_connection_logs'].map(calls);
  view.activity(false, true);
  await act(async () => { await vi.advanceTimersByTimeAsync(30000); });
  expect(['list_mcp_servers', 'list_mcp_tools', 'list_mcp_usage_logs', 'list_mcp_connection_logs'].map(calls)).toEqual(counts);
  view.activity(true);
  await flush();
  expect(['list_mcp_servers', 'list_mcp_tools', 'list_mcp_usage_logs', 'list_mcp_connection_logs'].map(calls)).toEqual(counts.map(n => n + 1));
  expect((screen.getByLabelText('Search servers') as HTMLInputElement).value).toBe('Alpha');
  view.activity(false); // Covered but still mounted and visible.
  const covered = calls('list_mcp_connection_logs');
  await act(async () => { await vi.advanceTimersByTimeAsync(30000); });
  expect(calls('list_mcp_connection_logs')).toBe(covered);
});

it('keeps the shell recovery consumer polling while the inactive page contributes no connection reads', async () => {
  const view = mount(true, true);
  await flush();
  expect(calls('list_mcp_servers')).toBe(1);
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' }));
  await flush();
  view.activity(false);
  await act(async () => { await vi.advanceTimersByTimeAsync(30000); });
  expect(calls('list_mcp_servers')).toBe(4);
  expect(calls('list_mcp_connection_logs')).toBe(1);
});

it('coalesces slow selected reads across quick selection changes, resume and manual refresh', async () => {
  const slow = Promise.withResolvers<McpUsageLog[]>();
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'list_mcp_usage_logs' && args && typeof args === 'object' && 'serverId' in args && args.serverId === 'a' ? slow.promise : handler(command, args));
  const view = mount();
  await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' }));
  await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Refresh usage' }));
  fireEvent.click(screen.getByRole('button', { name: 'Select Beta' }));
  await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' }));
  view.activity(false);
  view.activity(true);
  await flush();
  expect(calls('list_mcp_usage_logs')).toBe(2); // One per target, not one per consumer.
  view.activity(false);
  await act(async () => { slow.resolve([{ id: 1, server_id: 'a', tool_name: 'latest-a', outcome: 'success', created_at: 1 }]); await slow.promise; });
  expect(useMcpStore.getState().logsByServer.a[0].tool_name).toBe('latest-a');
  expect(useMcpStore.getState().logsByServer.b).toEqual([]);
});

it('retains prior content on failed resume snapshots', async () => {
  const view = mount();
  await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' }));
  await flush();
  view.activity(false);
  mocked.mockRejectedValue('snapshot unavailable');
  view.activity(true);
  await flush();
  expect(screen.getByRole('button', { name: 'Select Alpha' }).getAttribute('aria-pressed')).toBe('true');
  expect(screen.getByText('a capability')).toBeTruthy();
  expect(screen.getAllByRole('alert').every(alert => alert.textContent?.includes('snapshot unavailable'))).toBe(true);
  expect(screen.getAllByRole('alert')).toHaveLength(4);
});

it('does not let a pre-mutation server snapshot overwrite the mutation result', async () => {
  useMcpStore.setState({ servers });
  const old = Promise.withResolvers<McpServer[]>();
  mocked.mockImplementation(async command => command === 'list_mcp_servers' ? old.promise : { ...servers[0], enabled: false });
  const read = useMcpStore.getState().fetch();
  await useMcpStore.getState().toggle('a', false);
  old.resolve(servers);
  await read;
  expect(useMcpStore.getState().servers[0].enabled).toBe(false);
});

it('queues a post-mutation usage read behind the earlier flight instead of swallowing it', async () => {
  const old = Promise.withResolvers<McpUsageLog[]>();
  const fresh: McpUsageLog[] = [{ id: 2, server_id: 'a', tool_name: 'new-call', outcome: 'success', created_at: 2 }];
  let reads = 0;
  mocked.mockImplementation(async command => {
    if (command === 'list_mcp_usage_logs') return ++reads === 1 ? old.promise : fresh;
    if (command === 'call_mcp_tool') return { outcome: 'success' };
    throw new Error(command);
  });
  const first = useMcpStore.getState().fetchLogs('a');
  await useMcpStore.getState().call({ server_id: 'a', tool_name: 'a-tool', arguments: {} });
  const afterCall = useMcpStore.getState().fetchLogs('a');
  expect(reads).toBe(1);
  old.resolve([]);
  await Promise.all([first, afterCall]);
  expect(reads).toBe(2);
  expect(useMcpStore.getState().logsByServer.a).toEqual(fresh);
});

it('drops old-account snapshots, failures and pending-action cleanup while allowing the new account to read', async () => {
  const old = Promise.withResolvers<McpServer[]>();
  const action = Promise.withResolvers<McpServer>();
  mocked.mockImplementation(async command => command === 'list_mcp_servers' ? old.promise : action.promise);
  const read = useMcpStore.getState().fetch();
  const mutation = useMcpStore.getState().toggle('a', false).catch(() => {});
  useDevelopment.getState().reset();
  expect(useMcpStore.getState().servers).toEqual([]);
  expect(useMcpStore.getState().pending).toEqual([]);
  const next = [{ ...servers[1], user_id: 'next-owner' }];
  mocked.mockResolvedValue(next);
  await useMcpStore.getState().fetch();
  old.reject('old account failed');
  action.resolve({ ...servers[0], enabled: false });
  await Promise.all([read, mutation]);
  expect(useMcpStore.getState().servers).toEqual(next);
});

it('clears a credential draft on inactivity while preserving the mounted server selection', async () => {
  const view = mount();
  await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Select Beta' }));
  await flush();
  fireEvent.change(screen.getByLabelText('Bearer token'), { target: { value: 'private-draft' } });
  view.activity(false);
  expect((screen.getByLabelText('Bearer token') as HTMLInputElement).value).toBe('');
  view.activity(true);
  await flush();
  expect(screen.getByLabelText('Bearer token')).toBeTruthy();
});

it('takes exactly one fresh snapshot after an in-flight resume and cancels queued presentation reads on unmount', async () => {
  const old = Promise.withResolvers<McpServer[]>();
  let lists = 0;
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => {
    if (command === 'list_mcp_servers') return ++lists === 1 ? old.promise : Promise.resolve([{ ...servers[0], name: 'Fresh Alpha' }]);
    return handler(command, args);
  });
  const view = mount();
  view.activity(false);
  view.activity(true);
  view.activity(false);
  view.activity(true);
  expect(lists).toBe(1);
  await act(async () => { old.resolve(servers); await old.promise; });
  await flush();
  expect(lists).toBe(2);
  expect(screen.getByText('Fresh Alpha')).toBeTruthy();

  const pending = Promise.withResolvers<McpServer[]>();
  mocked.mockImplementation(async () => pending.promise);
  fireEvent.click(screen.getByRole('button', { name: 'Refresh status' }));
  view.activity(false);
  view.activity(true);
  view.unmount();
  const count = calls('list_mcp_servers');
  pending.resolve(servers);
  await flush();
  expect(calls('list_mcp_servers')).toBe(count);
});

it('refreshes the selected target after a slow resume without landing another target in its detail', async () => {
  const old = Promise.withResolvers<McpUsageLog[]>();
  let reads = 0;
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => {
    if (command === 'list_mcp_usage_logs' && args && typeof args === 'object' && 'serverId' in args && args.serverId === 'a') {
      return ++reads === 1 ? old.promise : Promise.resolve([{ id: 2, server_id: 'a', tool_name: 'fresh-usage', outcome: 'success', created_at: 2 }]);
    }
    return handler(command, args);
  });
  const view = mount();
  await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' }));
  await flush();
  view.activity(false);
  view.activity(true);
  view.activity(false);
  view.activity(true);
  expect(reads).toBe(1);
  old.resolve([]);
  await flush();
  expect(reads).toBe(2);
  expect(screen.getByText('fresh-usage')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Select Beta' }));
  await flush();
  expect(screen.queryByText('fresh-usage')).toBeNull();
  expect(screen.getByText('b capability')).toBeTruthy();
});

it('lets a hidden tool action finish and keeps its usage refresh rather than cancelling business work', async () => {
  const response = Promise.withResolvers<unknown>();
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'call_mcp_tool' ? response.promise : handler(command, args));
  const view = mount();
  await flush();
  let action!: Promise<unknown>;
  act(() => { action = useMcpStore.getState().call({ server_id: 'a', tool_name: 'a-tool', arguments: {} }); });
  view.activity(false);
  expect(useMcpStore.getState().pending).toContain('a');
  await act(async () => { response.resolve({ outcome: 'success' }); await action; await useMcpStore.getState().fetchLogs('a'); });
  expect(useMcpStore.getState().pending).not.toContain('a');
  expect(calls('call_mcp_tool')).toBe(1);
  expect(calls('cancel_mcp_tool')).toBe(0);
  expect(calls('list_mcp_usage_logs')).toBe(1);
});

it('does not start any presentation request until an initially inactive surface resumes', async () => {
  const view = mount(false);
  await flush();
  await act(async () => { await vi.advanceTimersByTimeAsync(30000); });
  expect(mocked).not.toHaveBeenCalled();
  view.activity(true);
  await flush();
  expect(calls('list_mcp_servers')).toBe(1);
});

it('keeps StrictMode replay ownership live so selected snapshots and resume refreshes still run', async () => {
  const controller = createWorkspaceController();
  const tree = (active: boolean) => <StrictMode><SurfaceScope id="mcp" active={active} controller={controller}><McpManagement /></SurfaceScope></StrictMode>;
  const view = render(tree(true));
  await flush();
  expect(calls('list_mcp_servers')).toBe(1);
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' }));
  await flush();
  expect(screen.getByText('a capability')).toBeTruthy();
  view.rerender(tree(false));
  view.rerender(tree(true));
  await flush();
  expect(calls('list_mcp_usage_logs')).toBe(2);
  expect(calls('list_mcp_connection_logs')).toBe(2);
});

it('does not let an older tool-list read replace a completed discovery mutation', async () => {
  const old = Promise.withResolvers<McpTool[]>();
  const discovered: McpTool[] = [{ server_id: 'a', tool_name: 'new-tool', input_schema: {}, schema_hash: 'new', discovered_at: 2 }];
  mocked.mockImplementation(async command => command === 'list_mcp_tools' ? old.promise : discovered);
  const read = useMcpStore.getState().loadTools('a');
  await useMcpStore.getState().discoverTools('a');
  old.resolve([]);
  await read;
  expect(useMcpStore.getState().tools).toEqual(discovered);
});

it('shows a slow first server read, then a retryable failure, then the successful empty state', async () => {
  const first = Promise.withResolvers<McpServer[]>();
  mocked.mockReturnValue(first.promise);
  mount();
  expect(screen.getByText('Loading servers…')).toBeTruthy();
  expect(screen.queryByText('No MCP servers yet')).toBeNull();
  await act(async () => { first.reject('server list unavailable'); await first.promise.catch(() => {}); });
  expect(screen.getByRole('alert').textContent).toContain('server list unavailable');
  expect(screen.queryByText('No MCP servers yet')).toBeNull();
  mocked.mockResolvedValue([]);
  fireEvent.click(within(screen.getByRole('alert')).getByRole('button', { name: 'Retry' }));
  await flush();
  expect(screen.getByText('No MCP servers yet')).toBeTruthy();
  expect(screen.queryByRole('alert')).toBeNull();
});

it('distinguishes slow selected resources from empty snapshots and retries each failed read in place', async () => {
  const tools = Promise.withResolvers<McpTool[]>();
  const usage = Promise.withResolvers<McpUsageLog[]>();
  const connections = Promise.withResolvers<unknown[]>();
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => {
    if (command === 'list_mcp_tools') return tools.promise;
    if (command === 'list_mcp_usage_logs') return usage.promise;
    if (command === 'list_mcp_connection_logs') return connections.promise;
    return handler(command, args);
  });
  mount(); await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' }));
  expect(screen.getByText('Loading tools…')).toBeTruthy();
  expect(screen.getByText('Loading usage…')).toBeTruthy();
  expect(screen.getByText('Loading connection events…')).toBeTruthy();
  expect(screen.queryByText('No calls recorded for this server.')).toBeNull();
  expect(screen.queryByText('No connection events for this server.')).toBeNull();
  await act(async () => { tools.reject('tools unavailable'); usage.reject('usage unavailable'); connections.reject('connections unavailable'); await Promise.allSettled([tools.promise, usage.promise, connections.promise]); });
  expect(screen.getAllByRole('alert')).toHaveLength(3);
  mocked.mockImplementation((command, args) => ['list_mcp_tools', 'list_mcp_usage_logs', 'list_mcp_connection_logs'].includes(String(command)) ? Promise.resolve([]) : handler(command, args));
  for (const alert of screen.getAllByRole('alert')) fireEvent.click(within(alert).getByRole('button', { name: 'Retry' }));
  await flush();
  expect(screen.queryByRole('alert')).toBeNull();
  expect(screen.getByText('Connect the server to discover tools.')).toBeTruthy();
  expect(screen.getByText('No calls recorded for this server.')).toBeTruthy();
  expect(screen.getByText('No connection events for this server.')).toBeTruthy();
});

it('keeps an old snapshot and its own failure visible through unrelated success and retry recovery', async () => {
  const handler = mocked.getMockImplementation()!;
  let failUsage = false;
  mocked.mockImplementation((command, args) => command === 'list_mcp_usage_logs'
    ? failUsage ? Promise.reject('usage refresh failed') : Promise.resolve([{ id: 1, server_id: 'a', tool_name: 'saved-call', outcome: 'success', created_at: 1 }])
    : handler(command, args));
  mount(); await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' })); await flush();
  failUsage = true;
  fireEvent.click(screen.getByRole('button', { name: 'Refresh usage' })); await flush();
  expect(screen.getByText('saved-call')).toBeTruthy();
  expect(screen.getByRole('alert').textContent).toContain('usage refresh failed');
  fireEvent.click(screen.getByRole('button', { name: 'Refresh status' })); await flush();
  expect(screen.getByRole('alert').textContent).toContain('usage refresh failed');
  const retry = Promise.withResolvers<McpUsageLog[]>();
  mocked.mockImplementation((command, args) => command === 'list_mcp_usage_logs' ? retry.promise : handler(command, args));
  fireEvent.click(within(screen.getByRole('alert')).getByRole('button', { name: 'Retry' }));
  expect(screen.getByText(/Refreshing…/)).toBeTruthy();
  expect(screen.getByText('saved-call')).toBeTruthy();
  expect(screen.getByRole('alert').textContent).toContain('usage refresh failed');
  await act(async () => { retry.resolve([]); await retry.promise; });
  expect(screen.queryByRole('alert')).toBeNull();
  expect(screen.queryByText('saved-call')).toBeNull();
  expect(screen.getByText('No calls recorded for this server.')).toBeTruthy();
});

it('keeps target-specific loading and failures out of another selected server', async () => {
  const slow = Promise.withResolvers<McpUsageLog[]>();
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'list_mcp_usage_logs' && args && typeof args === 'object' && 'serverId' in args && args.serverId === 'a' ? slow.promise : handler(command, args));
  mount(); await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' })); await flush();
  expect(screen.getByText('Loading usage…')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Select Beta' })); await flush();
  await act(async () => { slow.reject('Alpha usage unavailable'); await slow.promise.catch(() => {}); });
  expect(screen.queryByText('Alpha usage unavailable')).toBeNull();
  expect(screen.getByText('No calls recorded for this server.')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' })); await flush();
  expect(screen.getByRole('alert').textContent).toContain('Alpha usage unavailable');
});

it('keeps a slow old-account failure out of the new account visible state', async () => {
  const old = Promise.withResolvers<McpServer[]>();
  mocked.mockReturnValue(old.promise);
  mount();
  mocked.mockResolvedValue([{ ...servers[1], name: 'New account server', user_id: 'next-owner' }]);
  act(() => { useDevelopment.getState().reset(); }); await flush();
  await act(async () => { old.reject('private old-account failure'); await old.promise.catch(() => {}); });
  expect(screen.getByRole('button', { name: 'Select New account server' })).toBeTruthy();
  expect(screen.queryByRole('alert')).toBeNull();
  expect(screen.queryByText('Loading servers…')).toBeNull();
});

it('recovers failed resume reads while retaining the selected server and unsaved configuration', async () => {
  const handler = mocked.getMockImplementation()!;
  let failReads = false;
  mocked.mockImplementation((command, args) => {
    if (command === 'get_mcp_server_config') return Promise.resolve({ name: 'Alpha', transport: 'stdio', stdio: { executable: 'server', args: [] } });
    if (failReads && String(command).startsWith('list_mcp_')) return Promise.reject('resume unavailable');
    return handler(command, args);
  });
  const view = mount(); await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Edit Alpha' })); await flush();
  fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Unsaved Alpha' } });
  const input = screen.getByLabelText('Name');
  view.activity(false); failReads = true; view.activity(true); await flush();
  expect(screen.getAllByRole('alert')).toHaveLength(4);
  expect(screen.getByLabelText('Name')).toBe(input);
  expect((input as HTMLInputElement).value).toBe('Unsaved Alpha');
  view.activity(false); failReads = false; view.activity(true); await flush();
  expect(screen.queryByRole('alert')).toBeNull();
  expect(screen.getByLabelText('Name')).toBe(input);
  expect((input as HTMLInputElement).value).toBe('Unsaved Alpha');
  expect(screen.getByRole('button', { name: 'Select Alpha' }).getAttribute('aria-pressed')).toBe('true');
  expect(screen.getByText('a capability')).toBeTruthy();
});

it('keeps slow discovery feedback with its server while switching targets and hiding the surface', async () => {
  const discovery = Promise.withResolvers<McpTool[]>();
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'refresh_mcp_tools' ? discovery.promise : handler(command, args));
  const view = mount(); await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' })); await flush();
  const refresh = screen.getByRole('button', { name: t('mcp.refreshTools') });
  fireEvent.click(refresh); fireEvent.click(refresh);
  expect(calls('refresh_mcp_tools')).toBe(1);
  expect(screen.getByRole('button', { name: t('mcp.refreshingTools') }).hasAttribute('disabled')).toBe(true);
  expect(screen.getByRole('status', { name: t('mcp.discoveryFeedback') }).textContent).toContain(t('mcp.discoveryPending', { name: 'Alpha' }));
  fireEvent.click(screen.getByRole('button', { name: 'Select Beta' })); await flush();
  expect(screen.queryByRole('status', { name: t('mcp.discoveryFeedback') })).toBeNull();
  view.activity(false, true);
  await act(async () => { discovery.resolve([]); await discovery.promise; });
  view.activity(true); await flush();
  expect(screen.queryByRole('status', { name: t('mcp.discoveryFeedback') })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' })); await flush();
  expect(screen.getByRole('status', { name: t('mcp.discoveryFeedback') }).textContent).toContain(t('mcp.discoverySucceeded', { name: 'Alpha' }));
  expect(calls('refresh_mcp_tools')).toBe(1);
});

it('retains a definite discovery failure and the prior tools without contaminating another server', async () => {
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'refresh_mcp_tools' ? Promise.reject('MCP server is not connected') : handler(command, args));
  mount(); await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' })); await flush();
  fireEvent.click(screen.getByRole('button', { name: t('mcp.refreshTools') })); await flush();
  expect(screen.getByRole('alert', { name: t('mcp.discoveryFeedback') }).textContent).toContain(t('mcp.discoveryFailed', { name: 'Alpha' }));
  expect(screen.getByText('a capability')).toBeTruthy();
  expect(screen.getByRole('button', { name: t('mcp.refreshTools') }).hasAttribute('disabled')).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: 'Select Beta' })); await flush();
  expect(screen.queryByRole('alert')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' })); await flush();
  expect(screen.getByRole('alert', { name: t('mcp.discoveryFeedback') })).toBeTruthy();
});

it('never infers discovery success from a snapshot after an unknown result and only retries the read', async () => {
  const handler = mocked.getMockImplementation()!;
  let failRead = false;
  mocked.mockImplementation((command, args) => {
    if (command === 'refresh_mcp_tools') return Promise.reject('database error: receipt unavailable');
    if (command === 'list_mcp_tools' && failRead) return Promise.reject('snapshot unavailable');
    return handler(command, args);
  });
  const view = mount(); await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' })); await flush();
  fireEvent.click(screen.getByRole('button', { name: t('mcp.refreshTools') })); await flush();
  const feedback = () => screen.getByRole('status', { name: t('mcp.discoveryFeedback') });
  expect(feedback().textContent).toContain(t('mcp.discoveryUnconfirmed', { name: 'Alpha' }));
  expect(screen.getByRole('button', { name: t('mcp.refreshTools') }).hasAttribute('disabled')).toBe(true);
  failRead = true;
  fireEvent.click(within(feedback()).getByRole('button', { name: t('mcp.readToolSnapshot') })); await flush();
  expect(screen.getByRole('alert').textContent).toContain('snapshot unavailable');
  failRead = false;
  fireEvent.click(within(screen.getByRole('alert')).getByRole('button', { name: 'Retry' })); await flush();
  expect(screen.queryByRole('alert')).toBeNull();
  expect(feedback().textContent).toContain(t('mcp.discoveryUnconfirmed', { name: 'Alpha' }));
  fireEvent.click(screen.getByRole('button', { name: 'Select Beta' })); await flush();
  expect(screen.queryByRole('status', { name: t('mcp.discoveryFeedback') })).toBeNull();
  expect(screen.getByText('b capability')).toBeTruthy();
  expect(useMcpStore.getState().discoveryByServer.b).toBeUndefined();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' })); await flush();
  expect(feedback().textContent).toContain(t('mcp.discoveryUnconfirmed', { name: 'Alpha' }));
  view.activity(false); view.activity(true); await flush();
  expect(feedback().textContent).toContain(t('mcp.discoveryUnconfirmed', { name: 'Alpha' }));
  expect(calls('refresh_mcp_tools')).toBe(1);
  await useMcpStore.getState().discoverTools('a').catch(() => {});
  expect(calls('refresh_mcp_tools')).toBe(1);
});

it('keeps confirmed discovery success separate from a failed later read and its retry', async () => {
  const handler = mocked.getMockImplementation()!;
  let failRead = false;
  mocked.mockImplementation((command, args) => command === 'list_mcp_tools' && failRead ? Promise.reject('snapshot unavailable') : handler(command, args));
  const view = mount(); await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' })); await flush();
  fireEvent.click(screen.getByRole('button', { name: t('mcp.refreshTools') })); await flush();
  failRead = true;
  view.activity(false); view.activity(true); await flush();
  expect(screen.getByRole('status', { name: t('mcp.discoveryFeedback') }).textContent).toContain(t('mcp.discoverySucceeded', { name: 'Alpha' }));
  expect(screen.getByRole('alert').textContent).toContain('snapshot unavailable');
  expect(screen.getByText('a capability')).toBeTruthy();
  failRead = false;
  fireEvent.click(within(screen.getByRole('alert')).getByRole('button', { name: 'Retry' })); await flush();
  expect(screen.queryByRole('alert')).toBeNull();
  expect(screen.getByRole('status', { name: t('mcp.discoveryFeedback') }).textContent).toContain(t('mcp.discoverySucceeded', { name: 'Alpha' }));
  expect(calls('refresh_mcp_tools')).toBe(1);
});

it('rejects synchronous discovery and other server actions before dispatching a duplicate', async () => {
  const discovery = Promise.withResolvers<McpTool[]>();
  mocked.mockReturnValue(discovery.promise);
  const first = useMcpStore.getState().discoverTools('a');
  const duplicate = useMcpStore.getState().discoverTools('a').catch(() => {});
  const conflicting = useMcpStore.getState().disconnect('a').catch(() => {});
  expect(calls('refresh_mcp_tools')).toBe(1);
  expect(calls('disconnect_mcp_server')).toBe(0);
  discovery.resolve([]);
  await Promise.all([first, duplicate, conflicting]);
});

it('keeps a late discovery receipt out of a new account with the same server id', async () => {
  const discovery = Promise.withResolvers<McpTool[]>();
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'refresh_mcp_tools' ? discovery.promise : handler(command, args));
  mount(); await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' })); await flush();
  fireEvent.click(screen.getByRole('button', { name: t('mcp.refreshTools') }));
  act(() => { useDevelopment.getState().reset(); }); await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' })); await flush();
  await act(async () => { discovery.resolve([]); await discovery.promise; });
  expect(screen.queryByRole('status', { name: t('mcp.discoveryFeedback') })).toBeNull();
  expect(screen.queryByRole('alert')).toBeNull();
  expect(screen.getByRole('button', { name: t('mcp.refreshTools') }).hasAttribute('disabled')).toBe(false);
});

it.each([
  ['MCP call timed out', 'failed'],
  ['MCP call was cancelled', 'failed'],
  ['MCP protocol error: invalid tool schema', 'failed'],
  ['MCP server returned an error: unavailable', 'failed'],
  ['transport error: connection lost', 'failed'],
  ['database error: final snapshot read failed', 'unconfirmed'],
  ['serialization error: final snapshot decoding failed', 'unconfirmed'],
  ['MCP server not found', 'unconfirmed'],
  ['IPC response lost', 'unconfirmed'],
] as const)('preserves the acknowledgement boundary for %s', async (error, outcome) => {
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'refresh_mcp_tools' ? Promise.reject(error) : handler(command, args));
  mount(); await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' })); await flush();
  fireEvent.click(screen.getByRole('button', { name: t('mcp.refreshTools') })); await flush();
  if (outcome === 'failed') {
    expect(screen.getByRole('alert', { name: t('mcp.discoveryFeedback') }).textContent).toContain(t('mcp.discoveryFailed', { name: 'Alpha' }));
    expect(screen.getByRole('button', { name: t('mcp.refreshTools') }).hasAttribute('disabled')).toBe(false);
  } else {
    expect(screen.getByRole('status', { name: t('mcp.discoveryFeedback') }).textContent).toContain(t('mcp.discoveryUnconfirmed', { name: 'Alpha' }));
    expect(screen.getByRole('button', { name: t('mcp.refreshTools') }).hasAttribute('disabled')).toBe(true);
    expect(screen.queryByRole('alert')).toBeNull();
  }
  expect(screen.getByText('a capability')).toBeTruthy();
  expect(calls('refresh_mcp_tools')).toBe(1);
});

it('retains an unconfirmed discovery receipt after unmount and reopening, including successful configuration and tool reads', async () => {
  const discovery = Promise.withResolvers<McpTool[]>();
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => {
    if (command === 'refresh_mcp_tools') return discovery.promise;
    if (command === 'get_mcp_server_config') return Promise.resolve({ name: 'Alpha', transport: 'stdio', stdio: { executable: 'server', args: [] } });
    return handler(command, args);
  });
  const view = mount(); await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Select Alpha' })); await flush();
  fireEvent.click(screen.getByRole('button', { name: t('mcp.refreshTools') }));
  view.unmount();
  expect(useMcpStore.getState().discoveryByServer.a.status).toBe('pending');
  await act(async () => { discovery.reject('IPC response lost'); await discovery.promise.catch(() => {}); });
  await flush();
  expect(useMcpStore.getState().discoveryByServer.a.status).toBe('unconfirmed');
  expect(useMcpStore.getState().pending).toEqual([]);
  mount(); await flush();
  fireEvent.click(screen.getByRole('button', { name: 'Edit Alpha' })); await flush();
  expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Alpha');
  fireEvent.click(screen.getByRole('button', { name: 'Cancel' })); await flush();
  fireEvent.click(screen.getByRole('button', { name: t('mcp.refreshStatus') })); await flush();
  fireEvent.click(screen.getByRole('button', { name: t('mcp.readToolSnapshot') })); await flush();
  expect(screen.getByText('a capability')).toBeTruthy();
  expect(useMcpStore.getState().discoveryByServer.a.status).toBe('unconfirmed');
  expect(screen.getByRole('button', { name: t('mcp.refreshTools') }).hasAttribute('disabled')).toBe(true);
  await useMcpStore.getState().discoverTools('a').catch(() => {});
  expect(calls('get_mcp_server_config')).toBe(1);
  expect(calls('refresh_mcp_tools')).toBe(1);
});

it.each(['success', 'failure'] as const)('does not let a late old-account discovery %s settle a new operation for the same server', async outcome => {
  const old = Promise.withResolvers<McpTool[]>();
  const next = Promise.withResolvers<McpTool[]>();
  mocked.mockReturnValueOnce(old.promise).mockReturnValueOnce(next.promise);
  const previous = useMcpStore.getState().discoverTools('a').catch(() => {});
  const oldReceipt = useMcpStore.getState().discoveryByServer.a;
  useDevelopment.getState().reset();
  const current = useMcpStore.getState().discoverTools('a');
  const nextReceipt = useMcpStore.getState().discoveryByServer.a;
  expect(nextReceipt.operationId).not.toBe(oldReceipt.operationId);
  expect(nextReceipt.accountEpoch).not.toBe(oldReceipt.accountEpoch);
  if (outcome === 'success') old.resolve([]); else old.reject('old account receipt unavailable');
  await previous;
  expect(useMcpStore.getState().discoveryByServer.a).toEqual(nextReceipt);
  expect(useMcpStore.getState().pending).toEqual(['a']);
  expect(useMcpStore.getState().tools).toEqual([]);
  next.resolve([{ server_id: 'a', tool_name: 'next-owner-tool', input_schema: {}, schema_hash: 'next', discovered_at: 2 }]);
  await current;
  expect(useMcpStore.getState().discoveryByServer.a.status).toBe('succeeded');
  expect(useMcpStore.getState().pending).toEqual([]);
  expect(useMcpStore.getState().tools[0].tool_name).toBe('next-owner-tool');
});

it.each(['new', 'editing'] as const)('keeps a newer selected server when a hidden %s save completes', async mode => {
  const saved = Promise.withResolvers<McpServer>();
  const savedServer = { ...servers[0], id: mode === 'new' ? 'created' : 'a', name: 'Saved Alpha' };
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => {
    if (command === 'get_mcp_server_config') return Promise.resolve({ name: 'Alpha', transport: 'stdio', stdio: { executable: 'server', args: [] } });
    if (command === 'create_mcp_server' || command === 'update_mcp_server') return saved.promise;
    return handler(command, args);
  });
  const view = mount(); await flush();
  if (mode === 'editing') {
    fireEvent.click(screen.getByRole('button', { name: 'Edit Alpha' })); await flush();
  } else {
    fireEvent.click(screen.getByRole('button', { name: t('mcp.addServer') }));
    fireEvent.change(screen.getByLabelText('Executable'), { target: { value: 'server' } });
  }
  fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Saved Alpha' } });
  fireEvent.click(screen.getByRole('button', { name: t(mode === 'editing' ? 'mcp.saveChanges' : 'mcp.saveServer') }));
  const command = mode === 'editing' ? 'update_mcp_server' : 'create_mcp_server';
  expect(calls(command)).toBe(1);
  fireEvent.click(screen.getByRole('button', { name: 'Select Beta' })); await flush();
  view.activity(false, true);
  await act(async () => { saved.resolve(savedServer); await saved.promise; });
  expect(useMcpStore.getState().servers.find(server => server.id === savedServer.id)?.name).toBe('Saved Alpha');
  view.activity(true); await flush();
  expect(screen.getByRole('button', { name: 'Select Beta' }).getAttribute('aria-pressed')).toBe('true');
  expect(screen.getByText('b capability')).toBeTruthy();
  expect(screen.queryByLabelText('Name')).toBeNull();
  expect(calls(command)).toBe(1);
});
