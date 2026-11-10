import { cleanup, fireEvent, render, screen, act } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import NativeSessionPanel from '../src/components/nativeExtensions/NativeSessionPanel';
import { useDevelopment } from '../src/stores/developmentStore';
import { useAgentRunner } from '../src/stores/agentRunnerStore';
import type { NativeObservedSkill } from '../src/types/nativeExtensions';
import { CapabilitySnapshotView } from '../src/components/development/CapabilitySnapshotView';
import { useNativeExtensions } from '../src/stores/nativeExtensions';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ t: (key: string) => key, useLocale: () => ({ locale: 'en-US', t: (key: string) => key }) }));
const caps = { observe_skills: true, reload_skills: true, reload_plugins: false, native_auth_guidance: null };
const handle = { session_id: 'native-session', generation: 'generation-a', project_id: 'project-a', engine: 'codex', capabilities: caps };
const observedSkill: NativeObservedSkill = { name: 'observed-skill', enabled: true, classification: 'skill', builtin: null, path: '/native/plugin/skills/observed-skill/SKILL.md', scope: 'repo', plugin_id: 'fixture@marketplace' };
beforeEach(() => {
  invoke.mockReset();
  useAgentRunner.setState({ session: null, selected: null, error: null, live: {} });
  useDevelopment.setState(state => ({ epoch: state.epoch + 1, loaded: true, projects: [{ id: 'project-a', owner: 'alice', name: 'Project A', path: '/tmp/project-a', directory_identity: 'a', archived: false, created_at: 1, last_opened_at: null }] }));
  invoke.mockImplementation(async (command: string) => command === 'list_native_sessions' ? [handle] : Promise.reject('connection_lost'));
});
afterEach(cleanup);
async function selectSession() {
  fireEvent.click(screen.getByRole('combobox', { name: 'nativeExtensions.chooseSession' }));
  fireEvent.click(await screen.findByRole('option', { name: 'Codex · native-session' }));
}
describe('native session authority boundaries', () => {
  it('keeps an unknown mutation blocked after the panel remounts', async () => {
    const view = render(<NativeSessionPanel target="Codex" domain="skills" />);
    await act(async () => {}); await selectSession();
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.reloadSkills' }));
    expect((await screen.findByRole('status')).textContent).toBe('nativeExtensions.action.unknown');
    expect(screen.getByRole('button', { name: 'nativeExtensions.reloadSkills' }).hasAttribute('disabled')).toBe(true);
    view.unmount(); render(<NativeSessionPanel target="Codex" domain="skills" />);
    await act(async () => {}); await selectSession();
    expect(screen.getByRole('button', { name: 'nativeExtensions.reloadSkills' }).hasAttribute('disabled')).toBe(true);
    expect(invoke.mock.calls.filter(call => call[0] === 'native_session_action')).toHaveLength(1);
  });
  it('unblocks a pre-dispatch busy refusal without inspecting or replaying the rejected request', async () => {
    const requests: string[] = [];
    invoke.mockImplementation(async (command: string, args: { input: { request_id: string; session_id: string; generation: string } }) => {
      if (command === 'list_native_sessions') return [handle];
      if (command === 'native_session_action') {
        requests.push(args.input.request_id);
        if (requests.length === 1) throw 'session_busy';
        return { ...args.input, state: 'applied' };
      }
      throw 'unexpected command';
    });
    render(<NativeSessionPanel target="Codex" domain="skills" />);
    await act(async () => {}); await selectSession();
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.reloadSkills' }));
    await screen.findByText('nativeExtensions.error.session_busy');
    expect(screen.getByRole('button', { name: 'nativeExtensions.reloadSkills' }).hasAttribute('disabled')).toBe(false);
    expect(screen.queryByRole('button', { name: 'nativeExtensions.verify' })).toBeNull();
    expect(requests).toHaveLength(1);
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.reloadSkills' }));
    await screen.findByText('nativeExtensions.action.applied');
    expect(requests).toHaveLength(2);
    expect(requests[0]).not.toBe(requests[1]);
    expect(invoke.mock.calls.some(([command]) => command === 'inspect_native_session_request')).toBe(false);
  });
  it('disables a missing Codex skills method without retaining an unknown mutation', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'list_native_sessions') return [handle];
      if (command === 'native_session_action') throw 'native_extension_Unsupported';
      throw 'unexpected command';
    });
    render(<NativeSessionPanel target="Codex" domain="skills" />);
    await act(async () => {}); await selectSession();
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.reloadSkills' })); });
    expect(screen.queryByRole('button', { name: 'nativeExtensions.verify' })).toBeNull();
    expect(screen.getByRole('button', { name: 'nativeExtensions.observeSession' }).hasAttribute('disabled')).toBe(true);
    expect(screen.queryByRole('button', { name: 'nativeExtensions.reloadSkills' })).toBeNull();
    expect(invoke.mock.calls.filter(([command]) => command === 'native_session_action')).toHaveLength(1);
  });
  it.each(['result_unknown', 'native_extension_Unavailable', 'provider_revision_changed', 'unrecognized-private-error'])('retains uncertainty rather than retrying an unproven failure (%s)', async failure => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'list_native_sessions') return [handle];
      if (command === 'native_session_action') throw failure;
      return null;
    });
    render(<NativeSessionPanel target="Codex" domain="skills" />);
    await act(async () => {}); await selectSession();
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.reloadSkills' }));
    await screen.findByText(/nativeExtensions.action.unknown/);
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.verify' }));
    await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.reloadSkills' }));
    expect(screen.getByRole('button', { name: 'nativeExtensions.reloadSkills' }).hasAttribute('disabled')).toBe(true);
    expect(invoke.mock.calls.filter(([command]) => command === 'native_session_action')).toHaveLength(1);
    if (failure === 'unrecognized-private-error') expect(document.body.textContent).not.toContain(failure);
  });
  it('discards an observation that completes after account replacement', async () => {
    const pending = Promise.withResolvers<unknown>();
    invoke.mockImplementation((command: string) => command === 'observe_native_session' ? pending.promise : Promise.resolve([handle]));
    render(<NativeSessionPanel target="Codex" domain="skills" />);
    await act(async () => {}); await selectSession();
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.observeSession' }));
    await act(async () => { useDevelopment.setState(state => ({ epoch: state.epoch + 1 })); });
    await act(async () => { pending.resolve({ ...handle, domain: 'skills', observed_at: 1, skills: [{ ...observedSkill, name: 'old-private-skill' }], mcp_servers: [], incomplete: false }); });
    expect(screen.queryByText(/old-private-skill/)).toBeNull();
    expect(screen.queryByRole('button', { name: 'nativeExtensions.reloadSkills' })).toBeNull();
  });
  it('verifies the retained request after remount without repeating a native action', async () => {
    let request: { session_id: string; generation: string; request_id: string };
    invoke.mockImplementation(async (command: string, args: { input: typeof request }) => {
      if (command === 'list_native_sessions') return [handle];
      if (command === 'native_session_action') { request = args.input; throw 'connection_lost'; }
      if (command === 'inspect_native_session_request') return { ...request, state: 'applied' };
      throw 'unexpected command';
    });
    const view = render(<NativeSessionPanel target="Codex" domain="skills" />);
    await act(async () => {}); await selectSession();
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.reloadSkills' }));
    await screen.findByText('nativeExtensions.action.unknown');
    view.unmount(); render(<NativeSessionPanel target="Codex" domain="skills" />);
    await act(async () => {}); await selectSession();
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.verify' }));
    await screen.findByText('nativeExtensions.action.applied');
    expect(screen.getByRole('button', { name: 'nativeExtensions.reloadSkills' }).hasAttribute('disabled')).toBe(false);
    expect(invoke.mock.calls.filter(([command]) => command === 'native_session_action')).toHaveLength(1);
    expect(invoke).toHaveBeenCalledWith('inspect_native_session_request', { input: { session_id: handle.session_id, generation: handle.generation, request_id: request!.request_id, domain: 'skills' } });
  });
  it('removes observed connections when the owned runtime closes', async () => {
    invoke.mockImplementation(async (command: string) => command === 'list_native_sessions' ? [handle] : { ...handle, domain: 'skills', observed_at: 1, skills: [observedSkill], mcp_servers: [], incomplete: false });
    render(<NativeSessionPanel target="Codex" domain="skills" />);
    await act(async () => {}); await selectSession();
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.observeSession' }));
    await screen.findByText('observed-skill', { exact: true });
    act(() => useAgentRunner.setState({ session: { id: handle.session_id, generation: handle.generation, engine: 'codex', native_id: null, workspace: '/tmp/project-a', status: 'closed', events: [], pending: [], error: null } }));
    expect(screen.queryAllByText(/observed-skill/)).toHaveLength(0);
    expect(screen.getByText('nativeExtensions.sessionUnobservable')).toBeTruthy();
  });
  it('keeps the original request blocked when receipt lookup returns no evidence', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'list_native_sessions') return [handle];
      if (command === 'inspect_native_session_request') return null;
      throw 'connection_lost';
    });
    render(<NativeSessionPanel target="Codex" domain="skills" />);
    await act(async () => {}); await selectSession();
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.reloadSkills' }));
    await screen.findByText('nativeExtensions.action.unknown');
    fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.verify' }));
    await act(async () => {});
    expect(screen.getByRole('button', { name: 'nativeExtensions.reloadSkills' }).hasAttribute('disabled')).toBe(true);
    expect(invoke.mock.calls.filter(([command]) => command === 'native_session_action')).toHaveLength(1);
  });
});

