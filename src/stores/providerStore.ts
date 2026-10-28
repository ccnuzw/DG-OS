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
  
  // Actions
  fetchProviders: () => Promise<void>;
  createProvider: (config: ProviderConfig, models?: Model[]) => Promise<boolean>;
  updateProvider: (id: string, config: ProviderConfig) => Promise<boolean>;
  toggleProvider: (id: string, enabled: boolean) => Promise<boolean>;
  migrateCredential: (providerId: string) => Promise<boolean>;
  deleteProvider: (id: string) => Promise<boolean>;
  testConnection: (config: ProviderConfig) => Promise<boolean>;
  fetchModels: (providerId: string) => Promise<void>;
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

  fetchProviders: async () => {
    set({ loading: true, error: null });
    try {
      const response = await invoke<CommandResponse<Provider[]>>('list_providers', {
      });
      
      if (response.success && response.data) {
        set({ providers: response.data, loading: false });
      } else {
        set({ 
          error: response.error?.message || 'Failed to fetch providers',
          loading: false 
        });
      }
    } catch (err) {
      set({ 
        error: err instanceof Error ? err.message : 'Unknown error',
        loading: false 
      });
    }
  },

  createProvider: async (config: ProviderConfig, models?: Model[]) => {
    set({ loading: true, error: null });
    try {
      let response;
      
      // If models are provided, use create_provider_with_models
      if (models && models.length > 0) {
        response = await invoke<CommandResponse<Provider>>('create_provider_with_models', {
          config,
            models,
        });
      } else {
        // Otherwise just create provider without models
        response = await invoke<CommandResponse<Provider>>('create_provider', {
          config,
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
          loading: false 
        });
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

  updateProvider: async (id: string, config: ProviderConfig) => {
    set({ loading: true, error: null });
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
          loading: false 
        });
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

  toggleProvider: async (id: string, enabled: boolean) => {
    set({ loading: true, error: null });
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
        loading: false,
      });
      return false;
    } catch (err) {
      set({
        error: err instanceof Error ? err.message : 'Unknown error',
        loading: false,
      });
      return false;
    }
  },

  migrateCredential: async (providerId: string) => {
    set({ loading: true, error: null });
    try {
      const response = await invoke<CommandResponse<Provider>>('migrate_provider_credential', { providerId });
      if (!response.success || !response.data) {
        set({ loading: false, error: response.error?.message || 'Credential migration is pending. Unlock your OS credential store and try again.' });
        return false;
      }
      const provider = response.data;
      set(state => ({
        providers: state.providers.map(row => row.id === providerId ? provider : row),
        selectedProvider: state.selectedProvider?.id === providerId ? provider : state.selectedProvider,
        loading: false,
      }));
      return true;
    } catch {
      set({ loading: false, error: 'Credential migration is pending. Unlock your OS credential store and try again.' });
      return false;
    }
  },

  deleteProvider: async (id: string) => {
    set({ loading: true, error: null });
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
          loading: false 
        });
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
    set({ modelRequest: request, modelsProviderId: providerId, models: [], modelsLoading: true, error: null });
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
          modelsLoading: false
        });
      }
    } catch (err) {
      if (get().modelRequest !== request) return;
      set({ 
        error: err instanceof Error ? err.message : 'Unknown error',
        modelsLoading: false
      });
    }
  },

  clearModels: () => set(state => ({
    models: [], modelsProviderId: null, modelsLoading: false, modelRequest: state.modelRequest + 1,
  })),

  setSelectedProvider: (provider) => set({ selectedProvider: provider }),
  
  clearError: () => set({ error: null }),
}));
