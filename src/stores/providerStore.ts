import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';

// Types matching Rust backend
export interface Provider {
  id: string;
  name: string;
  provider_type: string;
  base_url: string;
  enabled: boolean;
  sort_order: number;
  created_at: string;
  updated_at: string;
  user_id: string;
  credential_ref?: string;
  credential_status: 'stored' | 'migration_required' | 'missing';
  config_revision: number;
  config?: Record<string, unknown>;
}

export interface ProviderConfig {
  name: string;
  provider_type: string;
  base_url: string;
  api_key: string;
  enabled: boolean;
  config?: Record<string, unknown>;
}

// Fixed ModelKind spellings (V1-FR-001 spec); `model_type` carries one of them (legacy 'chat' reads as text-generation).
export const MODEL_KINDS = [
  'text-generation',
  'embedding',
  'rerank',
  'image-generation',
  'video-generation',
  'speech-synthesis',
  'speech-transcription',
  'moderation',
] as const;
export type ModelKind = (typeof MODEL_KINDS)[number];

export type CapabilityState = 'supported' | 'unsupported' | 'unknown';
export interface ModelCapabilities {
  streaming: CapabilityState;
  vision: CapabilityState;
  tools: CapabilityState;
  reasoning: CapabilityState;
  structured_output: CapabilityState;
}

export interface Model {
  id: string;
  provider_id: string;
  model_id: string;
  model_name: string;
  model_type: string;
  context_window?: number;
  max_output_tokens?: number;
  supports_streaming: boolean;
  supports_vision: boolean;
  cached_at: string;
  kind_source?: 'protocol' | 'rule' | 'default' | 'manual';
  source?: 'remote' | 'manual';
  capabilities?: ModelCapabilities;
}

// Vendor table: owned by the host (`list_provider_vendors`); the renderer ships no vendor list or default URLs.
export interface VendorEndpoint {
  provider_type: string;
  display_name: string;
  default_base_url: string | null;
}
export interface ProviderVendor {
  vendor_id: string;
  display_name: string;
  endpoints: VendorEndpoint[];
}

function hostOf(url: string): string | null {
  try {
    return new URL(url.trim()).hostname.toLowerCase();
  } catch {
    return null;
  }
}

/**
 * The vendor a provider belongs to: `config.vendor_id` when saved, else the vendor whose official endpoint of this
 * `provider_type` has exactly the base URL's host (full host match, so `api.openai.com.evil.example` is custom),
 * else `custom`. Mirrors the host-side inference; the official hosts come from the vendor table, not from here.
 */
export function resolveVendorId(
  provider: { provider_type: string; base_url: string; config?: Record<string, unknown> },
  vendors: readonly ProviderVendor[],
): string {
  const saved = provider.config?.vendor_id;
  if (typeof saved === 'string' && saved !== '') return saved;
  const host = hostOf(provider.base_url);
  if (host) {
    for (const vendor of vendors) {
      for (const endpoint of vendor.endpoints) {
        if (endpoint.provider_type !== provider.provider_type || !endpoint.default_base_url) continue;
        if (hostOf(endpoint.default_base_url) === host) return vendor.vendor_id;
      }
    }
  }
  return 'custom';
}

interface CommandResponse<T> {
  success: boolean;
  data?: T;
  error?: {
    code: number;
    message: string;
  };
}

interface ProviderStore {
  providers: Provider[];
  selectedProvider: Provider | null;
  models: Model[];
  modelsProviderId: string | null;
  modelsLoading: boolean;
  modelRequest: number;
  loading: boolean;
  error: string | null;
  /** The raw failure of the last create/update/toggle/migrate/delete, so a page can hand it to `handleError`. */
  lastFailure: unknown;
  vendors: ProviderVendor[];
  vendorsStatus: 'idle' | 'loading' | 'loaded' | 'error';
  
  // Actions
  fetchProviders: () => Promise<void>;
  createProvider: (config: ProviderConfig, models?: Model[], probeModel?: string) => Promise<boolean>;
  updateProvider: (id: string, config: ProviderConfig) => Promise<boolean>;
  toggleProvider: (id: string, enabled: boolean) => Promise<boolean>;
  migrateCredential: (providerId: string) => Promise<boolean>;
  deleteProvider: (id: string) => Promise<boolean>;
  testConnection: (config: ProviderConfig) => Promise<boolean>;
  fetchModels: (providerId: string) => Promise<void>;
  fetchVendors: () => Promise<void>;
  addManualModel: (providerId: string, modelId: string, kind?: string) => Promise<boolean>;
  removeManualModel: (providerId: string, modelId: string) => Promise<boolean>;
  clearModels: () => void;
  setSelectedProvider: (provider: Provider | null) => void;
  clearError: () => void;
}