it.each(['available', 'missing', 'wrong-engine', 'wrong-project'] as const)('resolves a workbench entry only against its exact owned handle (%s)', async scenario => {
 const candidate = scenario === 'wrong-engine' ? { ...handle, engine: 'claude' } : scenario === 'wrong-project' ? { ...handle, project_id: 'another-project' } : handle;
 invoke.mockImplementation(async (command: string) => command === 'list_native_sessions' ? [candidate, { ...handle, session_id: 'manual-choice' }] : Promise.reject('unexpected action'));
 const requested = scenario === 'missing' ? 'closed-session' : handle.session_id;
 const view = render(<NativeSessionPanel target="Codex" domain="skills" initialProjectId="project-a" initialSessionId={requested} />);
 await act(async () => {});
 expect(invoke).toHaveBeenCalledWith('list_native_sessions', { input: { project_id: 'project-a', domain: 'skills' } });
 expect(screen.getByRole('button', { name: 'nativeExtensions.observeSession' }).hasAttribute('disabled')).toBe(scenario !== 'available');
 if (scenario !== 'available') expect(screen.getByText('nativeExtensions.entrySessionUnavailable')).toBeTruthy();
 if (scenario === 'available') {
  fireEvent.click(screen.getByRole('combobox', { name: 'nativeExtensions.chooseSession' }));
  fireEvent.click(await screen.findByRole('option', { name: 'Codex · manual-choice' }));
  view.rerender(<NativeSessionPanel target="Codex" domain="skills" initialProjectId="project-a" initialSessionId={requested} />);
  expect(screen.getByRole('button', { name: 'nativeExtensions.observeSession' }).hasAttribute('disabled')).toBe(false);
  expect(screen.getByRole('combobox', { name: 'nativeExtensions.chooseSession' }).textContent).toContain('manual-choice');
 }
 expect(invoke.mock.calls.some(([command]) => command === 'observe_native_session' || command === 'native_session_action')).toBe(false);
});

