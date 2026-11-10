import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { MemoryRouter } from 'react-router-dom';
import { AgentConversation } from '../src/pages/workbench/AgentConversation';
import { useAgentRunner, type AgentSession } from '../src/stores/agentRunnerStore';
import { AgentImages } from '../src/components/development/AgentImages';
import { fileBase64 } from '../src/components/chat/Attachments';
import { useDevelopment } from '../src/stores/developmentStore';
import { guardWorkbenchDrafts, useDevelopmentDrafts } from '../src/stores/developmentDrafts';
import { acknowledgeAgentImages, addAgentImages, agentImageInputs, agentImagePaste, agentImagesKey, captureAgentImages, clearAgentImages, removeAgentImage, transferAgentImages, useAgentImages } from '../src/stores/agentImages';
import { hostCode } from '../src/stores/hostError';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ locale: 'en-US', t: (key: string, args?: { name?: string }) => args?.name ? `${key} ${args.name}` : key }), t: (key: string) => key }));
vi.mock(import('../src/components/chat/Attachments'), async importOriginal => {
  const original = await importOriginal();
  return { ...original, fileBase64: vi.fn(original.fileBase64) };
});
const PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6bXkAAAAASUVORK5CYII=';
const session = { id: 'session', generation: 'generation' };
const owner = () => agentImagesKey('project', session);
const png = (name = 'selected.png') => new File([Uint8Array.from(atob(PNG), char => char.charCodeAt(0))], name, { type: 'image/png' });
afterEach(() => {
  cleanup(); useAgentImages.getState().reset(); useDevelopmentDrafts.getState().clear(); useDevelopment.getState().reset(); useAgentRunner.getState().reset();
  invoke.mockReset(); vi.mocked(fileBase64).mockClear(); vi.useRealTimers();
});
function Composer({ imageOwner = owner(), disabled = false }: { imageOwner?: string; disabled?: boolean }) {
  return <><textarea aria-label="composer" onPaste={agentImagePaste(imageOwner, disabled)} /><AgentImages owner={imageOwner} disabled={disabled} /></>;
}

it('selects real file bytes, previews locally and removes without native IPC or path forwarding', async () => {
  render(<Composer />);
  fireEvent.change(screen.getByTestId('agent-image-picker'), { target: { files: [png()] } });
  await screen.findByRole('button', { name: 'agentImages.preview selected.png' });
  expect(agentImageInputs(captureAgentImages(owner()))).toEqual([{ mime: 'image/png', base64: PNG }]);
  expect(invoke).not.toHaveBeenCalled();
  fireEvent.click(screen.getByTestId('agent-image-preview-0'));
  expect(await screen.findByTestId('agent-image-dialog')).toBeTruthy();
  expect(screen.getByAltText('selected.png').getAttribute('src')).toBe(`data:image/png;base64,${PNG}`);
  fireEvent.click(screen.getByTestId('agent-image-remove-0'));
  await waitFor(() => expect(captureAgentImages(owner())).toEqual([]));
  expect(screen.queryByTestId('agent-image-dialog')).toBeNull();
});

it('pastes image bytes but leaves text, URLs and arbitrary path clipboard content untouched', async () => {
  render(<Composer />);
  const textarea = screen.getByRole('textbox', { name: 'composer' });
  expect(fireEvent.paste(textarea, { clipboardData: { files: [], items: [], getData: () => 'https://example.com/private.png' } })).toBe(true);
  expect(fireEvent.paste(textarea, { clipboardData: { files: [], items: [], getData: () => '/private/photo.png' } })).toBe(true);
  expect(captureAgentImages(owner())).toEqual([]);
  expect(fireEvent.paste(textarea, { clipboardData: { files: [png('pasted.png')], items: [] } })).toBe(false);
  await screen.findByRole('button', { name: 'agentImages.preview pasted.png' });
  expect(invoke).not.toHaveBeenCalled();
});

it('supports item-only native clipboard images and refuses image mutation while a send is unresolved', async () => {
  const view = render(<Composer />);
  const textarea = screen.getByRole('textbox', { name: 'composer' });
  fireEvent.paste(textarea, { clipboardData: { files: [], items: [{ kind: 'file', type: 'image/png', getAsFile: () => png() }] } });
  await screen.findByRole('button', { name: 'agentImages.preview selected.png' });
  view.rerender(<Composer disabled />);
  expect(screen.getByTestId('agent-image-add')).toHaveProperty('disabled', true);
  expect(screen.getByTestId('agent-image-remove-0')).toHaveProperty('disabled', true);
  fireEvent.paste(textarea, { clipboardData: { files: [png('later.png')], items: [] } });
  expect(captureAgentImages(owner())).toHaveLength(1);
  fireEvent.click(screen.getByTestId('agent-image-preview-0'));
  expect(await screen.findByTestId('agent-image-dialog')).toBeTruthy();
});

