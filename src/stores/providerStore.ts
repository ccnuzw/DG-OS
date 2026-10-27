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
}

export interface ProviderConfig {
  name: string;
  provider_type: string;
  base_url: string;
  api_key: string;
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
  loading: boolean;
  error: string | null;
  
  // Actions
  fetchProviders: (userId: string) => Promise<void>;
  createProvider: (config: ProviderConfig, userId: string, models?: Model[]) => Promise<boolean>;
  updateProvider: (id: string, config: ProviderConfig) => Promise<boolean>;
  deleteProvider: (id: string) => Promise<boolean>;
  testConnection: (config: ProviderConfig) => Promise<boolean>;
  fetchModels: (providerId: string) => Promise<void>;
  setSelectedProvider: (provider: Provider | null) => void;
  clearError: () => void;
}

export const useProviderStore = create<ProviderStore>((set) => ({
  providers: [],
  selectedProvider: null,
  models: [],
  loading: false,
  error: null,

  fetchProviders: async (userId: string) => {
    set({ loading: true, error: null });
    try {
      const response = await invoke<CommandResponse<Provider[]>>('list_providers', {
        userId,
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

  createProvider: async (config: ProviderConfig, userId: string, models?: Model[]) => {
    set({ loading: true, error: null });
    try {
      let response;
      
      // If models are provided, use create_provider_with_models
      if (models && models.length > 0) {
        response = await invoke<CommandResponse<Provider>>('create_provider_with_models', {
          config,
          userId,
          models,
        });
      } else {
        // Otherwise just create provider without models
        response = await invoke<CommandResponse<Provider>>('create_provider', {
          config,
          userId,
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
    set({ loading: true, error: null });
    try {
      const response = await invoke<CommandResponse<Model[]>>('list_models', {
        providerId,
      });
      
      if (response.success && response.data) {
        set({ models: response.data, loading: false });
      } else {
        set({ 
          error: response.error?.message || 'Failed to fetch models',
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

  setSelectedProvider: (provider) => set({ selectedProvider: provider }),
  
  clearError: () => set({ error: null }),
}));
