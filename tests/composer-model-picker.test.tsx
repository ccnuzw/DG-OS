import type { ComponentProps } from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { ComposerModelPicker } from '../src/components/development/ComposerModelPicker';
import type { CliProviderCatalog, CliProviderConfiguration } from '../src/stores/developmentCapabilities';
import { useDevelopment } from '../src/stores/developmentStore';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key }));

afterEach(() => {
  cleanup();
  useDevelopment.getState().reset();
  invoke.mockReset();
});

type Engine = CliProviderConfiguration['engine'];
type Provider = CliProviderCatalog['providers'][number];
type Props = ComponentProps<typeof ComposerModelPicker>;
type Read = { projectId: string; engine: Engine };
const model = { id: 'model-alpha', name: 'Model Alpha', model_type: 'text' };
const binding: CliProviderConfiguration = { engine: 'codex', provider: { provider_id: 'provider-alpha', model_id: model.id, provider_revision: 17 } };

function provider(overrides: Partial<Provider> = {}): Provider {
  return { id: 'provider-alpha', name: 'Provider Alpha', config_revision: 17, enabled: true, availability: { available: true, reason: null }, models: [{ ...model }], ...overrides };
}
function catalog(engine: Engine, providers: Provider[] = [], projectId = 'project-a'): CliProviderCatalog {
  return { project_id: projectId, engine, providers };
}
function host(catalogs: Partial<Record<Engine, CliProviderCatalog>> = {}) {
  const state = {
    catalogs: { codex: catalog('codex'), claude: catalog('claude'), ...catalogs },
    read: (args: Read): CliProviderCatalog | Promise<CliProviderCatalog> => state.catalogs[args.engine],
  };
  invoke.mockImplementation((command: string, args: Read) => {
    if (command !== 'read_development_cli_provider_catalog') throw new Error(`Unexpected picker IPC: ${command}`);
    return Promise.resolve(state.read(args));
  });
  return state;
}
function picker(onChange: Props['onChange'], props: Partial<Omit<Props, 'onChange'>> = {}) {
  return <ComposerModelPicker projectId="project-a" value={null} preferred={null} {...props} onChange={onChange} />;
}
function trigger() { return screen.getByTestId<HTMLButtonElement>('composer-model-picker-trigger'); }
function option(name = model.name) {
  return screen.getByRole<HTMLButtonElement>('button', { name: new RegExp(name) });
}
async function open() {
  fireEvent.click(trigger());
  await screen.findByTestId('composer-model-picker-dialog');
}
async function ready(name = model.name) {
  await waitFor(() => expect(option(name).disabled).toBe(false));
}

it.each(['codex', 'claude'] as const)('selects the exact provider, model, revision and %s route when it is the only available route', async engine => {
  host({ [engine]: catalog(engine, [provider()]) });
  const onChange = vi.fn();
  render(picker(onChange));
  await open(); await ready();
  expect(onChange).not.toHaveBeenCalled();
  fireEvent.click(option());
  expect(onChange).toHaveBeenCalledExactlyOnceWith({ ...binding, engine });
  expect(screen.queryByTestId('composer-model-picker-dialog')).toBeNull();
});

it('uses the available default route without an extra engine-selection step', async () => {
  host({ codex: catalog('codex', [provider()]), claude: catalog('claude', [provider({ config_revision: 23 })]) });
  const onChange = vi.fn();
  render(picker(onChange));
  await open(); await ready();
  fireEvent.click(option());
  expect(onChange).toHaveBeenCalledExactlyOnceWith(binding);
});

it.each(['codex', 'claude'] as const)('uses the preferred %s engine for another model on the same provider revision', async engine => {
  host({ codex: catalog('codex', [provider()]), claude: catalog('claude', [provider()]) });
  const onChange = vi.fn();
  const preferred = { engine, provider: { ...binding.provider, model_id: 'previous-model' } };
  render(picker(onChange, { preferred }));
  await open(); await ready();
  expect(onChange).not.toHaveBeenCalled();
  fireEvent.click(option());
  expect(onChange).toHaveBeenCalledExactlyOnceWith({ ...binding, engine });
});

it.each([
  { name: 'another provider', preferred: { engine: 'claude' as const, provider: { ...binding.provider, provider_id: 'other-provider' } } },
  { name: 'an older provider revision', preferred: { engine: 'claude' as const, provider: { ...binding.provider, provider_revision: 16 } } },
])('does not borrow a preferred engine from $name', async ({ preferred }) => {
  host({ codex: catalog('codex', [provider()]), claude: catalog('claude', [provider()]) });
  const onChange = vi.fn();
  render(picker(onChange, { preferred }));
  await open(); await ready();
  fireEvent.click(option());
  expect(onChange).toHaveBeenCalledExactlyOnceWith(binding);
});

