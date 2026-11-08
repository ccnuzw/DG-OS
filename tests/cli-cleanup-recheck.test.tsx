import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { CliCleanupRecheck } from '../src/components/development/CliCleanupRecheck';
import { useDevelopment } from '../src/stores/developmentStore';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key }));
afterEach(() => { cleanup(); useDevelopment.getState().reset(); invoke.mockReset(); });

it('hides late cleanup evidence after account changes', async () => {
  const pending = Promise.withResolvers<void>();
  invoke.mockReturnValue(pending.promise);
  render(<CliCleanupRecheck />);
  fireEvent.click(screen.getByRole('button', { name: 'dev.recheckCleanup' }));
  expect((screen.getByRole('button') as HTMLButtonElement).disabled).toBe(true);
  act(() => useDevelopment.getState().reset());
  await act(async () => pending.resolve());
  expect(screen.queryByRole('status')).toBeNull();
  expect(screen.queryByRole('alert')).toBeNull();
});

it('reports checked evidence without treating it as confirmed exit or starting a session', async () => {
  invoke.mockResolvedValue(undefined);
  render(<CliCleanupRecheck />);
  fireEvent.click(screen.getByRole('button', { name: 'dev.recheckCleanup' }));
  expect((await screen.findByRole('status')).textContent).toBe('dev.cleanupRechecked');
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(['recheck_development_cli_cleanup']);
});

it('keeps unknown host failures private and lets the user recheck', async () => {
  invoke.mockRejectedValue('private-path-and-credential').mockResolvedValueOnce(undefined);
  render(<CliCleanupRecheck />);
  fireEvent.click(screen.getByRole('button'));
  await screen.findByRole('status');
  fireEvent.click(screen.getByRole('button'));
  expect((await screen.findByRole('alert')).textContent).toBe('dev.error.unknown');
  expect(screen.queryByRole('status')).toBeNull();
  expect((screen.getByRole('button') as HTMLButtonElement).disabled).toBe(false);
});