it('renders Codex source metadata without turning reported paths into file authority', async () => {
  invoke.mockImplementation(async (command: string) => command === 'list_native_sessions' ? [handle] : { ...handle, domain: 'skills', observed_at: 1, skills: [observedSkill], mcp_servers: [], incomplete: true });
  render(<NativeSessionPanel target="Codex" domain="skills" initialProjectId="project-a" initialSessionId={handle.session_id} />);
  await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.observeSession' }));
  await screen.findByText(observedSkill.path!);
  expect(screen.getByText('nativeExtensions.classification.skill')).toBeTruthy();
  expect(screen.getByText('nativeExtensions.observedScope.repo')).toBeTruthy();
  expect(screen.getByText(observedSkill.plugin_id!)).toBeTruthy();
  expect(screen.getByText('nativeExtensions.observedReadOnly')).toBeTruthy();
  expect(screen.getByText('nativeExtensions.incomplete')).toBeTruthy();
  expect(screen.queryByRole('link')).toBeNull();
  expect(screen.queryByRole('button', { name: 'nativeExtensions.update' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'nativeExtensions.visibility' })).toBeNull();
  fireEvent.click(screen.getByText(observedSkill.path!));
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(['list_native_sessions', 'observe_native_session']);
});

it('distinguishes explicit Claude builtins from unknown command origins and exposes no visibility write', async () => {
  const claude = { ...handle, engine: 'claude' };
  const unknown: NativeObservedSkill = { name: 'help', classification: 'unknown', enabled: null, builtin: null, path: null, scope: null, plugin_id: null };
  invoke.mockImplementation(async (command: string) => command === 'list_native_sessions' ? [claude] : { ...claude, domain: 'skills', observed_at: 1, skills: [{ ...unknown, builtin: true, classification: 'builtin_command' }, unknown, { ...unknown, name: 'mcp__fixture__tool', builtin: false }], mcp_servers: [], incomplete: false });
  render(<NativeSessionPanel target="Claude" domain="skills" initialProjectId="project-a" initialSessionId={handle.session_id} />);
  await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.observeSession' }));
  await screen.findByText('nativeExtensions.classification.builtin_command');
  expect(screen.getAllByText('nativeExtensions.classification.unknown')).toHaveLength(2);
  expect(screen.getAllByText('nativeExtensions.commandSourceUnknown')).toHaveLength(3);
  expect(screen.queryByText('nativeExtensions.classification.skill')).toBeNull();
  expect(screen.queryByRole('button', { name: 'nativeExtensions.enableSkill' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'nativeExtensions.disableSkill' })).toBeNull();
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(['list_native_sessions', 'observe_native_session']);
});

