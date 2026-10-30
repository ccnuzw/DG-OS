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

describe('Provider creation with a probe model', () => {
  const config = { name: 'New', provider_type: 'openai_compatible', base_url: 'https://svc.example/v1', api_key: 'synthetic-key', enabled: true };
  const model = {
    id: 'm1', provider_id: 'preview', model_id: 'm-one', model_name: 'm-one', model_type: 'text-generation',
    supports_streaming: true, supports_vision: false, cached_at: '2026-10-30T00:00:00Z',
  };

  it('sends probeModel (trimmed) with create_provider only when it is set', async () => {
    mockedInvoke.mockResolvedValue({ success: true, data: provider });
    await useProviderStore.getState().createProvider(config, [], '  probe-model  ');
    expect(mockedInvoke).toHaveBeenLastCalledWith('create_provider', { config, probeModel: 'probe-model' });

    await useProviderStore.getState().createProvider(config, [], '   ');
    expect(mockedInvoke).toHaveBeenLastCalledWith('create_provider', { config });
    await useProviderStore.getState().createProvider(config);
    expect(mockedInvoke).toHaveBeenLastCalledWith('create_provider', { config });
  });

  it('sends probeModel with create_provider_with_models only when it is set', async () => {
    mockedInvoke.mockResolvedValue({ success: true, data: provider });
    await useProviderStore.getState().createProvider(config, [model], 'probe-model');
    expect(mockedInvoke).toHaveBeenLastCalledWith('create_provider_with_models', { config, models: [model], probeModel: 'probe-model' });
    await useProviderStore.getState().createProvider(config, [model]);
    expect(mockedInvoke).toHaveBeenLastCalledWith('create_provider_with_models', { config, models: [model] });
  });
});

describe('Provider vendors', () => {
  it('loads the vendor table from the host', async () => {
    const vendors = [{ vendor_id: 'acme', display_name: 'Acme', endpoints: [{ provider_type: 'openai_compatible', display_name: 'OpenAI compatible', default_base_url: 'https://acme.example/v1' }] }];
    mockedInvoke.mockResolvedValue({ success: true, data: vendors });
    await useProviderStore.getState().fetchVendors();
    expect(mockedInvoke).toHaveBeenCalledWith('list_provider_vendors', {});
    expect(useProviderStore.getState().vendors).toEqual(vendors);
    expect(useProviderStore.getState().vendorsStatus).toBe('loaded');
  });

  it('marks the table as failed without touching the general error', async () => {
    mockedInvoke.mockRejectedValue(new Error('host down'));
    await useProviderStore.getState().fetchVendors();
    expect(useProviderStore.getState().vendorsStatus).toBe('error');
    expect(useProviderStore.getState().vendors).toEqual([]);
    expect(useProviderStore.getState().error).toBeNull();
  });
});

describe('Manual models', () => {
  const manual = {
    id: 'mm1', provider_id: provider.id, model_id: 'my-model', model_name: 'my-model', model_type: 'embedding',
    supports_streaming: false, supports_vision: false, cached_at: '2026-10-30T00:00:00Z', source: 'manual' as const,
  };

  it('adds a manual model, omitting kind when it is not given, and shows the returned row', async () => {
    useProviderStore.setState({ models: [], modelsProviderId: provider.id });
    mockedInvoke.mockResolvedValue({ success: true, data: manual });
    expect(await useProviderStore.getState().addManualModel(provider.id, 'my-model')).toBe(true);
    expect(mockedInvoke).toHaveBeenLastCalledWith('add_manual_model', { providerId: provider.id, modelId: 'my-model' });
    expect(await useProviderStore.getState().addManualModel(provider.id, 'my-model', 'embedding')).toBe(true);
    expect(mockedInvoke).toHaveBeenLastCalledWith('add_manual_model', { providerId: provider.id, modelId: 'my-model', kind: 'embedding' });
    expect(useProviderStore.getState().models).toEqual([manual]);
  });

  it('keeps the raw failure of a rejected add for the page to report', async () => {
    mockedInvoke.mockResolvedValue({ success: false, error: { code: 1001020, message: 'bad kind' } });
    expect(await useProviderStore.getState().addManualModel(provider.id, 'x', 'nonsense')).toBe(false);
    expect(useProviderStore.getState().lastFailure).toEqual({ code: 1001020, message: 'bad kind' });
  });

  it('removes a manual model by provider and model id', async () => {
    useProviderStore.setState({ models: [manual], modelsProviderId: provider.id });
    mockedInvoke.mockResolvedValue({ success: true, data: null });
    expect(await useProviderStore.getState().removeManualModel(provider.id, 'my-model')).toBe(true);
    expect(mockedInvoke).toHaveBeenCalledWith('remove_manual_model', { providerId: provider.id, modelId: 'my-model' });
    expect(useProviderStore.getState().models).toEqual([]);
  });
});