it('enforces count, type, per-file and total bounds without discarding the existing draft', async () => {
  const key = owner();
  await addAgentImages(key, [png()]);
  const original = captureAgentImages(key);
  await addAgentImages(key, [png(), png(), png(), png()]);
  expect(useAgentImages.getState().drafts[key].error).toBe('limits');
  await addAgentImages(key, [new File(['<svg/>'], 'image.svg', { type: 'image/svg+xml' })]);
  expect(useAgentImages.getState().drafts[key].error).toBe('type');
  const huge = png(); Object.defineProperty(huge, 'size', { value: 2 * 1024 * 1024 + 1 });
  await addAgentImages(key, [huge]);
  expect(useAgentImages.getState().drafts[key].error).toBe('limits');
  const max = png(); Object.defineProperty(max, 'size', { value: 2 * 1024 * 1024 });
  await addAgentImages(key, [max, max, max]);
  expect(useAgentImages.getState().drafts[key].error).toBe('limits');
  expect(captureAgentImages(key)).toEqual(original);
});

it('transfers image-only first-send draft identities to a new session and acknowledges only submitted IDs', async () => {
  const draft = agentImagesKey('project', 'new-conversation');
  await addAgentImages(draft, [png('first-send.png')]);
  const captured = captureAgentImages(draft);
  const key = owner();
  transferAgentImages(draft, key);
  expect(captureAgentImages(draft)).toEqual([]);
  expect(captureAgentImages(key)).toEqual(captured);
  expect(agentImageInputs(captured)).toEqual([{ mime: 'image/png', base64: PNG }]);
  await addAgentImages(key, [png('later.png')]);
  acknowledgeAgentImages(key, captured);
  expect(captureAgentImages(key).map(image => image.name)).toEqual(['later.png']);
  expect(useDevelopmentDrafts.getState().entries[draft]).toBeUndefined();
  expect(useDevelopmentDrafts.getState().entries[key].value).toEqual(captureAgentImages(key).map(image => image.id));
});

it('preserves reselected identical bytes and another session when an earlier request is acknowledged', async () => {
  const key = owner();
  await addAgentImages(key, [png()]);
  const submitted = captureAgentImages(key);
  removeAgentImage(key, submitted[0].id);
  await addAgentImages(key, [png()]);
  const other = agentImagesKey('project', { ...session, id: 'other' });
  await addAgentImages(other, [png()]);
  acknowledgeAgentImages(key, submitted);
  expect(captureAgentImages(key)).toHaveLength(1);
  expect(captureAgentImages(key)[0].id).not.toBe(submitted[0].id);
  expect(captureAgentImages(other)).toHaveLength(1);
});

it('does not repopulate acknowledged images when a newer import finishes', async () => {
  const key = owner();
  await addAgentImages(key, [png()]);
  const submitted = captureAgentImages(key);
  const pending = Promise.withResolvers<string>();
  vi.mocked(fileBase64).mockReturnValueOnce(pending.promise);
  const importing = addAgentImages(key, [png('new.png')]);
  acknowledgeAgentImages(key, submitted);
  pending.resolve(PNG); await importing;
  expect(captureAgentImages(key).map(image => image.name)).toEqual(['new.png']);
});

it.each(['reset', 'discard', 'clear'] as const)('discards late FileReader results after %s and never writes bytes to generic draft guards', async change => {
  const key = owner();
  const pending = Promise.withResolvers<string>();
  vi.mocked(fileBase64).mockReturnValueOnce(pending.promise);
  const importing = addAgentImages(key, [png()]);
  expect(useAgentImages.getState().drafts[key].busy).toBe(true);
  expect(JSON.stringify(useDevelopmentDrafts.getState().entries)).not.toContain(PNG);
  if (change === 'reset') useDevelopment.getState().reset();
  else if (change === 'discard') useDevelopmentDrafts.getState().clear();
  else clearAgentImages(key);
  pending.resolve(PNG); await importing;
  expect(captureAgentImages(key)).toEqual([]);
  expect(useAgentImages.getState().drafts[key]).toBeUndefined();
});

it('isolates project, generation and account keys and expires unsent previews without sending them', async () => {
  const key = owner();
  await addAgentImages(key, [png()]);
  expect(captureAgentImages(agentImagesKey('other-project', session))).toEqual([]);
  expect(captureAgentImages(agentImagesKey('project', { ...session, generation: 'replaced' }))).toEqual([]);
  expect(useDevelopmentDrafts.getState().entries[key].value).toEqual(captureAgentImages(key).map(image => image.id));
  clearAgentImages(key);
  vi.useFakeTimers();
  vi.mocked(fileBase64).mockResolvedValueOnce(PNG);
  await addAgentImages(key, [png()]);
  act(() => vi.advanceTimersByTime(60 * 60 * 1000));
  expect(captureAgentImages(key)).toEqual([]);
  expect(useAgentImages.getState().drafts[key].error).toBe('expired');
  expect(useDevelopmentDrafts.getState().entries[key]).toBeUndefined();
  expect(invoke).not.toHaveBeenCalled();
});

it('marks image-only drafts as unsaved until the submitted image is acknowledged', async () => {
  await addAgentImages(owner(), [png()]);
  // Generic guards receive only opaque image identities, not image payloads.
  expect(Object.values(useDevelopmentDrafts.getState().entries).some(entry => entry.kind === 'conversation')).toBe(true);
  const captured = captureAgentImages(owner());
  acknowledgeAgentImages(owner(), captured);
  expect(await guardWorkbenchDrafts('project')).toBe(true);
});