it('shows Claude active source evidence with separate coverage and no file or visibility authority', async () => {
  const claude = { ...handle, engine: 'claude' };
  const active: NativeObservedSkill = { name: 'apps/web:deploy', enabled: null, classification: 'skill', builtin: false, aliases: ['deploy'], source: 'plugin', plugin_name: 'fixture', path: '/native/deploy/SKILL.md', scope: null, plugin_id: 'fixture@marketplace' };
  invoke.mockImplementation(async (command: string) => command === 'list_native_sessions' ? [claude] : { ...claude, domain: 'skills', observed_at: 1, skills: [active, { ...active, name: 'source-only', source: 'project', path: null, plugin_id: null, plugin_name: null }], mcp_servers: [], incomplete: false, source_incomplete: true });
  render(<NativeSessionPanel target="Claude" domain="skills" initialProjectId="project-a" initialSessionId={handle.session_id} />);
  await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.observeSession' }));
  await screen.findByText(active.path!);
  expect(screen.getByText('plugin', { exact: true })).toBeTruthy();
  expect(screen.getByText('project', { exact: true })).toBeTruthy();
  expect(screen.getByText(active.plugin_id!)).toBeTruthy();
  expect(screen.getByText(active.plugin_name!)).toBeTruthy();
  expect(screen.getByText('nativeExtensions.activeSourceCoverage')).toBeTruthy();
  expect(screen.getByText('nativeExtensions.observedReadOnly')).toBeTruthy();
  expect(screen.queryByText('nativeExtensions.incomplete')).toBeNull();
  expect(screen.queryByRole('link')).toBeNull();
  expect(screen.queryByRole('button', { name: 'nativeExtensions.enableSkill' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'nativeExtensions.visibility' })).toBeNull();
  fireEvent.click(screen.getByText(active.path!));
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(['list_native_sessions', 'observe_native_session']);
});

