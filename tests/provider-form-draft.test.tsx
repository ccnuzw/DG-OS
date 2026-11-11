import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import ProviderForm from '../src/components/ProviderForm';
import { setLocaleForTesting } from '../src/i18n/runtime';
import { useProviderStore } from '../src/stores/providerStore';
import type { Provider, ProviderConfig } from '../src/stores/providerStore';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const mockedInvoke = vi.mocked(invoke);
const initialState = useProviderStore.getState();

const provider: Provider = {
  id: 'draft-provider', name: 'Saved gateway', provider_type: 'openai_compatible',
  base_url: 'https://example.invalid/v1', enabled: true, sort_order: 0,
  created_at: '2026-11-01T00:00:00Z', updated_at: '2026-11-01T00:00:00Z',
  user_id: 'default-user', credential_status: 'stored', config_revision: 1,
  config: { vendor_id: 'custom' },
};

function renderForm(current = provider, onSave = vi.fn(async () => true)) {
  return render(<ProviderForm provider={current} onSave={onSave} onChange={() => {}} />);
}

function cliRoute(engine: 'Codex (Responses)' | 'Claude Code (Messages)') {
  return within(screen.getByRole('group', { name: engine }));
}

async function chooseCliOption(engine: 'Codex (Responses)' | 'Claude Code (Messages)', label: string, option: string) {
  const trigger = cliRoute(engine).getByRole('combobox', { name: label });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'Enter' });
  const listbox = await screen.findByRole('listbox');
  fireEvent.click(within(listbox).getByRole('option', { name: option, exact: true }));
  await waitFor(() => expect(screen.queryByRole('listbox')).toBeNull());
}