export const useProviderStore = create<ProviderStore>((set, get) => ({
  providers: [],
  selectedProvider: null,
  models: [],
  modelsProviderId: null,
  modelsLoading: false,
  modelRequest: 0,
  loading: false,
  error: null,
  lastFailure: null,
  vendors: [],
  vendorsStatus: 'idle',

  fetchProviders: async () => {
    set({ loading: true, error: null, lastFailure: null });
    try {
      const response = await invoke<CommandResponse<Provider[]>>('list_providers', {
      });
      
      if (response.success && response.data) {
        set({ providers: response.data, loading: false });
      } else {
        set({ 
          error: response.error?.message || 'Failed to fetch providers',
          lastFailure: response.error ?? null,
          loading: false 
        });
      }
    } catch (err) {
      set({ 
        error: err instanceof Error ? err.message : 'Unknown error',
        lastFailure: err,
        loading: false 
      });
    }
  },

  createProvider: async (config: ProviderConfig, models?: Model[], probeModel?: string) => {
    set({ loading: true, error: null, lastFailure: null });
    try {
      let response;
      
      // `probeModel` is sent only when set; the host uses it when the service has no model list endpoint.
      const probe = probeModel?.trim();
      const extra = probe ? { probeModel: probe } : {};
      // If models are provided, use create_provider_with_models
      if (models && models.length > 0) {
        response = await invoke<CommandResponse<Provider>>('create_provider_with_models', {
          config,
          models,
          ...extra,
        });
      } else {
        // Otherwise just create provider without models
        response = await invoke<CommandResponse<Provider>>('create_provider', {
          config,
          ...extra,
        });
      }
      
      if (response.success && response.data) {
        set((state) => ({
          providers: [...state.providers, response.data!],
          loading: false,
        }));
        return true;
      } else {
        set({ 
          error: response.error?.message || 'Failed to create provider',
          lastFailure: response.error ?? null,
          loading: false 
        });
        return false;
      }
    } catch (err) {
      set({ 
        error: err instanceof Error ? err.message : 'Unknown error',
        lastFailure: err,
        loading: false 
      });
      return false;
    }
  },

  updateProvider: async (id: string, config: ProviderConfig) => {
    set({ loading: true, error: null, lastFailure: null });
    try {
      const response = await invoke<CommandResponse<Provider>>('update_provider', {
        id,
        config,
      });
      
      if (response.success && response.data) {
        set((state) => ({
          providers: state.providers.map((p) =>
            p.id === id ? response.data! : p
          ),
          selectedProvider: state.selectedProvider?.id === id ? response.data! : state.selectedProvider,
          ...(state.modelsProviderId === id ? {
            models: [], modelsProviderId: null, modelsLoading: false, modelRequest: state.modelRequest + 1,
          } : {}),
          loading: false,
        }));
        return true;
      } else {
        set({ 
          error: response.error?.message || 'Failed to update provider',
          lastFailure: response.error ?? null,
          loading: false 
        });
        return false;
      }
    } catch (err) {
      set({ 
        error: err instanceof Error ? err.message : 'Unknown error',
        lastFailure: err,
        loading: false 
      });
      return false;
    }
  },

  toggleProvider: async (id: string, enabled: boolean) => {
    set({ loading: true, error: null, lastFailure: null });
    try {
      const response = await invoke<CommandResponse<Provider>>('toggle_provider', {
        id,
        enabled,
      });

      if (response.success && response.data) {
        set((state) => ({
          providers: state.providers.map((provider) =>
            provider.id === id ? response.data! : provider
          ),
          selectedProvider:
            state.selectedProvider?.id === id ? response.data! : state.selectedProvider,
          loading: false,
        }));
        return true;
      }

      set({
        error: response.error?.message || 'Failed to update provider status',
        lastFailure: response.error ?? null,
        loading: false,
      });
      return false;
    } catch (err) {
      set({
        error: err instanceof Error ? err.message : 'Unknown error',
        lastFailure: err,
        loading: false,
      });
      return false;
    }
  },

  migrateCredential: async (providerId: string) => {
    set({ loading: true, error: null, lastFailure: null });
    try {
      const response = await invoke<CommandResponse<Provider>>('migrate_provider_credential', { providerId });
      if (!response.success || !response.data) {
        set({ loading: false, error: response.error?.message || 'Credential migration is pending. Unlock your OS credential store and try again.', lastFailure: response.error ?? null });
        return false;
      }
      const provider = response.data;
      set(state => ({
        providers: state.providers.map(row => row.id === providerId ? provider : row),
        selectedProvider: state.selectedProvider?.id === providerId ? provider : state.selectedProvider,
        loading: false,
      }));
      return true;
    } catch (err) {
      set({ loading: false, error: 'Credential migration is pending. Unlock your OS credential store and try again.', lastFailure: err });
      return false;
    }
  },

  deleteProvider: async (id: string) => {
    set({ loading: true, error: null, lastFailure: null });
    try {
      const response = await invoke<CommandResponse<void>>('delete_provider', {
        id,
      });
      
      if (response.success) {
        set((state) => ({
          providers: state.providers.filter((p) => p.id !== id),
          selectedProvider:
            state.selectedProvider?.id === id ? null : state.selectedProvider,
          ...(state.modelsProviderId === id ? {
            models: [], modelsProviderId: null, modelsLoading: false, modelRequest: state.modelRequest + 1,
          } : {}),
          loading: false,
        }));
        return true;
      } else {
        set({ 
          error: response.error?.message || 'Failed to delete provider',
          lastFailure: response.error ?? null,
          loading: false 
        });
        return false;
      }
    } catch (err) {
      set({ 
        error: err instanceof Error ? err.message : 'Unknown error',
        lastFailure: err,
        loading: false 
      });
      return false;
    }
  },

  testConnection: async (config: ProviderConfig) => {
    set({ loading: true, error: null });
    try {
      const response = await invoke<CommandResponse<boolean>>('test_provider_connection', {
        config,
      });
      
      set({ loading: false });
      
      if (response.success && response.data) {
        return true;
      } else {
        set({ error: response.error?.message || 'Connection test failed' });
        return false;
      }
    } catch (err) {
      set({ 
        error: err instanceof Error ? err.message : 'Unknown error',
        loading: false 
      });
      return false;
    }
  },

  fetchModels: async (providerId: string) => {
    const request = get().modelRequest + 1;
    set({ modelRequest: request, modelsProviderId: providerId, models: [], modelsLoading: true, error: null, lastFailure: null });
    try {
      const response = await invoke<CommandResponse<Model[]>>('list_models', {
        providerId,
      });
      if (get().modelRequest !== request) return;
      
      if (response.success && response.data) {
        set({ models: response.data, modelsLoading: false });
      } else {
        set({ 
          error: response.error?.message || 'Failed to fetch models',
          lastFailure: response.error ?? null,
          modelsLoading: false
        });
      }
    } catch (err) {
      if (get().modelRequest !== request) return;
      set({ 
        error: err instanceof Error ? err.message : 'Unknown error',
        lastFailure: err,
        modelsLoading: false
      });
    }
  },

  fetchVendors: async () => {
    if (get().vendorsStatus === 'loading') return;
    set({ vendorsStatus: 'loading' });
    try {
      const response = await invoke<CommandResponse<ProviderVendor[]>>('list_provider_vendors', {});
      if (response.success && Array.isArray(response.data)) {
        set({ vendors: response.data, vendorsStatus: 'loaded' });
      } else {
        set({ vendorsStatus: 'error' });
      }
    } catch {
      set({ vendorsStatus: 'error' });
    }
  },

  addManualModel: async (providerId: string, modelId: string, kind?: string) => {
    set({ lastFailure: null });
    try {
      const response = await invoke<CommandResponse<Model>>('add_manual_model', {
        providerId,
        modelId,
        ...(kind ? { kind } : {}),
      });
      if (!response.success) {
        set({ lastFailure: response.error ?? null });
        return false;
      }
      const added = response.data;
      if (added && typeof added.model_id === 'string' && get().modelsProviderId === providerId) {
        set((state) => ({
          models: [...state.models.filter((row) => row.model_id !== added.model_id), added],
        }));
      } else {
        await get().fetchModels(providerId);
      }
      return true;
    } catch (err) {
      set({ lastFailure: err });
      return false;
    }
  },

  removeManualModel: async (providerId: string, modelId: string) => {
    set({ lastFailure: null });
    try {
      const response = await invoke<CommandResponse<null>>('remove_manual_model', { providerId, modelId });
      if (!response.success) {
        set({ lastFailure: response.error ?? null });
        return false;
      }
      if (get().modelsProviderId === providerId) {
        set((state) => ({ models: state.models.filter((row) => row.model_id !== modelId) }));
      } else {
        await get().fetchModels(providerId);
      }
      return true;
    } catch (err) {
      set({ lastFailure: err });
      return false;
    }
  },

  clearModels: () => set(state => ({
    models: [], modelsProviderId: null, modelsLoading: false, modelRequest: state.modelRequest + 1,
  })),

  setSelectedProvider: (provider) => set({ selectedProvider: provider }),
  
  clearError: () => set({ error: null }),
}));
