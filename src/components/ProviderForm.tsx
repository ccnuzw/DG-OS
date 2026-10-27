import { useState, useEffect } from 'react';
import { ProviderConfig, Model } from '../stores/providerStore';
import { invoke } from '@tauri-apps/api/core';

interface ProviderFormProps {
  initialData?: ProviderConfig & { id?: string };
  onSubmit: (config: ProviderConfig, models: Model[]) => Promise<boolean>;
  onCancel: () => void;
  loading?: boolean;
}

interface CommandResponse<T> {
  success: boolean;
  data?: T;
  error?: {
    code: number;
    message: string;
  };
}

export default function ProviderForm({
  initialData,
  onSubmit,
  onCancel,
  loading = false,
}: ProviderFormProps) {
  const isEditMode = !!initialData?.id;
  const providerId = initialData?.id;
  
  const [formData, setFormData] = useState<ProviderConfig>(
    initialData || {
      name: '',
      provider_type: 'openai_compatible',
      base_url: 'https://api.openai.com/v1',
      api_key: '',
    }
  );
  
  // Track if API key has been modified
  const [apiKeyModified, setApiKeyModified] = useState(false);
  const [apiKeyPlaceholder, setApiKeyPlaceholder] = useState('');
  
  const [testResult, setTestResult] = useState<'success' | 'error' | null>(null);
  
  // Connection test state
  const [testing, setTesting] = useState(false);
  
  // Model fetching state
  const [fetchingModels, setFetchingModels] = useState(false);
  const [previewModels, setPreviewModels] = useState<Model[]>([]);
  const [modelsFetchError, setModelsFetchError] = useState<string | null>(null);

  useEffect(() => {
    if (isEditMode && initialData) {
      // In edit mode, show masked API key placeholder
      setApiKeyPlaceholder('sk-****...****');
      // Don't set the actual api_key in formData - keep it empty
      setFormData({
        ...initialData,
        api_key: '', // Empty for security
      });
    }
  }, []);

  const handleApiKeyChange = (value: string) => {
    setApiKeyModified(true);
    setFormData({ ...formData, api_key: value });
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    
    // If in edit mode and API key wasn't modified, send empty string
    // Backend should keep the original API key
    const submitData = {
      ...formData,
      api_key: isEditMode && !apiKeyModified ? '' : formData.api_key,
    };
    
    // Pass the fetched models along with the config
    const success = await onSubmit(submitData, previewModels);
    if (success) {
      // Form will be closed by parent
    }
  };

  const handleTestConnection = async () => {
    // In edit mode, we can test even without API key (backend has it)
    // In create mode, we need the API key
    if (!isEditMode && !formData.api_key) {
      return;
    }

    setTesting(true);
    setTestResult(null);
    
    try {
      const testResponse = await invoke<CommandResponse<boolean>>('test_provider_connection', {
        config: formData,
      });
      
      if (testResponse.success && testResponse.data) {
        setTestResult('success');
      } else {
        setTestResult('error');
      }
    } catch (err) {
      setTestResult('error');
    } finally {
      setTesting(false);
    }
  };

  const handleFetchModels = async () => {
    setFetchingModels(true);
    setModelsFetchError(null);
    setPreviewModels([]);
    
    try {
      if (isEditMode && providerId) {
        // Edit mode: use backend refresh_models (has stored API key)
        const response = await invoke<CommandResponse<Model[]>>('refresh_models', {
          providerId: providerId,
        });
        
        if (response.success && response.data) {
          setPreviewModels(response.data);
        } else {
          setModelsFetchError(response.error?.message || 'Failed to fetch models');
        }
      } else {
        // Create mode: fetch directly using form API key
        if (!formData.api_key) {
          setModelsFetchError('Please enter an API key first');
          setFetchingModels(false);
          return;
        }

        // First test connection
        const testResponse = await invoke<CommandResponse<boolean>>('test_provider_connection', {
          config: formData,
        });
        
        if (!testResponse.success || !testResponse.data) {
          setModelsFetchError('Connection test failed. Please check your configuration.');
          setFetchingModels(false);
          return;
        }

        // Then fetch models by calling the OpenAI compatible /v1/models endpoint
        const response = await fetch(`${formData.base_url}/models`, {
          headers: {
            'Authorization': `Bearer ${formData.api_key}`,
            'Content-Type': 'application/json',
          },
        });

        if (!response.ok) {
          setModelsFetchError(`Failed to fetch models: ${response.statusText}`);
          setFetchingModels(false);
          return;
        }

        const data: { data?: unknown[] } = await response.json();
        
        // OpenAI API returns { data: [...models], object: "list" }
        if (data.data && Array.isArray(data.data)) {
          const models: Model[] = data.data.map((model: unknown) => {
            const modelObj = model as Record<string, unknown>;
            return {
              id: crypto.randomUUID(),
              provider_id: 'preview',
              model_id: typeof modelObj.id === 'string' ? modelObj.id : 'unknown',
              model_name: typeof modelObj.id === 'string' ? modelObj.id : 'unknown',
              model_type: typeof modelObj.object === 'string' ? modelObj.object : 'model',
              context_window: typeof modelObj.context_window === 'number' ? modelObj.context_window : undefined,
              max_output_tokens: typeof modelObj.max_output_tokens === 'number' ? modelObj.max_output_tokens : undefined,
              supports_streaming: true,
              supports_vision: typeof modelObj.id === 'string' && (modelObj.id.includes('vision') || modelObj.id.includes('gpt-4')),
              cached_at: new Date().toISOString(),
            };
          });
          
          setPreviewModels(models);
        } else {
          setModelsFetchError('Invalid response format from API');
        }
      }
      
    } catch (err) {
      setModelsFetchError(err instanceof Error ? err.message : 'Unknown error');
    } finally {
      setFetchingModels(false);
    }
  };

  // Can test/fetch if:
  // - Edit mode: always (backend has API key)
  // - Create mode: only if API key is filled
  const canTestOrFetch = isEditMode || !!formData.api_key;

  return (
    <form onSubmit={handleSubmit} className="provider-form">
      <div className="form-group">
        <label htmlFor="name">
          Provider Name <span className="required">*</span>
        </label>
        <input
          id="name"
          type="text"
          value={formData.name}
          onChange={(e) => setFormData({ ...formData, name: e.target.value })}
          placeholder="My OpenAI Provider"
          required
          disabled={loading}
        />
      </div>

      <div className="form-group">
        <label htmlFor="provider_type">
          Provider Type <span className="required">*</span>
        </label>
        <select
          id="provider_type"
          value={formData.provider_type}
          onChange={(e) =>
            setFormData({ ...formData, provider_type: e.target.value })
          }
          required
          disabled={loading}
        >
          <option value="openai_compatible">OpenAI Compatible</option>
        </select>
        <small>V1 only supports OpenAI compatible APIs</small>
      </div>

      <div className="form-group">
        <label htmlFor="base_url">
          Base URL <span className="required">*</span>
        </label>
        <input
          id="base_url"
          type="url"
          value={formData.base_url}
          onChange={(e) => setFormData({ ...formData, base_url: e.target.value })}
          placeholder="https://api.openai.com/v1"
          required
          disabled={loading}
        />
        <small>The base URL for the API endpoint</small>
      </div>

      <div className="form-group">
        <label htmlFor="api_key">
          API Key <span className="required">*</span>
        </label>
        <input
          id="api_key"
          type="password"
          value={formData.api_key}
          onChange={(e) => handleApiKeyChange(e.target.value)}
          placeholder={isEditMode ? apiKeyPlaceholder : 'sk-...'}
          required={!isEditMode}
          disabled={loading}
          onFocus={(e) => {
            if (isEditMode && !apiKeyModified) {
              e.target.placeholder = 'Enter new API key or leave empty to keep current';
            }
          }}
        />
        <small>
          {isEditMode && !apiKeyModified
            ? 'Leave empty to keep current API key, or enter a new one'
            : 'Your API key will be encrypted before storage'}
        </small>
      </div>

      {testResult && (
        <div className={`test-result ${testResult}`}>
          {testResult === 'success' ? (
            <>✓ Connection test successful!</>
          ) : (
            <>✗ Connection test failed. Please check your configuration.</>
          )}
        </div>
      )}

      {previewModels.length > 0 && (
        <div className="models-preview">
          <h4>Available Models ({previewModels.length})</h4>
          <div className="models-preview-list">
            {previewModels.map((model) => (
              <div key={model.id} className="model-preview-item">
                <div className="model-preview-name">{model.model_name}</div>
                <div className="model-preview-id">{model.model_id}</div>
                {model.context_window && (
                  <div className="model-preview-meta">
                    Context: {model.context_window.toLocaleString()} tokens
                  </div>
                )}
              </div>
            ))}
          </div>
          <small className="models-preview-note">
            {isEditMode 
              ? 'These models are already saved in the database'
              : 'These models will be saved when you create the provider'}
          </small>
        </div>
      )}

      {modelsFetchError && (
        <div className="test-result error">
          ✗ {modelsFetchError}
        </div>
      )}

      <div className="form-actions">
        <button
          type="button"
          onClick={onCancel}
          className="btn-secondary"
          disabled={loading || testing || fetchingModels}
        >
          Cancel
        </button>
        
        <button
          type="button"
          onClick={handleTestConnection}
          className="btn-secondary"
          disabled={loading || testing || fetchingModels || !canTestOrFetch}
          title={isEditMode ? "Test connection using stored API key" : "Test API connection"}
        >
          {testing ? 'Testing...' : '🔌 Test Connection'}
        </button>

        <button
          type="button"
          onClick={handleFetchModels}
          className="btn-secondary"
          disabled={loading || testing || fetchingModels || !canTestOrFetch}
          title={isEditMode ? "Fetch models using stored API key" : "Fetch available models from API"}
        >
          {fetchingModels ? 'Fetching...' : '📥 Fetch Models'}
        </button>
        
        <button
          type="submit"
          className="btn-primary"
          disabled={loading || testing || fetchingModels}
        >
          {loading ? 'Saving...' : initialData?.id ? 'Update' : 'Create'}
        </button>
      </div>
    </form>
  );
}