beforeEach(() => {
  setLocaleForTesting('en-US');
  useProviderStore.setState(initialState, true);
  mockedInvoke.mockResolvedValue({ success: true, data: true });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('ProviderForm saved drafts', () => {
  it('keeps connection edits through toggle and refresh revisions, and saves the current enabled state', async () => {
    const onSave = vi.fn(async (_config: ProviderConfig) => true);
    const view = renderForm(provider, onSave);
    fireEvent.change(screen.getByLabelText('Name *'), { target: { value: 'Draft gateway' } });
    fireEvent.change(screen.getByLabelText('Base URL *'), { target: { value: 'https://draft.invalid/v2' } });
    fireEvent.change(screen.getByLabelText('API Key *'), { target: { value: 'draft-key' } });

    view.rerender(<ProviderForm provider={{ ...provider, enabled: false, config_revision: 2 }} onSave={onSave} onChange={() => {}} />);
    expect((screen.getByLabelText('Name *') as HTMLInputElement).value).toBe('Draft gateway');
    expect((screen.getByLabelText('Base URL *') as HTMLInputElement).value).toBe('https://draft.invalid/v2');

    view.rerender(<ProviderForm provider={{ ...provider, enabled: false, config_revision: 3 }} onSave={onSave} onChange={() => {}} />);
    expect((screen.getByLabelText('Name *') as HTMLInputElement).value).toBe('Draft gateway');
    fireEvent.click(screen.getByRole('button', { name: 'Save connection configuration' }));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect(onSave.mock.calls[0][0].enabled).toBe(false);
    expect(onSave.mock.calls[0][0].base_url).toBe('https://draft.invalid/v2');
  });

  it('blocks testing an advanced draft with the saved key instead of testing the old connection', () => {
    renderForm();
    fireEvent.click(screen.getByRole('button', { name: 'Advanced options' }));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Allow plain HTTP' }));

    expect(screen.getByRole('button', { name: 'Test Connection' })).toHaveProperty('disabled', true);
    expect(mockedInvoke).not.toHaveBeenCalled();
  });

  it('tests advanced settings with a newly entered key using the draft connection', async () => {
    renderForm();
    fireEvent.click(screen.getByRole('button', { name: 'Advanced options' }));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Allow plain HTTP' }));
    fireEvent.change(screen.getByLabelText('API Key (optional)'), { target: { value: 'draft-key' } });
    fireEvent.click(screen.getByRole('button', { name: 'Test Connection' }));

    await waitFor(() => expect(mockedInvoke).toHaveBeenCalledWith('test_provider_connection', {
      config: expect.objectContaining({ api_key: 'draft-key', config: expect.objectContaining({ allow_insecure_http: true }) }),
    }));
  });
  it.each([false, true])('fetches an edited connection without saving (tested first: %s)', async (testFirst) => {
    const onSave = vi.fn(async () => true);
    const onModelsRefreshed = vi.fn(async () => {});
    const remote = { id: 'draft-model', provider_id: '', model_id: 'draft-model', model_name: 'Draft model', model_type: 'embedding', supports_streaming: false, supports_vision: false, cached_at: '' };
    mockedInvoke.mockImplementation(async command => ({ success: true, data: command === 'list_models_for_config' ? [remote] : true }));
    render(<ProviderForm provider={provider} onSave={onSave} onModelsRefreshed={onModelsRefreshed} onChange={() => {}} />);
    fireEvent.change(screen.getByLabelText('Base URL *'), { target: { value: 'https://draft.invalid/v1' } });
    expect(screen.getByRole('button', { name: 'Fetch model preview' })).toHaveProperty('disabled', true);
    fireEvent.change(screen.getByLabelText(/^API Key/), { target: { value: 'draft-key' } });
    if (testFirst) {
      fireEvent.click(screen.getByRole('button', { name: 'Test Connection' }));
      await waitFor(() => expect(screen.getByRole('button', { name: 'Test Connection' })).toHaveProperty('disabled', false));
    }
    fireEvent.click(screen.getByRole('button', { name: 'Fetch model preview' }));
    expect(await screen.findByRole('button', { name: /^Draft model / })).toBeTruthy();
    expect(mockedInvoke).toHaveBeenCalledWith('list_models_for_config', { config: expect.objectContaining({ base_url: 'https://draft.invalid/v1', api_key: 'draft-key', provider_type: 'openai_compatible' }) });
    expect(mockedInvoke).not.toHaveBeenCalledWith('refresh_models', expect.anything());
    expect(onSave).not.toHaveBeenCalled();
    expect(onModelsRefreshed).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: 'Add model manually' })).toBeNull();
    fireEvent.change(screen.getByLabelText('Base URL *'), { target: { value: 'https://another.invalid/v1' } });
    expect(screen.queryByRole('button', { name: /^Draft model / })).toBeNull();
  });

  it('restores saved models on discard and rejects a late draft preview', async () => {
    const saved = { id: 'saved-model', provider_id: provider.id, model_id: 'saved-model', model_name: 'Saved model', model_type: 'embedding', supports_streaming: false, supports_vision: false, cached_at: '' };
    const { promise, resolve } = Promise.withResolvers<unknown>();
    mockedInvoke.mockImplementation(() => promise);
    render(<ProviderForm provider={provider} initialModels={[saved]} onSave={vi.fn()} onChange={() => {}} />);
    fireEvent.change(screen.getByLabelText(/^API Key/), { target: { value: 'draft-key' } });
    fireEvent.click(screen.getByRole('button', { name: 'Fetch model preview' }));
    fireEvent.click(screen.getByRole('button', { name: 'Discard' }));
    await act(async () => { resolve({ success: true, data: [{ ...saved, id: 'late', model_name: 'Late draft' }] }); });
    expect(screen.getByRole('button', { name: /^Saved model / })).toBeTruthy();
    expect(screen.queryByRole('button', { name: /^Late draft / })).toBeNull();
    expect(screen.getByRole('button', { name: 'Fetch models' })).toHaveProperty('disabled', false);
  });

  it('fetches a new provider preview without a name, test, or save', async () => {
    mockedInvoke.mockImplementation(async command => ({ success: true, data: command === 'list_models_for_config' ? [{ id: 'new', provider_id: '', model_id: 'new-model', model_name: 'New model', model_type: 'embedding', supports_streaming: false, supports_vision: false, cached_at: '' }] : [] }));
    render(<ProviderForm provider={null} onSave={vi.fn()} onChange={() => {}} />);
    fireEvent.change(screen.getByLabelText('Base URL *'), { target: { value: 'https://new.invalid/v1' } });
    fireEvent.change(screen.getByLabelText(/^API Key/), { target: { value: 'new-key' } });
    fireEvent.click(screen.getByRole('button', { name: 'Fetch model preview' }));
    expect(await screen.findByRole('button', { name: /^New model / })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Create', exact: true })).toHaveProperty('disabled', true);
  });
});

