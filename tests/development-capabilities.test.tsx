import { StrictMode, useEffect } from 'react';
import { act, cleanup, render } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { readCliProviderCatalog, type CliProviderCatalog } from '../src/stores/developmentCapabilities';
import { useDevelopment } from '../src/stores/developmentStore';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

afterEach(() => {
  cleanup();
  useDevelopment.getState().reset();
  invoke.mockReset();
});

type Engine = CliProviderCatalog['engine'];
function catalog(projectId = 'project-a', engine: Engine = 'codex'): CliProviderCatalog {
  return { project_id: projectId, engine, providers: [] };
}

it('shares overlapping catalog reads during StrictMode effect replay and concurrent consumers', async () => {
  const requests = { codex: Promise.withResolvers<CliProviderCatalog>(), claude: Promise.withResolvers<CliProviderCatalog>() };
  invoke.mockImplementation((command: string, args: { projectId: string; engine: Engine }) => {
    expect(command).toBe('read_development_cli_provider_catalog');
    expect(args.projectId).toBe('project-a');
    return requests[args.engine].promise;
  });
  const settled = vi.fn();
  function Consumer() {
    useEffect(() => {
      void Promise.all([readCliProviderCatalog('project-a', 'codex'), readCliProviderCatalog('project-a', 'claude')]).then(settled);
    }, []);
    return null;
  }
  render(<StrictMode><Consumer /><Consumer /></StrictMode>);
  expect(invoke).toHaveBeenCalledTimes(2);
  await act(async () => {
    requests.codex.resolve(catalog());
    requests.claude.resolve(catalog('project-a', 'claude'));
  });
  expect(settled).toHaveBeenCalledTimes(4);
  expect(settled).toHaveBeenCalledWith([catalog(), catalog('project-a', 'claude')]);
});

it('isolates pending reads by project and engine and releases each settled read independently', async () => {
  const requests = Array.from({ length: 4 }, () => Promise.withResolvers<CliProviderCatalog>());
  let next = 0;
  invoke.mockImplementation(() => requests[next++].promise);
  const codexA = readCliProviderCatalog('project-a', 'codex');
  const claudeA = readCliProviderCatalog('project-a', 'claude');
  const codexB = readCliProviderCatalog('project-b', 'codex');
  expect(readCliProviderCatalog('project-a', 'codex')).toBe(codexA);
  expect(readCliProviderCatalog('project-a', 'claude')).toBe(claudeA);
  expect(readCliProviderCatalog('project-b', 'codex')).toBe(codexB);
  expect(invoke).toHaveBeenCalledTimes(3);
  expect(invoke).toHaveBeenNthCalledWith(1, 'read_development_cli_provider_catalog', { projectId: 'project-a', engine: 'codex' });
  expect(invoke).toHaveBeenNthCalledWith(2, 'read_development_cli_provider_catalog', { projectId: 'project-a', engine: 'claude' });
  expect(invoke).toHaveBeenNthCalledWith(3, 'read_development_cli_provider_catalog', { projectId: 'project-b', engine: 'codex' });
  requests[0].resolve(catalog());
  await expect(codexA).resolves.toEqual(catalog());
  const freshA = readCliProviderCatalog('project-a', 'codex');
  expect(freshA).not.toBe(codexA);
  expect(readCliProviderCatalog('project-a', 'claude')).toBe(claudeA);
  expect(invoke).toHaveBeenCalledTimes(4);
  requests[1].resolve(catalog('project-a', 'claude'));
  requests[2].resolve(catalog('project-b'));
  requests[3].resolve(catalog());
  await Promise.all([claudeA, codexB, freshA]);
});

it('releases failed reads without caching the error or wrapping its evidence', async () => {
  const request = Promise.withResolvers<CliProviderCatalog>();
  const failure = { code: 'provider_unavailable' };
  invoke.mockReturnValueOnce(request.promise).mockResolvedValueOnce(catalog());
  const first = readCliProviderCatalog('project-a', 'codex');
  const second = readCliProviderCatalog('project-a', 'codex');
  expect(second).toBe(first);
  const rejected = expect(first).rejects.toBe(failure);
  request.reject(failure);
  await rejected;
  await expect(readCliProviderCatalog('project-a', 'codex')).resolves.toEqual(catalog());
  expect(invoke).toHaveBeenCalledTimes(2);
});

it('never shares pending catalogs across an account epoch reset for the same project and engine', async () => {
  const previous = Promise.withResolvers<CliProviderCatalog>();
  const current = Promise.withResolvers<CliProviderCatalog>();
  invoke.mockReturnValueOnce(previous.promise).mockReturnValueOnce(current.promise);
  const oldRead = readCliProviderCatalog('project-a', 'codex');
  useDevelopment.getState().reset();
  const currentRead = readCliProviderCatalog('project-a', 'codex');
  expect(currentRead).not.toBe(oldRead);
  expect(invoke).toHaveBeenCalledTimes(2);
  previous.resolve(catalog());
  await oldRead;
  expect(readCliProviderCatalog('project-a', 'codex')).toBe(currentRead);
  current.resolve(catalog());
  await currentRead;
});

it('invalidates owner changes and never lets an old settlement evict a newer same-key flight', async () => {
  const requests = Array.from({ length: 3 }, () => Promise.withResolvers<CliProviderCatalog>());
  let next = 0;
  invoke.mockImplementation(() => requests[next++].promise);
  useDevelopment.setState({ workspaceOwner: 'account:alice' });
  const oldAlice = readCliProviderCatalog('project-a', 'codex');
  useDevelopment.setState({ workspaceOwner: 'account:bob' });
  const bob = readCliProviderCatalog('project-a', 'codex');
  useDevelopment.setState({ workspaceOwner: 'account:alice' });
  const currentAlice = readCliProviderCatalog('project-a', 'codex');
  expect(currentAlice).not.toBe(oldAlice);
  expect(bob).not.toBe(oldAlice);
  expect(invoke).toHaveBeenCalledTimes(3);
  requests[0].resolve(catalog());
  await oldAlice;
  expect(readCliProviderCatalog('project-a', 'codex')).toBe(currentAlice);
  requests[1].resolve(catalog());
  requests[2].resolve(catalog());
  await Promise.all([bob, currentAlice]);
});
