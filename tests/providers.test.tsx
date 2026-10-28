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
  credential_status: 'stored' as const,
  config_revision: 0,
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

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

describe('Provider model cache lifecycle', () => {
  it('publishes only the latest requested provider models', async () => {
    const first = deferred<unknown>();
    const second = deferred<unknown>();
    mockedInvoke.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const oldRequest = useProviderStore.getState().fetchModels('first');
    const latestRequest = useProviderStore.getState().fetchModels('second');
    second.resolve({ success: true, data: [{ id: 'second-model', provider_id: 'second' }] });
    await latestRequest;
    first.resolve({ success: true, data: [{ id: 'first-model', provider_id: 'first' }] });
    await oldRequest;
    expect(useProviderStore.getState().modelsProviderId).toBe('second');
    expect(useProviderStore.getState().models.map(row => row.id)).toEqual(['second-model']);
    expect(useProviderStore.getState().modelsLoading).toBe(false);
  });

  it('clears old models on selection and ignores errors after leaving', async () => {
    const request = deferred<unknown>();
    mockedInvoke.mockReturnValueOnce(request.promise);
    const fetching = useProviderStore.getState().fetchModels('first');
    expect(useProviderStore.getState().models).toEqual([]);
    useProviderStore.getState().clearModels();
    request.reject(new Error('late fixture error'));
    await fetching;
    expect(useProviderStore.getState().modelsProviderId).toBeNull();
    expect(useProviderStore.getState().error).toBeNull();
    expect(useProviderStore.getState().modelsLoading).toBe(false);
  });

  it('invalidates a pending cache read when the provider is saved', async () => {
    const request = deferred<unknown>();
    mockedInvoke.mockReturnValueOnce(request.promise).mockResolvedValueOnce({
      success: true, data: { ...provider, config_revision: 1 },
    });
    const fetching = useProviderStore.getState().fetchModels(provider.id);
    await useProviderStore.getState().updateProvider(provider.id, {
      ...provider, api_key: '',
    });
    request.resolve({ success: true, data: [{ id: 'old-cache' }] });
    await fetching;
    expect(useProviderStore.getState().selectedProvider?.config_revision).toBe(1);
    expect(useProviderStore.getState().models).toEqual([]);
  });
});
