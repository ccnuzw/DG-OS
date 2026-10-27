import { useState, useEffect } from 'react';
import { ProviderConfig } from '../stores/providerStore';
import { invoke } from '@tauri-apps/api/core';

interface ProviderFormProps {
  initialData?: ProviderConfig & { id?: string };
  onSubmit: (config: ProviderConfig) => Promise<boolean>;
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
  
  // Model fetching state
  const [fetchingModels, setFetchingModels] = useState(false);
  const [modelsFetched, setModelsFetched] = useState(false);

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
    
    const success = await onSubmit(submitData);
    if (success) {
      // Form will be closed by parent
    }
  };


  const handleFetchModels = async () => {
    if (!formData.base_url || !formData.api_key) {
      return;
    }

    setFetchingModels(true);
    setTestResult(null);
    
    try {
      // Use the test connection command to validate
      const testResponse = await invoke<CommandResponse<boolean>>('test_provider_connection', {
        config: formData,
      });
      
      if (!testResponse.success || !testResponse.data) {
        setTestResult('error');
        setFetchingModels(false);
        return;
      }

      // Connection successful
      setTestResult('success');
      setModelsFetched(true);
      
    } catch (err) {
      setTestResult('error');
    } finally {
      setFetchingModels(false);
    }
  };

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

      {modelsFetched && (
        <div className="models-preview">
          <h4>Ready to fetch models</h4>
          <p>Models will be fetched automatically after saving the provider.</p>
        </div>
      )}

      <div className="form-actions">
        <button
          type="button"
          onClick={onCancel}
          className="btn-secondary"
          disabled={loading || fetchingModels}
        >
          Cancel
        </button>
        
        <button
          type="button"
          onClick={handleFetchModels}
          className="btn-secondary"
          disabled={loading || fetchingModels || !formData.api_key}
          title="Test connection and validate API key"
        >
          {fetchingModels ? 'Testing...' : '🔌 Test & Validate'}
        </button>
        
        <button
          type="submit"
          className="btn-primary"
          disabled={loading || fetchingModels}
        >
          {loading ? 'Saving...' : initialData?.id ? 'Update' : 'Create'}
        </button>
      </div>
    </form>
  );
}
