import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import NativeAuthPanel from '../src/components/nativeExtensions/NativeAuthPanel';
import { useDevelopment } from '../src/stores/developmentStore';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ t: (key: string) => key }));
const session = { session_id: 'owned-session', generation: 'generation-one' };
const servers = [{ name: 'fixture', allowed: true }];
const waiting = { ...session, request_id: '', server_name: 'fixture', kind: 'login', state: 'waiting_callback', reason: null, credential_outcome: 'unknown', authorization_url: 'https://example.test/authorize?state=transient', callback_required: true, expires_at: Date.now() + 180000, exit_code: null };
beforeEach(() => { invoke.mockReset(); useDevelopment.setState(state => ({ epoch: state.epoch + 1 })); });
afterEach(cleanup);
it('keeps one operation identity across remount and clears the transient URL on cancel', async () => {
  let request_id = '';
  let stopped = false;
  invoke.mockImplementation(async (command: string, args: { input: { request_id: string } }) => {
    if (command === 'start_native_auth') request_id = args.input.request_id;
    if (command === 'cancel_native_auth') stopped = true;
    return { ...waiting, request_id, ...(stopped ? { state: 'stopped', reason: 'cancelled', authorization_url: null, callback_required: false } : {}) };
  });
  const view = render(<NativeAuthPanel session={session} servers={servers} disabled={false} />);
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.auth.login' }));
  await screen.findByLabelText('nativeExtensions.auth.authorizationUrl');
  view.unmount(); render(<NativeAuthPanel session={session} servers={servers} disabled={false} />);
  await screen.findByLabelText('nativeExtensions.auth.authorizationUrl');
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.auth.cancel' }));
  await screen.findByText('fixture · nativeExtensions.auth.state.stopped');
  expect(screen.queryByLabelText('nativeExtensions.auth.authorizationUrl')).toBeNull();
  expect(screen.getByText('nativeExtensions.auth.credentialUnknown')).toBeTruthy();
  expect(invoke.mock.calls.filter(([command]) => command === 'start_native_auth')).toHaveLength(1);
  expect(invoke.mock.calls.find(([command]) => command === 'cancel_native_auth')?.[1]).toEqual({ input: { ...session, request_id } });
  expect(invoke.mock.calls.some(([command]) => ['native_session_action', 'close_agent_session', 'interrupt_agent_session', 'send_agent_message'].includes(command))).toBe(false);
});
it('rejects late UI results after account replacement', async () => {
  let release: ((value: unknown) => void) | undefined;
  let request_id = '';
  invoke.mockImplementation((command: string, args: { input: { request_id: string } }) => {
    request_id = args.input.request_id;
    if (command === 'start_native_auth') return new Promise(resolve => { release = resolve; });
    return Promise.reject('session_invalid');
  });
  render(<NativeAuthPanel session={session} servers={servers} disabled={false} />);
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.auth.login' }));
  await act(async () => { useDevelopment.setState(state => ({ epoch: state.epoch + 1 })); release?.({ ...waiting, request_id }); });
  expect(screen.queryByLabelText('nativeExtensions.auth.authorizationUrl')).toBeNull();
  expect(screen.queryByText(request_id)).toBeNull();
  expect(invoke.mock.calls.filter(([command]) => command === 'start_native_auth')).toHaveLength(1);
});

it('keeps cancel disabled while the original start is still being admitted', async () => {
  const pending = Promise.withResolvers<unknown>();
  let request_id = '';
  invoke.mockImplementation((command: string, args: { input: { request_id: string } }) => {
    request_id = args.input.request_id;
    return command === 'start_native_auth' ? pending.promise : Promise.resolve({ ...waiting, request_id });
  });
  render(<NativeAuthPanel session={session} servers={servers} disabled={false} />);
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.auth.login' }));
  await screen.findByLabelText('nativeExtensions.auth.authorizationUrl');
  expect(screen.getByRole('button', { name: 'nativeExtensions.auth.cancel' }).hasAttribute('disabled')).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.auth.cancel' }));
  expect(invoke.mock.calls.some(([command]) => command === 'cancel_native_auth')).toBe(false);
  await act(async () => { pending.resolve({ ...waiting, request_id }); });
  expect(screen.getByRole('button', { name: 'nativeExtensions.auth.cancel' }).hasAttribute('disabled')).toBe(false);
});

it.each(['session_busy', 'native_extension_Unsupported', 'request_capacity'])('allows a new intent only after an explicit pre-process refusal (%s)', async refusal => {
  invoke.mockImplementation(async (command: string) => { if (command === 'start_native_auth') throw refusal; throw 'native_auth_not_found'; });
  render(<NativeAuthPanel session={session} servers={servers} disabled={false} />);
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.auth.login' }));
  await screen.findByText(`nativeExtensions.error.${refusal}`);
  expect(screen.getByRole('button', { name: 'nativeExtensions.auth.login' }).hasAttribute('disabled')).toBe(false);
  expect(screen.queryByRole('button', { name: 'nativeExtensions.verify' })).toBeNull();
  expect(invoke.mock.calls.filter(([command]) => command === 'start_native_auth')).toHaveLength(1);
});

it('clears callback secrets before dispatch and never replays an unknown interaction', async () => {
  const pending = Promise.withResolvers<unknown>();
  let request_id = ''; let submitted = false;
  invoke.mockImplementation((command: string, args: { input: { request_id: string } }) => {
    request_id = args.input.request_id;
    if (command === 'submit_native_auth_callback') { submitted = true; return pending.promise; }
    return Promise.resolve({ ...waiting, request_id, ...(submitted ? { state: 'auth_unknown', reason: 'cleanup_pending', authorization_url: null, callback_required: false } : {}) });
  });
  const view = render(<NativeAuthPanel session={session} servers={servers} disabled={false} />);
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.auth.login' }));
  const callback = await screen.findByLabelText('nativeExtensions.auth.callback');
  fireEvent.change(callback, { target: { value: 'http://localhost/callback?code=private-CALLBACK&state=transient' } });
  fireEvent.click(screen.getByRole('button', { name: 'nativeExtensions.auth.submit' }));
  expect(screen.queryByDisplayValue(/private-CALLBACK/)).toBeNull();
  expect(screen.queryByLabelText('nativeExtensions.auth.authorizationUrl')).toBeNull();
  await act(async () => { pending.reject('private-transport-error'); });
  await screen.findByText('nativeExtensions.auth.controlFailed');
  expect(document.body.textContent).not.toContain('private-transport-error');
  view.unmount(); render(<NativeAuthPanel session={session} servers={servers} disabled={false} />);
  await screen.findByText('fixture · nativeExtensions.auth.state.auth_unknown');
  expect(screen.getByRole('button', { name: 'nativeExtensions.auth.login' }).hasAttribute('disabled')).toBe(true);
  expect(screen.queryByLabelText('nativeExtensions.auth.callback')).toBeNull();
  expect(invoke.mock.calls.filter(([command]) => command === 'submit_native_auth_callback')).toHaveLength(1);
});
