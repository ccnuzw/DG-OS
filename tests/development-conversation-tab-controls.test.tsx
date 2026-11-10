import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ConversationTabs } from '../src/components/development/ConversationTabs';
import { useDevelopment, type ConversationTab } from '../src/stores/developmentStore';
import { useAgentRunner } from '../src/stores/agentRunnerStore';
import { copyText } from '../src/components/chat/clipboard';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('../src/components/chat/clipboard', () => ({ copyText: vi.fn(async () => true) }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key }));

const tabs: ConversationTab[] = [
  { id: 'draft', projectId: 'p', taskId: null, sessionId: null, title: 'Draft title' },
  { id: 'saved', projectId: 'p', taskId: null, sessionId: 'real-session-id', title: 'Saved title' },
  { id: 'last', projectId: 'p', taskId: null, sessionId: 'last-session-id', title: 'Last title' },
];

beforeEach(() => {
  useDevelopment.setState({ tabs, activeTabId: 'saved', projects: [], tasks: [], sessions: [], sessionNames: {}, workspaceOwner: 'alice' });
});
afterEach(() => {
  cleanup();
  useDevelopment.getState().reset();
  useAgentRunner.getState().reset();
  localStorage.clear();
  vi.clearAllMocks();
});

it('stops a sequential close batch at the retained draft and never touches execution', async () => {
  const close = vi.fn(async () => {});
  const runner = useAgentRunner.getState();
  render(<ConversationTabs busy={false} onSelect={vi.fn()} onClose={close} />);
  fireEvent.contextMenu(screen.getByTestId('conversation-tab-saved'));
  await act(async () => { fireEvent.click(screen.getByRole('menuitem', { name: 'workbenchNavigation.closeAllTabs' })); });
  expect(close).toHaveBeenCalledTimes(1);
  expect(close).toHaveBeenCalledWith(tabs[0]);
  expect(useDevelopment.getState().tabs).toEqual(tabs);
  expect(useAgentRunner.getState()).toBe(runner);
});

it('awaits each close, retains hidden drafts, and stops when a later tab remains open', async () => {
  const pending = Promise.withResolvers<void>();
  const close = vi.fn(async (tab: ConversationTab) => {
    if (tab.id === 'draft') { await pending.promise; useDevelopment.getState().closeTab(tab.id); }
  });
  render(<ConversationTabs busy={false} onSelect={vi.fn()} onClose={close} />);
  fireEvent.contextMenu(screen.getByTestId('conversation-tab-saved'));
  fireEvent.click(screen.getByRole('menuitem', { name: 'workbenchNavigation.closeAllTabs' }));
  expect(close.mock.calls.map(([tab]) => tab.id)).toEqual(['draft']);
  await act(async () => { pending.resolve(); });
  expect(close.mock.calls.map(([tab]) => tab.id)).toEqual(['draft', 'saved']);
  expect(useDevelopment.getState().tabs.filter(tab => !tab.hidden).map(tab => tab.id)).toEqual(['saved', 'last']);
  expect(useDevelopment.getState().tabs.find(tab => tab.id === 'draft')?.hidden).toBe(true);
  expect(screen.queryByTestId('conversation-tab-draft')).toBeNull();
  act(() => { useDevelopment.getState().activateTab('draft'); });
  expect(screen.getByTestId('conversation-tab-draft')).toBeTruthy();
});

it('closes only tabs to the right, in order, and disables empty targets', async () => {
  const close = vi.fn(async (tab: ConversationTab) => { useDevelopment.getState().closeTab(tab.id); });
  render(<ConversationTabs busy={false} onSelect={vi.fn()} onClose={close} />);
  fireEvent.contextMenu(screen.getByTestId('conversation-tab-saved'));
  await act(async () => { fireEvent.click(screen.getByRole('menuitem', { name: 'workbenchNavigation.closeTabsRight' })); });
  expect(close.mock.calls.map(([tab]) => tab.id)).toEqual(['last']);
  expect(useDevelopment.getState().tabs.map(tab => tab.id)).toEqual(['draft', 'saved']);
  fireEvent.contextMenu(screen.getByTestId('conversation-tab-saved'));
  expect(screen.getByRole('menuitem', { name: 'workbenchNavigation.closeTabsRight' }).getAttribute('aria-disabled')).toBe('true');
});

