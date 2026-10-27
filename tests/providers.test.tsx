import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { useProviderStore } from '../src/stores/providerStore';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const mockedInvoke = vi.mocked(invoke);
const initialState = useProviderStore.getState();

const provider = {
  id: 'provider-1',
  name: 'Workspace AI',
  provider_type: 'openai_compatible',
  base_url: 'https://example.com/v1',
  enabled: true,
  sort_order: 0,
  created_at: '2026-10-27T00:00:00Z',
  updated_at: '2026-10-27T00:00:00Z',
  user_id: 'default-user',
};

beforeEach(() => {
  useProviderStore.setState({
    ...initialState,
    providers: [provider],
    selectedProvider: provider,
    loading: false,
    error: null,
  }, true);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('Provider enablement', () => {
  it('uses the dedicated toggle command and updates both provider collections', async () => {
    const disabledProvider = { ...provider, enabled: false };
    mockedInvoke.mockResolvedValue({ success: true, data: disabledProvider });

    const success = await useProviderStore.getState().toggleProvider(provider.id, false);

    expect(success).toBe(true);
    expect(mockedInvoke).toHaveBeenCalledWith('toggle_provider', {
      id: provider.id,
      enabled: false,
    });
    expect(useProviderStore.getState().providers[0].enabled).toBe(false);
    expect(useProviderStore.getState().selectedProvider?.enabled).toBe(false);
    expect(useProviderStore.getState().error).toBeNull();
  });

  it('keeps the current state and reports the backend error when toggling fails', async () => {
    mockedInvoke.mockResolvedValue({
      success: false,
      error: { code: 404, message: 'Provider not found' },
    });

    const success = await useProviderStore.getState().toggleProvider(provider.id, false);

    expect(success).toBe(false);
    expect(useProviderStore.getState().providers[0].enabled).toBe(true);
    expect(useProviderStore.getState().error).toBe('Provider not found');
    expect(useProviderStore.getState().loading).toBe(false);
  });
});
