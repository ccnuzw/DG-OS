import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { Modal } from '@dgos/ui';
import { afterEach, expect, it, vi } from 'vitest';
import { AgentConversation } from '../src/pages/workbench/AgentConversation';
import { AgentSkillPicker } from '../src/components/development/AgentSkillPicker';
import { useAgentRunner, type AgentSession } from '../src/stores/agentRunnerStore';
import { useDevelopment } from '../src/stores/developmentStore';
import { guardWorkbenchDrafts, useDevelopmentDrafts } from '../src/stores/developmentDrafts';
import { acknowledgeAgentSkills, agentSkillsKey, captureAgentSkills } from '../src/stores/agentSkills';
import { addConversationReference } from '../src/stores/developmentReferences';
import { removeDevelopmentProject } from '../src/components/development/removeProject';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ locale: 'en-US', t: (key: string) => key }), t: (key: string) => key }));
afterEach(() => { cleanup(); useDevelopment.getState().reset(); useAgentRunner.getState().reset(); useDevelopmentDrafts.getState().clear(); invoke.mockReset(); });
const session: AgentSession = { id: 'skill-session', generation: 'skill-generation', engine: 'codex', native_id: 'native', workspace: '/project', status: 'idle', events: [], pending: [], error: null };
const skill = { name: 'review', path: '/native/review/SKILL.md' };
function select(value = session) {
  useDevelopment.setState({ selection: { project_id: 'p', task_id: null, session_id: value.id } });
  useAgentRunner.setState({ selected: value.id, session: value, snapshots: { [value.id]: value } });
}
function draft(id = 'first', projectId: string | null = 'p') { useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [agentSkillsKey(projectId, session)]: { kind: 'conversation', baseline: [], value: [{ id, skill }] } } })); }
it('shows only current enabled unique native skills and isolates selection by project, session and generation', async () => {
  select();
  invoke.mockResolvedValue({ session_id: session.id, generation: session.generation, engine: 'codex', domain: 'skills', incomplete: false, skills: [{ ...skill, enabled: true, classification: 'skill' }, { name: 'disabled', path: '/disabled', enabled: false, classification: 'skill' }, ...Array.from({ length: 2 }, () => ({ name: 'duplicate', path: '/duplicate', enabled: true, classification: 'skill' }))] });
  render(<AgentSkillPicker projectId="p" session={session} disabled={false} active />);
  fireEvent.click(screen.getByRole('button', { name: 'agentSkills.title' }));
  fireEvent.click(await screen.findByRole('checkbox', { name: /review/ }));
  expect(screen.queryByRole('checkbox', { name: /disabled/ })).toBeNull();
  expect(screen.queryByRole('checkbox', { name: /duplicate/ })).toBeNull();
  expect(captureAgentSkills('p', session).map(item => item.skill)).toEqual([skill]);
  expect(captureAgentSkills('other-project', session)).toEqual([]);
  expect(captureAgentSkills(null, session)).toEqual([]);
  expect(captureAgentSkills('p', { ...session, generation: 'replacement' })).toEqual([]);
  expect(captureAgentSkills('p', { ...session, id: 'other' })).toEqual([]);
  expect(invoke).toHaveBeenCalledWith('observe_native_session', { input: { session_id: session.id, generation: session.generation, domain: 'skills' } });
});
it.each(['codex', 'claude'] as const)('never invents %s skill lists before native discovery', engine => {
  render(<AgentSkillPicker projectId="p" engine={engine} disabled={false} active />);
  fireEvent.click(screen.getByRole('button', { name: 'agentSkills.title' }));
  expect(screen.getByText(engine === 'codex' ? 'agentSkills.beforeStart' : 'agentSkills.unsupported')).toBeTruthy();
  expect(invoke).not.toHaveBeenCalled();
});
it('sends text plus explicit native skill and references, clearing only the acknowledged selection identity', async () => {
  select(); draft();
  const receipt = Promise.withResolvers<null>();
  invoke.mockImplementation(command => command === 'send_agent_message' ? receipt.promise : Promise.resolve(session));
  const reference = { project_id: 'p', task_id: null, path: 'a.ts', kind: 'file' as const, expected_hash: 'version' };
  addConversationReference('p', session.id, reference);
  render(<MemoryRouter><AgentConversation session={session} drafts={{ [session.id]: 'Review this' }} setDraft={vi.fn()} busy={false} error={false} /></MemoryRouter>);
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message')).toBe(true));
  expect(invoke.mock.calls.find(([command]) => command === 'send_agent_message')![1].input).toMatchObject({ text: 'Review this', skills: [skill], references: [reference] });
  act(() => draft('later-reselected'));
  await act(async () => { receipt.resolve(null); });
  expect(captureAgentSkills('p', session).map(item => item.id)).toEqual(['later-reselected']);
});
it.each(['agent_skill_unavailable', 'unknown'])('preserves selected skill after %s and never replays an unknown send', async failure => {
  select(); draft();
  invoke.mockImplementation(command => command === 'send_agent_message' ? Promise.reject(failure) : Promise.resolve(session));
  render(<MemoryRouter><AgentConversation session={session} drafts={{ [session.id]: 'Review this' }} setDraft={vi.fn()} busy={false} error={false} /></MemoryRouter>);
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  await waitFor(() => expect(useAgentRunner.getState().error).toBe(failure));
  expect(captureAgentSkills('p', session).map(item => item.skill)).toEqual([skill]);
  if (failure === 'unknown') {
    await screen.findByRole('button', { name: 'directConversation.recheck' });
    fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
    expect(invoke.mock.calls.filter(([command]) => command === 'send_agent_message')).toHaveLength(1);
  }
});
it('returns to a clean draft after selecting and cancelling the final native skill', async () => {
  select();
  invoke.mockResolvedValue({ session_id: session.id, generation: session.generation, engine: 'codex', domain: 'skills', incomplete: false, skills: [{ ...skill, enabled: true, classification: 'skill' }] });
  render(<AgentSkillPicker projectId="p" session={session} disabled={false} active />);
  fireEvent.click(screen.getByRole('button', { name: 'agentSkills.title' }));
  const checkbox = await screen.findByRole('checkbox', { name: /review/ });
  const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onCancel?.(); return { id: 'confirm', close: () => {} }; });
  try {
    fireEvent.click(checkbox);
    expect(await guardWorkbenchDrafts('p')).toBe(false);
    expect(captureAgentSkills('p', session).map(item => item.skill)).toEqual([skill]);
    confirm.mockClear();
    fireEvent.click(checkbox);
    expect(captureAgentSkills('p', session)).toEqual([]);
    expect(await guardWorkbenchDrafts('p')).toBe(true);
    expect(await guardWorkbenchDrafts()).toBe(true);
    expect(confirm).not.toHaveBeenCalled();
  } finally { confirm.mockRestore(); }
});
it('protects skill drafts before project removal and clears only the removed project after confirmation', async () => {
  select(); draft(); draft('other-project', 'other'); draft('unscoped', null);
  const updateProject = vi.spyOn(useDevelopment.getState(), 'updateProject').mockResolvedValue({ ok: true, value: null });
  const confirm = vi.spyOn(Modal, 'confirm')
    .mockImplementationOnce(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; })
    .mockImplementation(options => { options.onCancel?.(); return { id: 'confirm', close: () => {} }; });
  try {
    expect(await removeDevelopmentProject('p')).toEqual({ ok: false, code: 'cancelled' });
    expect(updateProject).not.toHaveBeenCalled();
    expect(captureAgentSkills('p', session).map(item => item.id)).toEqual(['first']);
    confirm.mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; });
    expect(await removeDevelopmentProject('p')).toEqual({ ok: true, value: null });
    expect(updateProject).toHaveBeenCalledWith('p', 'unlink', null);
    expect(captureAgentSkills('p', session)).toEqual([]);
    expect(captureAgentSkills('other', session).map(item => item.id)).toEqual(['other-project']);
    expect(captureAgentSkills(null, session).map(item => item.id)).toEqual(['unscoped']);
  } finally { updateProject.mockRestore(); confirm.mockRestore(); }
});
it('acknowledges only submitted skill identities within their project scope', () => {
  select(); draft(); draft('other-project', 'other');
  const submitted = captureAgentSkills('p', session);
  const added = { id: 'concurrent', skill: { name: 'implement', path: '/native/implement/SKILL.md' } };
  useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [agentSkillsKey('p', session)]: { kind: 'conversation', baseline: [], value: [...submitted, added] } } }));
  acknowledgeAgentSkills('p', session, submitted);
  expect(captureAgentSkills('p', session)).toEqual([added]);
  expect(captureAgentSkills('other', session).map(item => item.id)).toEqual(['other-project']);
  acknowledgeAgentSkills('p', session, [added]);
  expect(captureAgentSkills('p', session)).toEqual([]);
});
