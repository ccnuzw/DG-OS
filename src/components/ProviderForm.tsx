import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ProviderConfig, Model, Provider } from '../stores/providerStore';

interface ProviderFormProps {
  provider: Provider | null;  // null = 新建模式
  initialModels?: Model[];  // 编辑模式时的初始模型列表
  onSave: (config: ProviderConfig, models: Model[]) => Promise<boolean>;
  onDelete?: () => Promise<boolean>;
  onChange: () => void;
  loading?: boolean;
}

interface CommandResponse<T> {
  success: boolean;
  data?: T;
  error?: { code: number; message: string };
}

export default function ProviderForm({
  provider,
  initialModels = [],
  onSave,
  onDelete,
  onChange,
  loading = false,
}: ProviderFormProps) {
  const isEditMode = !!provider;
  const isNewMode = !provider;

  // 表单状态
  const [formData, setFormData] = useState<ProviderConfig>(
    provider
      ? {
          name: provider.name,
          provider_type: provider.provider_type,
          base_url: provider.base_url,
          api_key: '', // 编辑模式下留空
          enabled: provider.enabled,
          config: provider.config,
        }
      : {
          name: '',
          provider_type: 'openai_compatible',
          base_url: 'https://api.openai.com/v1',
          api_key: '',
          enabled: true,
          config: undefined,
        }
  );

  // 操作状态
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<'success' | 'error' | null>(null);
  const [testMessage, setTestMessage] = useState<string>('');
  
  const [fetchingModels, setFetchingModels] = useState(false);
  const [models, setModels] = useState<Model[]>([]);
  const [modelsError, setModelsError] = useState<string | null>(null);

  const [saving, setSaving] = useState(false);

  // 当 provider 变化时重置表单
  useEffect(() => {
    if (provider) {
      setFormData({
        name: provider.name,
        provider_type: provider.provider_type,
        base_url: provider.base_url,
        api_key: '',
        enabled: provider.enabled,
        config: provider.config,
      });
      // 编辑模式：加载初始模型列表
      setModels(initialModels);
    } else {
      setFormData({
        name: '',
        provider_type: 'openai_compatible',
        base_url: 'https://api.openai.com/v1',
        api_key: '',
        enabled: true,
        config: undefined,
      });
      // 新建模式：清空模型列表
      setModels([]);
    }
    setTestResult(null);
    setModelsError(null);
  }, [provider, initialModels]);

  // 表单变化处理
  const handleChange = (field: keyof ProviderConfig, value: any) => {
    setFormData({ ...formData, [field]: value });
    onChange();
  };

  // 测试连接
  const handleTestConnection = async () => {
    setTesting(true);
    setTestResult(null);
    setTestMessage('');

    try {
      let response: CommandResponse<boolean>;

      if (isEditMode && provider) {
        // 编辑模式：如果没有输入新的 API Key，使用数据库中的
        if (!formData.api_key) {
          response = await invoke<CommandResponse<boolean>>('test_provider_by_id', {
            providerId: provider.id,
          });
        } else {
          // 输入了新的 API Key，使用新的配置测试
          response = await invoke<CommandResponse<boolean>>('test_provider_connection', {
            config: formData,
          });
        }
      } else {
        // 新建模式：必须有 API Key
        if (!formData.api_key) {
          setTestResult('error');
          setTestMessage('Please enter an API Key');
          setTesting(false);
          return;
        }
        response = await invoke<CommandResponse<boolean>>('test_provider_connection', {
          config: formData,
        });
      }

      if (response.success && response.data) {
        setTestResult('success');
        setTestMessage('Connection successful');
      } else {
        setTestResult('error');
        setTestMessage(response.error?.message || 'Connection failed');
      }
    } catch (err) {
      setTestResult('error');
      setTestMessage(err instanceof Error ? err.message : 'Unknown error');
    } finally {
      setTesting(false);
    }
  };

  // 拉取/刷新模型
  const handleFetchModels = async () => {
    setFetchingModels(true);
    setModelsError(null);

    try {
      if (isEditMode && provider) {
        // 编辑模式：调用 refresh_models（会保存到数据库）
        const response = await invoke<CommandResponse<Model[]>>('refresh_models', {
          providerId: provider.id,
        });

        if (response.success && response.data) {
          setModels(response.data);
        } else {
          setModelsError(response.error?.message || 'Failed to fetch models');
        }
      } else {
        // 新建模式：通过后端请求模型，避免 WebView fetch 的跨域/网络错误。
        if (!formData.api_key) {
          setModelsError('Please enter an API Key first');
          setFetchingModels(false);
          return;
        }

        const response = await invoke<CommandResponse<Model[]>>('list_models_for_config', {
          config: formData,
        });

        if (response.success && response.data) {
          setModels(response.data.map((model) => ({
            ...model,
            provider_id: 'preview',
          })));
        } else {
          setModelsError(response.error?.message || 'Failed to fetch models');
        }
      }
    } catch (err) {
      setModelsError(err instanceof Error ? err.message : 'Unknown error');
    } finally {
      setFetchingModels(false);
    }
  };

  // 保存
  const handleSave = async () => {
    setSaving(true);
    const success = await onSave(formData, models);
    setSaving(false);
    
    if (success && isNewMode) {
      // 新建成功后重置表单
      setFormData({
        name: '',
        provider_type: 'openai_compatible',
        base_url: 'https://api.openai.com/v1',
        api_key: '',
        enabled: true,
        config: undefined,
      });
      setModels([]);
      setTestResult(null);
    }
  };

  // 删除
  const handleDelete = async () => {
    if (onDelete) {
      await onDelete();
    }
  };

  return (
    <div className="provider-form">
      {/* 标题 */}
      <header className="form-header">
        <h2>{isNewMode ? 'New Provider' : provider.name}</h2>
        {isEditMode && (
          <span className="provider-id">ID: {provider.id.slice(0, 8)}...</span>
        )}
      </header>

      {/* 配置区 */}
      <section className="config-section">
        <h3>Configuration</h3>

        <div className="form-group">
          <label htmlFor="name">
            Name <span className="required">*</span>
          </label>
          <input
            id="name"
            type="text"
            value={formData.name}
            onChange={(e) => handleChange('name', e.target.value)}
            placeholder="My Provider"
            disabled={loading || saving}
          />
        </div>

        <div className="form-group">
          <label htmlFor="provider_type">Provider Type</label>
          <select
            id="provider_type"
            value={formData.provider_type}
            onChange={(e) => handleChange('provider_type', e.target.value)}
            disabled={loading || saving}
          >
            <option value="openai_compatible">OpenAI Compatible</option>
          </select>
          <small>Currently only supports OpenAI compatible APIs</small>
        </div>

        <div className="form-group">
          <label htmlFor="base_url">
            Base URL <span className="required">*</span>
          </label>
          <input
            id="base_url"
            type="url"
            value={formData.base_url}
            onChange={(e) => handleChange('base_url', e.target.value)}
            placeholder="https://api.openai.com/v1"
            disabled={loading || saving}
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
            onChange={(e) => handleChange('api_key', e.target.value)}
            placeholder={isEditMode ? 'Leave empty to keep current' : 'sk-...'}
            disabled={loading || saving}
          />
          {isEditMode ? (
            <small>Leave empty to keep current API key, or enter a new one</small>
          ) : (
            <small>Your API key will be encrypted before storage</small>
          )}
        </div>

        <div className="form-group">
          <label className="checkbox-label">
            <input
              type="checkbox"
              checked={formData.enabled}
              onChange={(e) => handleChange('enabled', e.target.checked)}
              disabled={loading || saving}
            />
            <span>Enabled</span>
          </label>
          <small>Disabled providers cannot be used for API calls</small>
        </div>
      </section>

      {/* 操作区 */}
      <section className="actions-section">
        <h3>Actions</h3>

        <div className="action-buttons">
          <button
            onClick={handleTestConnection}
            disabled={testing || saving || (!isEditMode && !formData.api_key)}
            className="btn-secondary"
          >
            {testing ? 'Testing...' : '🔌 Test Connection'}
          </button>

          <button
            onClick={handleFetchModels}
            disabled={fetchingModels || saving || (!isEditMode && !formData.api_key)}
            className="btn-secondary"
          >
            {fetchingModels ? 'Fetching...' : '📥 Refresh Models'}
          </button>
        </div>

        {/* 测试结果 */}
        {testResult && (
          <div className={`test-result ${testResult}`}>
            {testResult === 'success' ? '✓' : '✗'} {testMessage}
          </div>
        )}

        {/* 模型错误 */}
        {modelsError && (
          <div className="test-result error">✗ {modelsError}</div>
        )}
      </section>

      {/* 模型列表 */}
      {models.length > 0 && (
        <section className="models-section">
          <h3>Models ({models.length})</h3>
          <div className="models-list">
            {models.map((model) => (
              <div key={model.id} className="model-card">
                <h4>{model.model_name}</h4>
                <div className="model-id">{model.model_id}</div>
                <div className="model-meta">
                  <span className="badge">{model.model_type}</span>
                  {model.supports_streaming && (
                    <span className="badge badge-info">Streaming</span>
                  )}
                  {model.supports_vision && (
                    <span className="badge badge-info">Vision</span>
                  )}
                </div>
                {model.context_window && (
                  <div className="model-context">
                    Context: {model.context_window.toLocaleString()} tokens
                  </div>
                )}
              </div>
            ))}
          </div>
          {isNewMode && (
            <small className="models-note">
              These models will be saved when you create the provider
            </small>
          )}
          {isEditMode && (
            <small className="models-note">
              Models were updated in the database
            </small>
          )}
        </section>
      )}

      {/* 底部操作 */}
      <footer className="form-footer">
        {isEditMode && onDelete && (
          <button
            onClick={handleDelete}
            disabled={saving}
            className="btn-danger"
          >
            🗑️ Delete
          </button>
        )}
        <div className="spacer" />
        <button
          onClick={handleSave}
          disabled={saving || loading || !formData.name || !formData.base_url || (!isEditMode && !formData.api_key)}
          className="btn-primary"
        >
          {saving ? 'Saving...' : isNewMode ? '💾 Create' : '💾 Save Changes'}
        </button>
      </footer>
    </div>
  );
}