it.each(['managed', 'plugin', 'sdk', null, 'user', 'project', 'local', 'incomplete', 'duplicate', 'unknown-status'] as const)('offers MCP controls only for a complete unambiguous independently admitted native source (%s)', async source => {
  const current = { ...handle, engine: 'claude', capabilities: { ...caps, observe_mcp: true, reconnect_mcp: true, toggle_mcp: true } };
  const server = { name: 'server', status: source === 'unknown-status' ? 'unknown' : 'connected', auth_status: null, source: source === 'incomplete' || source === 'duplicate' || source === 'unknown-status' ? 'user' : source, scope: null, tools: ['echo'], schema_observed: false };
  invoke.mockImplementation(async (command: string) => command === 'list_native_sessions' ? [current] : { ...current, domain: 'mcp', observed_at: 1, skills: [], mcp_servers: source === 'duplicate' ? [server, server] : [server], incomplete: source === 'incomplete' });
  render(<NativeSessionPanel target="Claude" domain="mcp" initialProjectId="project-a" initialSessionId={handle.session_id} />);
  await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.observeSession' }));
  await screen.findAllByText(new RegExp(`server · ${server.status}`));
  const allowed = source === 'user' || source === 'project' || source === 'local';
  expect(screen.queryByRole('button', { name: 'nativeExtensions.reconnect' }) !== null).toBe(allowed);
  expect(screen.queryByRole('button', { name: 'nativeExtensions.enable' }) !== null).toBe(allowed);
  expect(screen.queryByRole('button', { name: 'nativeExtensions.disable' }) !== null).toBe(allowed);
  expect(invoke.mock.calls.some(([command]) => command === 'native_session_action')).toBe(false);
});