it('closes a hidden preview and rejects a file chooser completed for the previous draft owner', async () => {
  const key = owner();
  await addAgentImages(key, [png()]);
  const view = render(<AgentImages owner={key} active />);
  fireEvent.click(screen.getByTestId('agent-image-preview-0'));
  await screen.findByTestId('agent-image-dialog');
  view.rerender(<AgentImages owner={key} active={false} />);
  await waitFor(() => expect(screen.queryByTestId('agent-image-dialog')).toBeNull());
  const next = agentImagesKey('project', 'different-draft');
  view.rerender(<AgentImages owner={next} active />);
  fireEvent.change(screen.getByTestId('agent-image-picker'), { target: { files: [png('old-chooser.png')] } });
  expect(captureAgentImages(next)).toEqual([]);
  expect(captureAgentImages(key)).toHaveLength(1);
});


it('dispatches an existing-session image-only turn and acknowledges only the image identities submitted at click time', async () => {
  const live: AgentSession = { ...session, engine: 'codex', native_id: 'native', workspace: '/project', status: 'idle', events: [], pending: [], error: null };
  useDevelopment.setState({ selection: { project_id: 'project', task_id: null, session_id: live.id } });
  useAgentRunner.setState({ selected: live.id, session: live, snapshots: { [live.id]: live } });
  await addAgentImages(owner(), [png()]);
  const accepted = Promise.withResolvers<null>();
  invoke.mockImplementation(command => command === 'send_agent_message' ? accepted.promise : Promise.resolve(live));
  render(<MemoryRouter><AgentConversation session={live} drafts={{ [live.id]: '' }} setDraft={vi.fn()} busy={false} error={false} /></MemoryRouter>);
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message')).toBe(true));
  const input = invoke.mock.calls.find(([command]) => command === 'send_agent_message')![1].input;
  expect(input).toMatchObject({ session_id: live.id, generation: live.generation, text: '', images: [{ mime: 'image/png', base64: PNG }] });
  expect(JSON.stringify(useDevelopmentDrafts.getState().entries)).not.toContain(PNG);
  await act(async () => { await addAgentImages(owner(), [png('next.png')]); accepted.resolve(null); });
  await waitFor(() => expect(captureAgentImages(owner()).map(image => image.name)).toEqual(['next.png']));
  expect(invoke.mock.calls.filter(([command]) => command === 'send_agent_message')).toHaveLength(1);
});

it('retains existing-session unknown image sends and reconciles the host run identity without replay', async () => {
  let live: AgentSession = { ...session, engine: 'codex', native_id: 'native', workspace: '/project', status: 'idle', events: [], pending: [], error: null };
  useDevelopment.setState({ selection: { project_id: 'project', task_id: null, session_id: live.id } });
  useAgentRunner.setState({ selected: live.id, session: live, snapshots: { [live.id]: live } });
  await addAgentImages(owner(), [png()]);
  invoke.mockImplementation(command => command === 'send_agent_message' ? Promise.reject('unknown') : Promise.resolve(live));
  render(<MemoryRouter><AgentConversation session={live} drafts={{ [live.id]: '' }} setDraft={vi.fn()} busy={false} error={false} /></MemoryRouter>);
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  await screen.findByRole('button', { name: 'directConversation.recheck' });
  const input = invoke.mock.calls.find(([command]) => command === 'send_agent_message')![1].input;
  expect(captureAgentImages(owner())).toHaveLength(1);
  expect(JSON.stringify(useDevelopmentDrafts.getState().entries)).not.toContain(PNG);
  expect(JSON.stringify(useDevelopmentDrafts.getState().entries)).not.toContain('selected.png');
  expect(screen.getByTestId('agent-image-remove-0')).toHaveProperty('disabled', true);
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  live = { ...live, active_run: input.request_id, status: 'running' };
  fireEvent.click(screen.getByRole('button', { name: 'directConversation.recheck' }));
  await waitFor(() => expect(captureAgentImages(owner())).toEqual([]));
  expect(invoke.mock.calls.filter(([command]) => command === 'send_agent_message')).toHaveLength(1);
});

it.each(['agent_image_count_limit', 'agent_image_bytes_limit', 'agent_image_total_limit', 'agent_image_type', 'agent_image_invalid', 'agent_image_dimensions', 'agent_images_unsupported'])('preserves definite host image validation code %s without exposing arbitrary host details', code => {
  expect(hostCode(code)).toBe(code);
  expect(hostCode(`${code}: /private/image.png`)).toBe('unknown');
  expect(hostCode(new Error(code))).toBe('unknown');
});

it.each(['invalid_git_hunk', 'restore_confirmation_required', 'takeover_stale', 'takeover_confirmation_required'])('preserves definite development refusal %s without accepting attached host details', code => {
  expect(hostCode(code)).toBe(code);
  expect(hostCode(`${code}: /private/workspace`)).toBe('unknown');
});