it.each(['revision', 'disabled', 'deleted'] as const)('does not silently replace a selection after its provider becomes %s', async change => {
  const state = host({ codex: catalog('codex', [provider()]) });
  const onChange = vi.fn();
  render(picker(onChange, { value: binding, preferred: binding }));
  await open(); await ready();
  expect(trigger().getAttribute('aria-invalid')).toBeNull();
  state.catalogs.codex = catalog('codex', [provider(change === 'revision' ? { config_revision: 18 } : change === 'disabled' ? { enabled: false } : { models: [] })]);
  fireEvent.click(screen.getByRole('button', { name: 'modelPicker.refresh' }));
  await waitFor(() => expect(trigger().getAttribute('aria-invalid')).toBe('true'));
  expect(onChange).not.toHaveBeenCalled();
  if (change === 'deleted') {
    expect(screen.queryByRole('button', { name: new RegExp(model.name) })).toBeNull();
  } else if (change === 'disabled') {
    expect(option().disabled).toBe(true);
    fireEvent.click(option());
    expect(onChange).not.toHaveBeenCalled();
  } else {
    fireEvent.click(option());
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ engine: 'codex', provider: { ...binding.provider, provider_revision: 18 } });
  }
});

it('never selects a disabled route when another engine can use the same model', async () => {
  host({ codex: catalog('codex', [provider({ enabled: false })]), claude: catalog('claude', [provider({ config_revision: 23 })]) });
  const onChange = vi.fn();
  render(picker(onChange, { preferred: binding }));
  await open(); await ready();
  fireEvent.click(option());
  expect(onChange).toHaveBeenCalledExactlyOnceWith({ engine: 'claude', provider: { ...binding.provider, provider_revision: 23 } });
});

it('does not expose a late catalog from the previous project over the current project', async () => {
  const previous = { codex: Promise.withResolvers<CliProviderCatalog>(), claude: Promise.withResolvers<CliProviderCatalog>() };
  const currentModel = { ...model, id: 'current-model', name: 'Current Model' };
  const state = host();
  state.read = args => args.projectId === 'project-a' ? previous[args.engine].promise : catalog(args.engine, args.engine === 'claude' ? [provider({ config_revision: 29, models: [currentModel] })] : [], args.projectId);
  const onChange = vi.fn();
  const view = render(picker(onChange));
  await open();
  view.rerender(picker(onChange, { projectId: 'project-b' }));
  expect(screen.queryByTestId('composer-model-picker-dialog')).toBeNull();
  await open(); await ready(currentModel.name);
  await act(async () => {
    previous.codex.resolve(catalog('codex', [provider()]));
    previous.claude.resolve(catalog('claude'));
  });
  expect(screen.queryByRole('button', { name: new RegExp(model.name) })).toBeNull();
  expect(onChange).not.toHaveBeenCalled();
  fireEvent.click(option(currentModel.name));
  expect(onChange).toHaveBeenCalledExactlyOnceWith({ engine: 'claude', provider: { ...binding.provider, model_id: currentModel.id, provider_revision: 29 } });
});

it('does not expose a late catalog after the account epoch changes for the same project', async () => {
  const previous = { codex: Promise.withResolvers<CliProviderCatalog>(), claude: Promise.withResolvers<CliProviderCatalog>() };
  const current = { codex: Promise.withResolvers<CliProviderCatalog>(), claude: Promise.withResolvers<CliProviderCatalog>() };
  const currentModel = { ...model, id: 'new-account-model', name: 'New Account Model' };
  const state = host();
  const previousEpoch = useDevelopment.getState().epoch;
  state.read = args => (useDevelopment.getState().epoch === previousEpoch ? previous : current)[args.engine].promise;
  const onChange = vi.fn();
  render(picker(onChange));
  await open();
  act(() => useDevelopment.getState().reset());
  expect(screen.queryByTestId('composer-model-picker-dialog')).toBeNull();
  await open();
  await act(async () => {
    previous.codex.resolve(catalog('codex', [provider()]));
    previous.claude.resolve(catalog('claude'));
  });
  expect(screen.queryByRole('button', { name: new RegExp(model.name) })).toBeNull();
  expect(onChange).not.toHaveBeenCalled();
  await act(async () => {
    current.codex.resolve(catalog('codex', [provider({ config_revision: 31, models: [currentModel] })]));
    current.claude.resolve(catalog('claude'));
  });
  await ready(currentModel.name);
  expect(screen.queryByRole('button', { name: new RegExp(model.name) })).toBeNull();
  expect(onChange).not.toHaveBeenCalled();
  fireEvent.click(option(currentModel.name));
  expect(onChange).toHaveBeenCalledExactlyOnceWith({ engine: 'codex', provider: { ...binding.provider, model_id: currentModel.id, provider_revision: 31 } });
});
