import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
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
