import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { Link, MemoryRouter, useLocation } from 'react-router-dom';
import { createWorkspaceController, type WorkspaceController } from '../src/workspace/controller';
import { TabBar } from '../src/workspace/TabBar';
import { useWorkspaceTabs } from '../src/workspace/useWorkspaceTabs';

vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string, values?: { title?: string }) => values?.title ? `${key}:${values.title}` : key }) }));
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

function Workspace({ controller }: { controller: WorkspaceController }) {
  const tabs = useWorkspaceTabs(controller);
  const location = useLocation();
  return <>
    <Link to="/">Desktop</Link>
    <Link to="/skills">Skills</Link>
    <Link to="/settings/modules">Settings</Link>
    <output aria-label="path">{location.pathname}</output>
    <TabBar {...tabs} />
    <input aria-label="Destination input" />
  </>;
}

it.each([
  ['Settings', '/settings/modules'],
  ['Skills', '/skills'],
])('keeps the new %s destination and its focus when a pending close finishes', async (destination, pathname) => {
  const frames: FrameRequestCallback[] = [];
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => frames.push(callback));
  const controller = createWorkspaceController();
  const pending = Promise.withResolvers<boolean>();
  const guard = vi.fn(() => pending.promise);
  controller.registerGuard('ai', guard);
  render(<MemoryRouter initialEntries={['/workbench']}><Workspace controller={controller} /></MemoryRouter>);

  fireEvent.click(screen.getByRole('button', { name: 'shell.closeTab:nav.workbench' }));
  await waitFor(() => expect(guard).toHaveBeenCalledTimes(1));
  fireEvent.click(screen.getByRole('link', { name: destination }));
  const input = screen.getByRole('textbox', { name: 'Destination input' });
  input.focus();
  await act(async () => { pending.resolve(true); });
  await waitFor(() => expect(screen.queryByRole('tab', { name: 'nav.workbench' })).toBeNull());
  act(() => { frames.splice(0).forEach(frame => frame(0)); });

  expect(screen.getByRole('status', { name: 'path' }).textContent).toBe(pathname);
  expect(document.activeElement).toBe(input);
});

it('navigates to and focuses the remaining tab when the active close has no newer navigation', async () => {
  const frames: FrameRequestCallback[] = [];
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => frames.push(callback));
  const controller = createWorkspaceController();
  controller.open({ kind: 'resource', id: '/skills', title: 'nav.skills' });
  render(<MemoryRouter initialEntries={['/workbench']}><Workspace controller={controller} /></MemoryRouter>);

  fireEvent.click(screen.getByRole('button', { name: 'shell.closeTab:nav.workbench' }));
  await waitFor(() => expect(screen.getByRole('status', { name: 'path' }).textContent).toBe('/skills'));
  act(() => { frames.splice(0).forEach(frame => frame(0)); });

  expect(document.activeElement).toBe(screen.getByRole('tab', { name: 'nav.skills' }));
});

it('keeps newer focus on the same route when an inactive guarded close finishes', async () => {
  const frames: FrameRequestCallback[] = [];
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => frames.push(callback));
  const controller = createWorkspaceController();
  controller.open({ kind: 'resource', id: '/skills', title: 'nav.skills' });
  const pending = Promise.withResolvers<boolean>();
  const guard = vi.fn(() => pending.promise);
  controller.registerGuard('/skills', guard);
  render(<MemoryRouter initialEntries={['/workbench']}><Workspace controller={controller} /></MemoryRouter>);

  fireEvent.click(screen.getByRole('button', { name: 'shell.closeTab:nav.skills' }));
  await waitFor(() => expect(guard).toHaveBeenCalledTimes(1));
  const input = screen.getByRole('textbox', { name: 'Destination input' });
  input.focus();
  await act(async () => { pending.resolve(true); });
  act(() => { frames.splice(0).forEach(frame => frame(0)); });

  expect(screen.getByRole('status', { name: 'path' }).textContent).toBe('/workbench');
  expect(document.activeElement).toBe(input);
});

it('keeps focus moved after close consent but before the queued focus frame', async () => {
  const frames: FrameRequestCallback[] = [];
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => frames.push(callback));
  const controller = createWorkspaceController();
  controller.open({ kind: 'resource', id: '/skills', title: 'nav.skills' });
  render(<MemoryRouter initialEntries={['/workbench']}><Workspace controller={controller} /></MemoryRouter>);

  fireEvent.click(screen.getByRole('button', { name: 'shell.closeTab:nav.workbench' }));
  await waitFor(() => expect(screen.getByRole('status', { name: 'path' }).textContent).toBe('/skills'));
  const input = screen.getByRole('textbox', { name: 'Destination input' });
  input.focus();
  act(() => { frames.splice(0).forEach(frame => frame(0)); });

  expect(document.activeElement).toBe(input);
});