it('shows supported observation and precise Codex MCP exclusions without offering a mutation', async () => {
  const current = { ...handle, capabilities: { ...caps, observe_mcp: true, reconnect_mcp: false, toggle_mcp: false, native_login: false, native_logout: false,
    native_auth_guidance: { verified_version: '0.167.0', reason: 'isolated_auth_process_unavailable', login_argv: ['codex', 'mcp', 'login', '<NAME>'], logout_argv: ['codex', 'mcp', 'logout', '<NAME>'] },
    mcp_control_guidance: { verified_version: '0.167.0', reason: 'missing_session_scoped_mcp_control_contract', inspect_argv: ['codex', 'mcp', 'get', '<NAME>'] } } };
  invoke.mockImplementation(async (command: string) => command === 'list_native_sessions' ? [current] : { ...current, domain: 'mcp', observed_at: 1, skills: [], mcp_servers: [{ name: 'fixture', status: 'connected', auth_status: 'oAuth', source: null, scope: null, tools: ['echo'], schema_observed: false }], incomplete: false });
  render(<NativeSessionPanel target="Codex" domain="mcp" initialProjectId="project-a" initialSessionId={handle.session_id} />);
  await act(async () => {});
  expect(screen.getByText('nativeExtensions.guidance.isolated_auth_process_unavailable')).toBeTruthy();
  expect(screen.getByText('nativeExtensions.guidance.missing_session_scoped_mcp_control_contract')).toBeTruthy();
  expect(screen.getByText('codex mcp login <NAME>')).toBeTruthy();
  expect(screen.getByText('codex mcp logout <NAME>')).toBeTruthy();
  expect(screen.getByText('codex mcp get <NAME>')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.observeSession' }));
  await screen.findByText('nativeExtensions.mcpObservedViaCodex');
  for (const name of ['reconnect', 'enable', 'disable']) expect(screen.queryByRole('button', { name: `nativeExtensions.${name}` })).toBeNull();
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(['list_native_sessions', 'observe_native_session']);
});

it.each(['generation-replaced', 'unknown'] as const)('blocks a formerly selected generation when runtime becomes %s', async reason => {
  invoke.mockImplementation(async (command: string) => command === 'list_native_sessions' ? [handle] : { ...handle, domain: 'skills', observed_at: 1, skills: [observedSkill], mcp_servers: [], incomplete: false });
  render(<NativeSessionPanel target="Codex" domain="skills" initialProjectId="project-a" initialSessionId={handle.session_id} />);
  await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.observeSession' }));
  await screen.findByText('observed-skill', { exact: true });
  act(() => useAgentRunner.setState({ session: { id: handle.session_id, generation: reason === 'generation-replaced' ? 'generation-b' : handle.generation, engine: 'codex', native_id: null, workspace: '/tmp/project-a', status: reason === 'unknown' ? 'unknown' : 'ready', events: [], pending: [], error: null } }));
  expect(screen.queryAllByText(/observed-skill/)).toHaveLength(0);
  expect(screen.queryByRole('button', { name: 'nativeExtensions.reloadSkills' })).toBeNull();
  expect(screen.getByRole('button', { name: 'nativeExtensions.observeSession' }).hasAttribute('disabled')).toBe(true);
  expect(invoke.mock.calls.some(([command]) => command === 'native_session_action')).toBe(false);
});

it('separates pinned provider configuration, actual runtime context, and native discovery from execution evidence', () => {
  const snapshot = { binding_id: 'binding', engine: 'codex' as const, provider_id: 'pinned-provider', model_id: 'pinned-model', provider_revision: 3, revision: 2, permission_epoch: 1 };
  const view = render(<CapabilitySnapshotView snapshot={snapshot} runtime={{ workspace: '/actual/session-workspace', cli_version: 'codex-cli 0.167.0' }} />);
  expect(screen.getByText('pinned-provider / pinned-model')).toBeTruthy();
  expect(screen.getByText('/actual/session-workspace')).toBeTruthy();
  expect(screen.getByText('codex-cli 0.167.0')).toBeTruthy();
  expect(screen.getByText('capabilities.catalogSource')).toBeTruthy();
  expect(screen.getByText('capabilities.configurationEvidence')).toBeTruthy();
  expect(screen.getByText('capabilities.nativeSkillsSource')).toBeTruthy();
  expect(screen.getByText('capabilities.nativeMcpSource')).toBeTruthy();
  expect(screen.getByText('capabilities.discoveryEvidence')).toBeTruthy();
  view.rerender(<CapabilitySnapshotView snapshot={snapshot} />);
  expect(screen.queryByText('/actual/session-workspace')).toBeNull();
  expect(screen.queryByText('codex-cli 0.167.0')).toBeNull();
  expect(screen.getAllByText('capabilities.notRecorded')).toHaveLength(2);
});

it.each(['oauth', 'plugin', 'bearer', 'unknown-runtime', 'incomplete', 'duplicate'] as const)('admits only unambiguous Codex OAuth controls without claiming configuration provenance (%s)', async scenario => {
  const current = { ...handle, capabilities: { ...caps, observe_mcp: true, native_login: true, native_logout: true } };
  const server = { name: 'fixture', status: scenario === 'unknown-runtime' ? 'unknown' : 'authenticationRequired', auth_status: scenario === 'bearer' ? 'bearerToken' : 'notLoggedIn', source: scenario === 'plugin' ? 'plugin' : null, scope: null, plugin_id: scenario === 'plugin' ? 'fixture@marketplace' : null, tools: [], schema_observed: false, last_call: null };
  invoke.mockImplementation(async (command: string) => command === 'list_native_sessions' ? [current] : { ...current, domain: 'mcp', observed_at: 1, skills: [], mcp_servers: scenario === 'duplicate' ? [server, server] : [server], incomplete: scenario === 'incomplete', source_incomplete: false });
  render(<NativeSessionPanel target="Codex" domain="mcp" initialProjectId="project-a" initialSessionId={handle.session_id} />);
  await act(async () => {});
  expect(screen.getByRole('button', { name: 'nativeExtensions.auth.login' }).hasAttribute('disabled')).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.observeSession' }));
  await screen.findByText('nativeExtensions.mcpObservedViaCodex');
  expect(screen.getByRole('button', { name: 'nativeExtensions.auth.login' }).hasAttribute('disabled')).toBe(scenario !== 'oauth');
  expect(screen.queryByRole('button', { name: 'nativeExtensions.reconnect' })).toBeNull();
  expect(screen.getAllByText(/nativeExtensions.toolOutcome.unobserved/).length).toBeGreaterThan(0);
  expect(invoke.mock.calls.some(([command]) => command === 'start_native_auth' || command === 'native_session_action')).toBe(false);
});

it('compares fresh file names without granting effective identity and keeps actual call evidence separate', async () => {
  const configSelection = { target: 'Codex' as const, scope: 'ProjectShared' as const, project_id: 'project-a' };
  const key = JSON.stringify([useDevelopment.getState().epoch, configSelection.target, configSelection.scope, configSelection.project_id]);
  const authentication = { configured: true, credential_present: null, authenticated: null, connected: null, approved: null, active_session: null, login: 'Unsupported' as const, logout: 'Unsupported' as const, delete_credentials: 'Unsupported' as const };
  const instances = ['fixture', 'not-discovered'].map(native_key => ({ native_key, transport: 'Http' as const, enabled: true, has_auth_configuration: false, unknown_fields_present: false, owner: 'Loose' as const, authentication }));
  useNativeExtensions.setState({ mcp: { [key]: { value: { instances, incomplete: false }, loading: false, error: null, stale: false } } });
  const current = { ...handle, capabilities: { ...caps, observe_mcp: true } };
  const observed = { ...current, domain: 'mcp', observed_at: 1, skills: [], mcp_servers: [{ name: 'fixture', status: 'connected', auth_status: 'oAuth', source: null, scope: null, tools: ['echo'], schema_observed: false, plugin_id: null, last_call: { tool: 'echo', status: 'failed', event_seq: 7 } }], incomplete: false };
  invoke.mockImplementation(async (command: string) => command === 'list_native_sessions' ? [current] : observed);
  render(<NativeSessionPanel target="Codex" domain="mcp" configSelection={configSelection} initialProjectId="project-a" initialSessionId={handle.session_id} />);
  await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.observeSession' }));
  await screen.findByRole('region', { name: 'nativeExtensions.configComparison' });
  expect(screen.getByText(/fixture · nativeExtensions.configuredFile · nativeExtensions.discoveredSameName/)).toBeTruthy();
  expect(screen.getByText(/not-discovered · nativeExtensions.configuredFile · nativeExtensions.notObservedInSession/)).toBeTruthy();
  expect(screen.getByText('nativeExtensions.configComparisonNotice')).toBeTruthy();
  expect(screen.getByText(/echo · nativeExtensions.toolOutcome.failed/)).toBeTruthy();
  expect(screen.queryByText(/nativeExtensions.toolOutcome.completed/)).toBeNull();
  act(() => useNativeExtensions.setState(state => ({ mcp: { ...state.mcp, [key]: { ...state.mcp[key], stale: true } } })));
  expect(screen.queryByText(/nativeExtensions.discoveredSameName/)).toBeNull();
  expect(screen.getByText('nativeExtensions.configComparisonUnavailable')).toBeTruthy();
});
