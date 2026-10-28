import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import ProviderManagement from '../src/pages/ProviderManagement';
import { useProviderStore } from '../src/stores/providerStore';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const mockedInvoke = vi.mocked(invoke);
const initialState = useProviderStore.getState();

beforeEach(() => {
  useProviderStore.setState(initialState, true);
  mockedInvoke.mockImplementation(async (command) => {
    if (command === 'list_providers') {
      return { success: true, data: [] };
    }
    if (command === 'list_models_for_config') {
      return {
        success: true,
        data: [{
          id: 'model-1',
          provider_id: '',
          model_id: 'gpt-test',
          model_name: 'gpt-test',
          model_type: 'chat',
          supports_streaming: true,
          supports_vision: false,
          cached_at: '2026-10-27T00:00:00Z',
        }],
      };
    }
    return { success: true, data: [] };
  });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('Provider creation entry points', () => {
  it('opens the new provider form from the empty state', async () => {
    render(<ProviderManagement />);

    expect(await screen.findByText('No Providers Yet')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '+ Create Provider' }));

    expect(await screen.findByRole('heading', { name: 'New Provider' })).toBeTruthy();
    expect(screen.getByLabelText('Name *')).toBeTruthy();
  });

  it('loads models before the new provider is saved', async () => {
    render(<ProviderManagement />);

    fireEvent.click(await screen.findByRole('button', { name: '+ Create Provider' }));
    fireEvent.change(screen.getByLabelText('API Key *'), { target: { value: 'test-key' } });
    fireEvent.click(screen.getByRole('button', { name: /Refresh Models/ }));

    expect(await screen.findByRole('heading', { name: 'gpt-test' })).toBeTruthy();
    expect(mockedInvoke).toHaveBeenCalledWith('list_models_for_config', {
      config: expect.objectContaining({ api_key: 'test-key' }),
    });
  });

  it('shows pending migration and retries without requesting a key from the renderer', async () => {
    const provider = {
      id: 'provider-pending', name: 'Local fixture', provider_type: 'openai_compatible',
      base_url: 'https://example.invalid/v1', enabled: true, sort_order: 0,
      created_at: '2026-10-28T00:00:00Z', updated_at: '2026-10-28T00:00:00Z',
      user_id: 'default-user', credential_status: 'migration_required',
    };
    mockedInvoke.mockImplementation(async command => {
      if (command === 'list_providers') return { success: true, data: [provider] };
      if (command === 'migrate_provider_credential') return { success: true, data: { ...provider, credential_status: 'stored', credential_ref: 'fixture-reference' } };
      return { success: true, data: [] };
    });
    render(<ProviderManagement />);
    fireEvent.click(await screen.findByRole('button', { name: 'Retry credential migration for Local fixture' }));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Retry credential migration for Local fixture' })).toBeNull());
    expect(mockedInvoke).toHaveBeenCalledWith('migrate_provider_credential', { providerId: 'provider-pending' });
    expect(useProviderStore.getState().providers[0].credential_status).toBe('stored');
  });
});

const providerFixture = {
  id: 'provider-fixture', name: 'Saved fixture', provider_type: 'openai_compatible',
  base_url: 'https://example.invalid/v1', enabled: true, sort_order: 0,
  created_at: '2026-10-28T00:00:00Z', updated_at: '2026-10-28T00:00:00Z',
  user_id: 'default-user', credential_status: 'stored', config_revision: 0,
};
const modelFixture = {
  id: 'model-fixture', provider_id: providerFixture.id, model_id: 'fixture-model',
  model_name: 'Fixture model', model_type: 'chat', supports_streaming: true,
  supports_vision: false, cached_at: '2026-10-28T00:00:00Z',
};

describe('Provider form lifecycle', () => {
  it('retains draft edits when the saved model list finishes loading', async () => {
    let resolveModels!: (value: unknown) => void;
    const modelResponse = new Promise(resolve => { resolveModels = resolve; });
    mockedInvoke.mockImplementation(async command => {
      if (command === 'list_providers') return { success: true, data: [providerFixture] };
      if (command === 'list_models') return modelResponse;
      return { success: true, data: [] };
    });
    render(<ProviderManagement />);
    fireEvent.click(await screen.findByRole('heading', { name: 'Saved fixture', level: 3 }));
    fireEvent.change(screen.getByLabelText('Name *'), { target: { value: 'Unsaved draft' } });
    fireEvent.change(screen.getByLabelText('API Key *'), { target: { value: 'synthetic-draft-key' } });
    await act(async () => { resolveModels({ success: true, data: [modelFixture] }); });
    expect(await screen.findByRole('heading', { name: 'Fixture model' })).toBeTruthy();
    expect((screen.getByLabelText('Name *') as HTMLInputElement).value).toBe('Unsaved draft');
    expect((screen.getByLabelText('API Key *') as HTMLInputElement).value).toBe('synthetic-draft-key');
  });

  it('clears the old cache after endpoint save and keeps the new saved baseline', async () => {
    let saved = providerFixture;
    mockedInvoke.mockImplementation(async (command, args) => {
      if (command === 'list_providers') return { success: true, data: [saved] };
      if (command === 'list_models') return { success: true, data: saved.config_revision === 0 ? [modelFixture] : [] };
      if (command === 'update_provider') {
        const config = (args as { config: typeof saved }).config;
        saved = { ...saved, base_url: config.base_url, config_revision: 1 };
        return { success: true, data: saved };
      }
      return { success: true, data: [] };
    });
    render(<ProviderManagement />);
    fireEvent.click(await screen.findByRole('heading', { name: 'Saved fixture', level: 3 }));
    expect(await screen.findByRole('heading', { name: 'Fixture model' })).toBeTruthy();
    fireEvent.change(screen.getByLabelText('Base URL *'), { target: { value: 'https://example.invalid/new' } });
    fireEvent.change(screen.getByLabelText('API Key *'), { target: { value: 'synthetic-new-key' } });
    fireEvent.click(screen.getByRole('button', { name: /Save Changes/ }));
    await waitFor(() => expect(useProviderStore.getState().providers[0].config_revision).toBe(1));
    await waitFor(() => expect(screen.queryByRole('heading', { name: 'Fixture model' })).toBeNull());
    expect((screen.getByLabelText('Base URL *') as HTMLInputElement).value).toBe(saved.base_url);
    expect((screen.getByLabelText('API Key *') as HTMLInputElement).value).toBe('');
    expect((screen.getByRole('button', { name: /Refresh Models/ }) as HTMLButtonElement).disabled).toBe(false);
  });
});
