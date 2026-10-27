import { useState } from 'react';
import { ProviderConfig } from '../stores/providerStore';

interface ProviderFormProps {
  initialData?: ProviderConfig;
  onSubmit: (config: ProviderConfig) => Promise<boolean>;
  onCancel: () => void;
  onTest?: (config: ProviderConfig) => Promise<boolean>;
  loading?: boolean;
}

export default function ProviderForm({
  initialData,
  onSubmit,
  onCancel,
  onTest,
  loading = false,
}: ProviderFormProps) {
  const [formData, setFormData] = useState<ProviderConfig>(
    initialData || {
      name: '',
      provider_type: 'openai_compatible',
      base_url: 'https://api.openai.com/v1',
      api_key: '',
    }
  );
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<'success' | 'error' | null>(null);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const success = await onSubmit(formData);
    if (success) {
      // Form will be closed by parent
    }
  };

  const handleTest = async () => {
    if (!onTest) return;
    
    setTesting(true);
    setTestResult(null);
    
    const success = await onTest(formData);
    setTestResult(success ? 'success' : 'error');
    setTesting(false);
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
          onChange={(e) => setFormData({ ...formData, api_key: e.target.value })}
          placeholder="sk-..."
          required
          disabled={loading}
        />
        <small>Your API key will be encrypted before storage</small>
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

      <div className="form-actions">
        <button
          type="button"
          onClick={onCancel}
          className="btn-secondary"
          disabled={loading || testing}
        >
          Cancel
        </button>
        
        {onTest && (
          <button
            type="button"
            onClick={handleTest}
            className="btn-secondary"
            disabled={loading || testing || !formData.api_key}
          >
            {testing ? 'Testing...' : 'Test Connection'}
          </button>
        )}
        
        <button
          type="submit"
          className="btn-primary"
          disabled={loading || testing}
        >
          {loading ? 'Saving...' : initialData ? 'Update' : 'Create'}
        </button>
      </div>
    </form>
  );
}