describe('ProviderForm CLI routes', () => {
  it('keeps absent routes automatic and does not persist declarations on unrelated edits', async () => {
    const onSave = vi.fn(async (_config: ProviderConfig) => true);
    renderForm(provider, onSave);

    expect(cliRoute('Codex (Responses)').getByRole('combobox', { name: 'Route' }).textContent).toContain('Auto');
    expect(cliRoute('Codex (Responses)').getByText('Automatic candidate for this provider type, not verified.')).toBeTruthy();
    expect(cliRoute('Claude Code (Messages)').getByText(/Not an automatic candidate/)).toBeTruthy();
    expect(screen.queryByLabelText('CLI base URL (optional)')).toBeNull();
    expect(mockedInvoke).not.toHaveBeenCalled();

    fireEvent.change(screen.getByLabelText('Name *'), { target: { value: 'Renamed gateway' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save connection configuration' }));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect(onSave.mock.calls[0][0].config).toEqual(provider.config);
    expect(onSave.mock.calls[0][0].config).not.toHaveProperty('cli_routes');
  });

  it('saves both routes independently with the existing key and preserves unrelated configuration', async () => {
    const current = { ...provider, config: { ...provider.config, workspace_id: 'workspace-1', extension: { retained: true } } };
    const onSave = vi.fn(async (_config: ProviderConfig) => true);
    renderForm(current, onSave);
    await chooseCliOption('Codex (Responses)', 'Route', 'Enabled');
    await chooseCliOption('Claude Code (Messages)', 'Route', 'Enabled');
    fireEvent.change(cliRoute('Codex (Responses)').getByLabelText('CLI base URL (optional)'), {
      target: { value: 'https://responses.invalid/v1' },
    });
    fireEvent.change(cliRoute('Claude Code (Messages)').getByLabelText('CLI base URL (optional)'), {
      target: { value: 'https://messages.invalid' },
    });
    await chooseCliOption('Codex (Responses)', 'CLI auth header style', 'Bearer');
    await chooseCliOption('Claude Code (Messages)', 'CLI auth header style', 'Both');

    expect(cliRoute('Codex (Responses)').getByRole('note').textContent).toContain('https://responses.invalid/v1');
    expect(cliRoute('Claude Code (Messages)').getByRole('note').textContent).toContain('https://messages.invalid');
    expect(screen.getByRole('button', { name: 'Fetch models' })).toHaveProperty('disabled', false);
    expect(screen.getByRole('button', { name: 'Test Connection' })).toHaveProperty('disabled', false);
    expect(mockedInvoke).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Save connection configuration' }));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect(onSave.mock.calls[0][0]).toMatchObject({
      provider_type: provider.provider_type, base_url: provider.base_url, api_key: '',
      config: {
        ...current.config,
        cli_routes: {
          responses: { enabled: true, base_url: 'https://responses.invalid/v1', auth_style: 'bearer' },
          messages: { enabled: true, base_url: 'https://messages.invalid', auth_style: 'both' },
        },
      },
    });
    expect(screen.getByRole('button', { name: 'Save connection configuration' })).toHaveProperty('disabled', true);
  });

  it('loads saved overrides and disables only the selected CLI without losing its settings', async () => {
    const current = { ...provider, config: {
      ...provider.config,
      cli_routes: {
        responses: { enabled: true, base_url: 'https://responses.invalid/v1', auth_style: 'bearer' },
        messages: { enabled: true, base_url: 'https://messages.invalid', auth_style: 'x-api-key' },
      },
    } };
    const onSave = vi.fn(async (_config: ProviderConfig) => true);
    renderForm(current, onSave);
    expect((cliRoute('Claude Code (Messages)').getByLabelText('CLI base URL (optional)') as HTMLInputElement).value).toBe('https://messages.invalid');
    expect(cliRoute('Claude Code (Messages)').getByRole('combobox', { name: 'CLI auth header style' }).textContent).toContain('x-api-key');
    await chooseCliOption('Codex (Responses)', 'Route', 'Disabled');
    expect(cliRoute('Codex (Responses)').queryByLabelText('CLI base URL (optional)')).toBeNull();
    expect(cliRoute('Codex (Responses)').getByRole('status').textContent).toContain('Disabled for this CLI');
    fireEvent.click(screen.getByRole('button', { name: 'Save connection configuration' }));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect(onSave.mock.calls[0][0].config?.cli_routes).toEqual({
      ...current.config.cli_routes,
      responses: { ...current.config.cli_routes.responses, enabled: false },
    });
    await chooseCliOption('Codex (Responses)', 'Route', 'Enabled');
    expect((cliRoute('Codex (Responses)').getByLabelText('CLI base URL (optional)') as HTMLInputElement).value).toBe('https://responses.invalid/v1');
  });

  it('removes only an automatic route declaration and removes the container when both are automatic', async () => {
    const current = { ...provider, config: { ...provider.config, retained: { count: 2 }, cli_routes: {
      responses: { enabled: true }, messages: { enabled: false },
    } } };
    const onSave = vi.fn(async (_config: ProviderConfig) => true);
    renderForm(current, onSave);
    await chooseCliOption('Codex (Responses)', 'Route', 'Auto');
    fireEvent.click(screen.getByRole('button', { name: 'Save connection configuration' }));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect(onSave.mock.calls[0][0].config?.cli_routes).toEqual({ messages: { enabled: false } });
    await chooseCliOption('Claude Code (Messages)', 'Route', 'Auto');
    fireEvent.click(screen.getByRole('button', { name: 'Save connection configuration' }));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(2));
    expect(onSave.mock.calls[1][0].config).toEqual({ ...provider.config, retained: { count: 2 } });
  });

  it('restores saved route modes and details on discard, including removing unsaved routes', async () => {
    const current = { ...provider, config: { ...provider.config, cli_routes: {
      messages: { enabled: true, base_url: 'https://saved.invalid', auth_style: 'x-api-key' },
    } } };
    renderForm(current);
    await chooseCliOption('Codex (Responses)', 'Route', 'Enabled');
    fireEvent.change(cliRoute('Claude Code (Messages)').getByLabelText('CLI base URL (optional)'), {
      target: { value: 'https://draft.invalid' },
    });
    await chooseCliOption('Claude Code (Messages)', 'CLI auth header style', 'Bearer');
    fireEvent.click(screen.getByRole('button', { name: 'Discard' }));

    expect(cliRoute('Codex (Responses)').getByRole('combobox', { name: 'Route' }).textContent).toContain('Auto');
    expect(cliRoute('Codex (Responses)').queryByLabelText('CLI base URL (optional)')).toBeNull();
    expect((cliRoute('Claude Code (Messages)').getByLabelText('CLI base URL (optional)') as HTMLInputElement).value).toBe('https://saved.invalid');
    expect(cliRoute('Claude Code (Messages)').getByRole('combobox', { name: 'CLI auth header style' }).textContent).toContain('x-api-key');
    expect(screen.getByRole('button', { name: 'Discard' })).toHaveProperty('disabled', true);
    expect(mockedInvoke).not.toHaveBeenCalled();
  });

  it('retains CLI drafts through revisions and saves the current provider enabled state', async () => {
    const onSave = vi.fn(async (_config: ProviderConfig) => true);
    const view = renderForm(provider, onSave);
    await chooseCliOption('Claude Code (Messages)', 'Route', 'Enabled');
    fireEvent.change(cliRoute('Claude Code (Messages)').getByLabelText('CLI base URL (optional)'), {
      target: { value: 'https://draft.invalid' },
    });
    view.rerender(<ProviderForm provider={{ ...provider, enabled: false, config_revision: 2 }} onSave={onSave} onChange={() => {}} />);
    expect((cliRoute('Claude Code (Messages)').getByLabelText('CLI base URL (optional)') as HTMLInputElement).value).toBe('https://draft.invalid');
    fireEvent.click(screen.getByRole('button', { name: 'Save connection configuration' }));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect(onSave.mock.calls[0][0]).toMatchObject({ enabled: false, config: { cli_routes: {
      messages: { enabled: true, base_url: 'https://draft.invalid' },
    } } });
  });

  it('keeps CLI routes unverified after an ordinary saved-connection test', async () => {
    renderForm();
    await chooseCliOption('Codex (Responses)', 'Route', 'Enabled');
    await chooseCliOption('Claude Code (Messages)', 'Route', 'Enabled');
    expect(screen.getByText(/This test checks only the ordinary Provider adapter/)).toBeTruthy();
    expect(mockedInvoke).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Test Connection' }));
    await screen.findByText('Model list access succeeded. Individual models and CLI compatibility were not tested.');
    expect(mockedInvoke).toHaveBeenCalledExactlyOnceWith('test_provider_by_id', { providerId: provider.id });
    expect(cliRoute('Codex (Responses)').getByRole('status').textContent).toContain('not verified');
    expect(cliRoute('Claude Code (Messages)').getByRole('status').textContent).toContain('not verified');
  });

  it('returns to a clean draft when optional overrides are cleared and leaves defaults absent on save', async () => {
    const current = { ...provider, config: { ...provider.config, cli_routes: { responses: { enabled: true } } } };
    const onSave = vi.fn(async (_config: ProviderConfig) => true);
    renderForm(current, onSave);
    const endpoint = cliRoute('Codex (Responses)').getByLabelText('CLI base URL (optional)');
    expect(endpoint.getAttribute('placeholder')).toBe(provider.base_url);
    fireEvent.change(endpoint, { target: { value: 'https://temporary.invalid/v1' } });
    const warning = cliRoute('Codex (Responses)').getByRole('note');
    expect(endpoint.getAttribute('aria-describedby')).toContain(warning.id);
    fireEvent.change(endpoint, { target: { value: '' } });
    await chooseCliOption('Codex (Responses)', 'CLI auth header style', 'Bearer');
    await chooseCliOption('Codex (Responses)', 'CLI auth header style', 'Inherit / protocol default');
    expect(cliRoute('Codex (Responses)').queryByRole('note')).toBeNull();
    expect(screen.getByRole('button', { name: 'Save connection configuration' })).toHaveProperty('disabled', true);
    await chooseCliOption('Claude Code (Messages)', 'Route', 'Enabled');
    fireEvent.click(screen.getByRole('button', { name: 'Save connection configuration' }));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect(onSave.mock.calls[0][0].config?.cli_routes).toEqual({ responses: { enabled: true }, messages: { enabled: true } });
  });

  it('retains route drafts after a failed save and disables their controls while saving', async () => {
    const pending = Promise.withResolvers<boolean>();
    const onSave = vi.fn(async (_config: ProviderConfig) => pending.promise);
    renderForm(provider, onSave);
    await chooseCliOption('Codex (Responses)', 'Route', 'Enabled');
    fireEvent.click(screen.getByRole('button', { name: 'Save connection configuration' }));
    expect(cliRoute('Codex (Responses)').getByRole('combobox', { name: 'Route' })).toHaveProperty('disabled', true);
    expect(cliRoute('Codex (Responses)').getByLabelText('CLI base URL (optional)')).toHaveProperty('disabled', true);
    expect(cliRoute('Codex (Responses)').getByRole('combobox', { name: 'CLI auth header style' })).toHaveProperty('disabled', true);
    await act(async () => { pending.resolve(false); });
    expect(cliRoute('Codex (Responses)').getByRole('combobox', { name: 'Route' }).textContent).toContain('Enabled');
    expect(screen.getByRole('button', { name: 'Save connection configuration' })).toHaveProperty('disabled', false);
  });

  it('creates independent CLI declarations without changing the ordinary adapter or testing automatically', async () => {
    useProviderStore.setState({ vendorsStatus: 'loaded', vendors: [{
      vendor_id: 'custom', display_name: 'Custom', endpoints: [{
        provider_type: 'openai_compatible', display_name: 'OpenAI compatible', default_base_url: null,
      }],
    }] });
    const onSave = vi.fn(async (_config: ProviderConfig) => true);
    render(<ProviderForm provider={null} onSave={onSave} onChange={() => {}} />);
    fireEvent.change(screen.getByLabelText('Name *'), { target: { value: 'New gateway' } });
    fireEvent.change(screen.getByLabelText('Base URL *'), { target: { value: 'https://ordinary.invalid/v1' } });
    fireEvent.change(screen.getByLabelText('API Key *'), { target: { value: 'new-key' } });
    await chooseCliOption('Codex (Responses)', 'Route', 'Disabled');
    await chooseCliOption('Claude Code (Messages)', 'Route', 'Enabled');
    expect(mockedInvoke).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Create', exact: true }));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect(onSave.mock.calls[0][0]).toMatchObject({
      provider_type: 'openai_compatible', base_url: 'https://ordinary.invalid/v1', api_key: 'new-key',
      config: { vendor_id: 'custom', cli_routes: { responses: { enabled: false }, messages: { enabled: true } } },
    });
  });
});