it('locks repeat closes and abandons a batch after the account changes', async () => {
  const pending = Promise.withResolvers<void>();
  const close = vi.fn(() => pending.promise);
  render(<ConversationTabs busy={false} onSelect={vi.fn()} onClose={close} />);
  fireEvent.contextMenu(screen.getByTestId('conversation-tab-saved'));
  fireEvent.click(screen.getByRole('menuitem', { name: 'workbenchNavigation.closeAllTabs' }));
  fireEvent.keyDown(screen.getByTestId('conversation-tab-saved'), { key: 'Delete' });
  expect(close).toHaveBeenCalledTimes(1);
  act(() => { useDevelopment.getState().reset(); useDevelopment.setState({ tabs, activeTabId: 'saved', workspaceOwner: 'bob' }); });
  await act(async () => { pending.resolve(); });
  expect(close).toHaveBeenCalledTimes(1);
  expect(useDevelopment.getState().tabs).toEqual(tabs);
});

it('renames the workspace display name with F2 and navigates using tab keys', () => {
  const select = vi.fn();
  render(<ConversationTabs busy={false} onSelect={select} onClose={vi.fn(async () => {})} />);
  const tab = screen.getByTestId('conversation-tab-saved');
  fireEvent.keyDown(tab, { key: 'End' });
  expect(select).toHaveBeenCalledWith(tabs[2]);
  fireEvent.keyDown(tab, { key: 'F2' });
  fireEvent.change(screen.getByRole('textbox', { name: 'workbenchNavigation.renameTab' }), { target: { value: 'Renamed conversation' } });
  fireEvent.click(screen.getByRole('button', { name: 'common.save' }));
  expect(useDevelopment.getState().tabs.find(tab => tab.id === 'saved')?.title).toBe('Renamed conversation');
  expect(useDevelopment.getState().sessionNames['real-session-id']).toBe('Renamed conversation');
});

it('copies the real session ID and disables that action for unsent drafts', async () => {
  render(<ConversationTabs busy={false} onSelect={vi.fn()} onClose={vi.fn(async () => {})} />);
  fireEvent.keyDown(screen.getByTestId('conversation-tab-saved'), { key: 'F10', shiftKey: true });
  await act(async () => { fireEvent.click(screen.getByRole('menuitem', { name: 'workbenchNavigation.copySessionId' })); });
  expect(copyText).toHaveBeenCalledWith('real-session-id');
  fireEvent.contextMenu(screen.getByTestId('conversation-tab-draft'));
  expect(screen.getByRole('menuitem', { name: 'workbenchNavigation.copySessionId' }).getAttribute('aria-disabled')).toBe('true');
});

it('reveals the requested tab without selecting it and copies its display title', async () => {
  const reveal = vi.fn(async () => {});
  const select = vi.fn();
  render(<ConversationTabs busy={false} onSelect={select} onClose={vi.fn(async () => {})} onReveal={reveal} />);
  fireEvent.contextMenu(screen.getByTestId('conversation-tab-draft'));
  await act(async () => {
    fireEvent.click(screen.getByRole('menuitem', { name: 'workbenchNavigation.revealConversation' }));
    const frame = Promise.withResolvers<void>();
    window.requestAnimationFrame(() => frame.resolve());
    await frame.promise;
  });
  expect(reveal).toHaveBeenCalledWith(tabs[0]);
  expect(select).not.toHaveBeenCalled();
  fireEvent.contextMenu(screen.getByTestId('conversation-tab-draft'));
  await act(async () => { fireEvent.click(screen.getByRole('menuitem', { name: 'workbenchNavigation.copyTitle' })); });
  expect(copyText).toHaveBeenCalledWith('Draft title');
});
