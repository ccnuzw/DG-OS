import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button, Card, Checkbox, Input, Select, Stack, cn, extractErrorCode } from '@dgos/ui';
import { usePageError } from '../pageError';
import { MODEL_KINDS, resolveVendorId, useProviderStore } from '../stores/providerStore';
import type { ProviderConfig, ProviderVendor, Model, Provider } from '../stores/providerStore';

interface ProviderFormProps {
  provider: Provider | null;  // null = 新建模式
  initialModels?: Model[];  // 编辑模式时的初始模型列表
  onSave: (config: ProviderConfig, models: Model[], probeModel?: string) => Promise<boolean>;
  onDelete?: () => Promise<boolean>;
  onChange: () => void;
  loading?: boolean;
  onModelsRefreshed?: () => Promise<void>;
}

const EMPTY_MODELS: Model[] = [];

const CODE_REQUEST_NOT_SUPPORTED = 1001020;

// Actionable text for the error codes a connection test can return (V1-FR-001 §8). Other codes show the host message.
const DIAGNOSTIC_HINTS: Readonly<Record<number, string>> = {
  1001002: 'Invalid API key',
  1001019: 'The key lacks permission, or the region is restricted',
  1001025:
    'The response was not the expected JSON. Check that the base URL is not missing /v1 and is not a web page address, and note redirects are not followed - enter the final address',
  1001023:
    'Plain HTTP is not allowed for this address. Use https://, or enable Allow plain HTTP under Advanced options',
};

const NO_MODEL_LIST_HINT = 'This service has no model list endpoint. Enter a model name below to test with it.';

const AUTH_STYLE_OPTIONS = [
  { value: 'default', label: 'Default' },
  { value: 'bearer', label: 'Bearer' },
  { value: 'x-api-key', label: 'x-api-key' },
  { value: 'both', label: 'Both' },
] as const;

const KIND_OPTIONS = [
  { value: 'auto', label: 'Auto' },
  ...MODEL_KINDS.map((kind) => ({ value: kind, label: kind })),
];

/** True for an `http://` address whose host is loopback or a private literal: cleartext there is normal, so only a note. */
function isLocalCleartext(baseUrl: string): boolean {
  let url: URL;
  try {
    url = new URL(baseUrl.trim());
  } catch {
    return false;
  }
  if (url.protocol !== 'http:') return false;
  // WHATWG URL keeps IPv6 hosts in brackets; compare the bare address.
  const host = url.hostname.toLowerCase().replace(/^\[|\]$/g, '');
  if (host === 'localhost' || host === '::1') return true;
  const octets = host.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
  if (!octets) return false;
  const [a, b] = [Number(octets[1]), Number(octets[2])];
  return a === 127 || a === 10 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168);
}

/** Selects `vendor` in `form`: its endpoint (the preferred protocol if offered) and, when asked, the endpoint's default URL. */
function withVendor(
  form: ProviderConfig,
  vendor: ProviderVendor,
  preferredType: string,
  baseUrl: 'default' | 'keep',
): ProviderConfig {
  const endpoint = vendor.endpoints.find((row) => row.provider_type === preferredType) ?? vendor.endpoints[0];
  return {
    ...form,
    provider_type: endpoint?.provider_type ?? form.provider_type,
    base_url: baseUrl === 'default' ? (endpoint?.default_base_url ?? '') : form.base_url,
    config: { ...form.config, vendor_id: vendor.vendor_id },
  };
}

function newProviderForm(vendors: ProviderVendor[]): ProviderConfig {
  const form: ProviderConfig = {
    name: '',
    provider_type: 'openai_compatible',
    base_url: '',
    api_key: '',
    enabled: true,
    config: undefined,
  };
  return vendors.length > 0 ? withVendor(form, vendors[0], form.provider_type, 'default') : form;
}

function savedProviderForm(provider: Provider): ProviderConfig {
  return {
    name: provider.name,
    provider_type: provider.provider_type,
    base_url: provider.base_url,
    api_key: '', // 编辑模式下留空
    enabled: provider.enabled,
    config: provider.config,
  };
}

interface CommandResponse<T> {
  success: boolean;
  data?: T;
  error?: { code: number; message: string };
}

export default function ProviderForm({
  provider,
  initialModels = EMPTY_MODELS,
  onSave,
  onDelete,
  onChange,
  loading = false,
  onModelsRefreshed,
}: ProviderFormProps) {
  const isEditMode = !!provider;
  const isNewMode = !provider;
  const reportError = usePageError('ProviderManagement');
  const vendors = useProviderStore((state) => state.vendors);
  const vendorsStatus = useProviderStore((state) => state.vendorsStatus);
  const addManualModel = useProviderStore((state) => state.addManualModel);
  const removeManualModel = useProviderStore((state) => state.removeManualModel);

  // 表单状态
  const [formData, setFormData] = useState<ProviderConfig>(() =>
    provider ? savedProviderForm(provider) : newProviderForm(vendors),
  );

  // 操作状态
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<'success' | 'error' | null>(null);
  const [testMessage, setTestMessage] = useState<string>('');
  const [testDetail, setTestDetail] = useState<string>('');
  // The service has no model list endpoint, so a test needs one model name to send a small request to.
  const [needsProbe, setNeedsProbe] = useState(false);
  const [probeModel, setProbeModel] = useState('');
  const [advancedOpen, setAdvancedOpen] = useState(false);

  const [fetchingModels, setFetchingModels] = useState(false);
  const [models, setModels] = useState<Model[]>([]);
  const [modelsError, setModelsError] = useState<string | null>(null);

  const [manualModelId, setManualModelId] = useState('');
  const [manualKind, setManualKind] = useState('auto');
  const [manualBusy, setManualBusy] = useState<string | null>(null);

  const [saving, setSaving] = useState(false);
  const operationGeneration = useRef(0);

  const configVendorId = typeof formData.config?.vendor_id === 'string' ? formData.config.vendor_id : '';
  const vendorId = configVendorId || (provider ? resolveVendorId(provider, vendors) : '');
  const vendor = vendors.find((row) => row.vendor_id === vendorId);
  const endpoints = vendor?.endpoints ?? [];
  const vendorOptions = vendors.map((row) => ({ value: row.vendor_id, label: row.display_name }));
  if (vendorId && !vendor && (vendors.length > 0 || configVendorId)) {
    vendorOptions.push({ value: vendorId, label: vendorId });
  }

  const endpointChanged = !!provider && (
    formData.base_url !== provider.base_url
    || formData.provider_type !== provider.provider_type
    || (configVendorId !== '' && configVendorId !== resolveVendorId(provider, vendors))
  );
  const needsNewKey = endpointChanged && !formData.api_key;
  const authStyle = typeof formData.config?.auth_style === 'string' ? formData.config.auth_style : 'default';
  const allowInsecureHttp = formData.config?.allow_insecure_http === true;

  // 当 provider 变化时重置表单
  useEffect(() => {
    if (provider) {
      setFormData(savedProviderForm(provider));
      // 编辑模式：加载初始模型列表
    } else {
      setFormData(newProviderForm(useProviderStore.getState().vendors));
      // 新建模式：清空模型列表
      setModels([]);
    }
    setTestResult(null);
    setTestMessage('');
    setTestDetail('');
    setNeedsProbe(false);
    setProbeModel('');
    setModelsError(null);
  }, [provider?.id, provider?.config_revision]);

  useEffect(() => {
    if (provider) setModels(initialModels);
  }, [initialModels, provider?.id]);

  useEffect(() => {
    operationGeneration.current += 1;
    setTesting(false);
    setFetchingModels(false);
    return () => { operationGeneration.current += 1; };
  }, [provider?.id, provider?.config_revision]);

  // The vendor table belongs to the host. The page also asks for it; the store collapses concurrent requests.
  useEffect(() => {
    if (!isNewMode) return;
    const { vendorsStatus: status, fetchVendors } = useProviderStore.getState();
    if (status === 'idle' || status === 'error') void fetchVendors();
  }, [isNewMode]);

  // A new provider starts on the first vendor once the table is available, unless one was already chosen.
  useEffect(() => {
    if (!isNewMode || vendors.length === 0) return;
    setFormData((current) => (
      current.config?.vendor_id
        ? current
        : withVendor(current, vendors[0], current.provider_type, current.base_url ? 'keep' : 'default')
    ));
  }, [isNewMode, vendors]);

  // Anything that changes what a test or a model preview would talk to makes earlier results stale.
  const invalidateChecks = (resetProbe = true) => {
    operationGeneration.current += 1;
    setTesting(false);
    setFetchingModels(false);
    setTestResult(null);
    setTestMessage('');
    setTestDetail('');
    setModelsError(null);
    if (isNewMode) setModels([]);
    if (resetProbe) {
      setProbeModel('');
      setNeedsProbe(false);
    }
  };

  // 表单变化处理
  const handleChange = <K extends keyof ProviderConfig>(field: K, value: ProviderConfig[K]) => {
    if (field === 'base_url' || field === 'provider_type' || field === 'api_key') invalidateChecks();
    setFormData({ ...formData, [field]: value });
    onChange();
  };

  const handleVendorChange = (id: string) => {
    const next = vendors.find((row) => row.vendor_id === id);
    if (!next || id === vendorId) return;
    invalidateChecks();
    setFormData((current) => withVendor(current, next, current.provider_type, isNewMode ? 'default' : 'keep'));
    onChange();
  };

  const handleEndpointChange = (type: string) => {
    if (type === formData.provider_type) return;
    const next = endpoints.find((row) => row.provider_type === type);
    const previousDefault = endpoints.find((row) => row.provider_type === formData.provider_type)?.default_base_url ?? null;
    invalidateChecks();
    setFormData((current) => ({
      ...current,
      provider_type: type,
      base_url: isNewMode
        ? (next?.default_base_url ?? (current.base_url === previousDefault ? '' : current.base_url))
        : current.base_url,
    }));
    onChange();
  };

  // Edits one key of `config` and leaves every other key (retry settings and so on) as it was.
  const setConfigKey = (key: string, value: unknown) => {
    invalidateChecks(false);
    setFormData((current) => {
      const next = { ...(current.config ?? {}) };
      if (value === undefined) delete next[key];
      else next[key] = value;
      return { ...current, config: current.config === undefined && Object.keys(next).length === 0 ? undefined : next };
    });
    onChange();
  };

  // 测试连接
  const handleTestConnection = async () => {
    if (needsNewKey) return;
    setTesting(true);
    setTestResult(null);
    setTestMessage('');
    setTestDetail('');
    const generation = operationGeneration.current;
    const probe = probeModel.trim();
    const probeArgs = probe ? { probeModel: probe } : {};

    try {
      let response: CommandResponse<boolean>;

      if (isEditMode && provider) {
        // 编辑模式：如果没有输入新的 API Key，使用数据库中的
        if (!formData.api_key) {
          response = await invoke<CommandResponse<boolean>>('test_provider_by_id', {
            providerId: provider.id,
            ...probeArgs,
          });
        } else {
          // 输入了新的 API Key，使用新的配置测试
          response = await invoke<CommandResponse<boolean>>('test_provider_connection', {
            config: formData,
            ...probeArgs,
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
          ...probeArgs,
        });
      }

      if (generation !== operationGeneration.current) return;
      if (response.success && response.data) {
        setTestResult('success');
        setTestMessage('Connection successful');
      } else {
        const hostMessage = response.error?.message || 'Connection failed';
        const code = response.error?.code;
        setTestResult('error');
        if (code === CODE_REQUEST_NOT_SUPPORTED && !probe) {
          // Ask for a model name once; with one already entered the host message is the answer.
          setNeedsProbe(true);
          setTestMessage(NO_MODEL_LIST_HINT);
          setTestDetail(hostMessage);
        } else if (code !== undefined && DIAGNOSTIC_HINTS[code]) {
          setTestMessage(DIAGNOSTIC_HINTS[code]);
          setTestDetail(hostMessage);
        } else {
          setTestMessage(hostMessage);
        }
        if (response.error) reportError(response.error);
      }
    } catch (err) {
      if (generation !== operationGeneration.current) return;
      setTestResult('error');
      setTestMessage(err instanceof Error ? err.message : 'Unknown error');
      reportError(err);
    } finally {
      if (generation === operationGeneration.current) setTesting(false);
    }
  };

  // 拉取/刷新模型
  const handleFetchModels = async () => {
    if (endpointChanged) return;
    setFetchingModels(true);
    setModelsError(null);
    const generation = operationGeneration.current;

    try {
      if (isEditMode && provider) {
        // 编辑模式：调用 refresh_models（会保存到数据库）
        const response = await invoke<CommandResponse<Model[]>>('refresh_models', {
          providerId: provider.id,
        });

        if (generation !== operationGeneration.current) return;
        if (response.success && response.data) {
          setModels(response.data);
          await onModelsRefreshed?.();
        } else {
          setModelsError(response.error?.message || 'Failed to fetch models');
          if (response.error) reportError(response.error);
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

        if (generation !== operationGeneration.current) return;
        if (response.success && response.data) {
          setModels(response.data.map((model) => ({
            ...model,
            provider_id: 'preview',
          })));
        } else {
          setModelsError(response.error?.message || 'Failed to fetch models');
          if (response.error) reportError(response.error);
        }
      }
    } catch (err) {
      if (generation !== operationGeneration.current) return;
      setModelsError(err instanceof Error ? err.message : 'Unknown error');
      reportError(err);
    } finally {
      if (generation === operationGeneration.current) setFetchingModels(false);
    }
  };

  // 保存
  const handleSave = async () => {
    if (needsNewKey) return;
    setSaving(true);
    const probe = isNewMode ? probeModel.trim() : '';
    const success = await onSave(formData, models, probe || undefined);
    setSaving(false);

    if (!success && isNewMode && !probe) {
      // Creating verifies the endpoint; a service without a model list needs a model name to do that.
      if (extractErrorCode(useProviderStore.getState().lastFailure) === CODE_REQUEST_NOT_SUPPORTED) {
        setNeedsProbe(true);
        setTestResult('error');
        setTestMessage(NO_MODEL_LIST_HINT);
        setTestDetail('');
      }
    }

    if (success && isEditMode) setFormData(current => ({ ...current, api_key: '' }));

    if (success && isNewMode) {
      // 新建成功后重置表单
      setFormData(newProviderForm(vendors));
      setModels([]);
      setTestResult(null);
      setTestMessage('');
      setTestDetail('');
      setNeedsProbe(false);
      setProbeModel('');
    }
  };

  // 删除
  const handleDelete = async () => {
    if (onDelete) {
      await onDelete();
    }
  };

  // 手动模型（仅已保存的 Provider）
  const handleAddManualModel = async () => {
    const modelId = manualModelId.trim();
    if (!provider || !modelId || manualBusy) return;
    setManualBusy('add');
    const ok = await addManualModel(provider.id, modelId, manualKind === 'auto' ? undefined : manualKind);
    setManualBusy(null);
    if (ok) setManualModelId('');
    else reportError(useProviderStore.getState().lastFailure);
  };

  const handleRemoveManualModel = async (model: Model) => {
    if (!provider || manualBusy) return;
    setManualBusy(model.model_id);
    const ok = await removeManualModel(provider.id, model.model_id);
    setManualBusy(null);
    if (!ok) reportError(useProviderStore.getState().lastFailure);
  };

  return (
    <div className="flex max-w-190 flex-col gap-6">
      {/* 标题 */}
      <header className="flex flex-wrap items-center justify-between gap-2 border-b border-border pb-4">
        <h2 className="m-0 min-w-0 wrap-anywhere text-2xl font-semibold text-foreground">
          {isNewMode ? 'New Provider' : provider.name}
        </h2>
        {isEditMode && (
          <span className="font-mono text-xs text-muted-foreground">ID: {provider.id.slice(0, 8)}...</span>
        )}
      </header>

      {/* 配置区 */}
      <Card>
        <Card.Header>
          <h3 className="m-0 text-base font-semibold">Configuration</h3>
        </Card.Header>
        <Card.Body>
          <Stack gap={4}>
            <Stack gap={1}>
              <Input
                id="name"
                label={<>Name <span className="text-error-text">*</span></>}
                value={formData.name}
                onChange={(e) => handleChange('name', e.target.value)}
                placeholder="My Provider"
                disabled={loading || saving}
              />
            </Stack>

            <Stack gap={1}>
              <Select
                id="vendor"
                label="Vendor"
                value={vendorId}
                onChange={handleVendorChange}
                options={vendorOptions}
                placeholder="Select a vendor"
                disabled={loading || saving}
              />
              {vendorsStatus === 'error' && vendors.length === 0 && (
                <p className="m-0 text-xs text-warning-text">The vendor list could not be loaded.</p>
              )}
            </Stack>

            {endpoints.length > 1 && (
              <Stack gap={1}>
                <Select
                  id="provider_type"
                  label="Provider Type"
                  value={formData.provider_type}
                  onChange={handleEndpointChange}
                  options={endpoints.map((row) => ({ value: row.provider_type, label: row.display_name }))}
                  disabled={loading || saving}
                />
                <p className="m-0 text-xs text-muted-foreground">The API protocol this service speaks</p>
              </Stack>
            )}

            <Stack gap={1}>
              <Input
                id="base_url"
                label={<>Base URL <span className="text-error-text">*</span></>}
                value={formData.base_url}
                onChange={(e) => handleChange('base_url', e.target.value)}
                placeholder="https://your-service.example.com/v1"
                disabled={loading || saving}
              />
              <p className="m-0 text-xs text-muted-foreground">The base URL for the API endpoint</p>
              {isLocalCleartext(formData.base_url) && (
                <p role="note" className="m-0 text-xs text-muted-foreground">
                  This address uses http:// on this machine or a private network. Traffic to it is not encrypted, which is usual for a local service.
                </p>
              )}
            </Stack>

            <Stack gap={1}>
              <Input
                id="api_key"
                type="password"
                label={<>API Key <span className="text-error-text">*</span></>}
                value={formData.api_key}
                onChange={(e) => handleChange('api_key', e.target.value)}
                placeholder={isEditMode ? 'Leave empty to keep current' : 'sk-...'}
                disabled={loading || saving}
              />
              {isEditMode ? (
                <p className="m-0 text-xs text-muted-foreground">Leave empty only when keeping the same endpoint and provider type. Changes require a new API key.</p>
              ) : (
                <p className="m-0 text-xs text-muted-foreground">Your API key is stored in your OS credential store, not in the configuration database.</p>
              )}
              {needsNewKey && (
                <p role="status" className="m-0 text-sm text-warning-text">
                  Enter a new API key before testing or saving the changed endpoint. Save changes before refreshing cached models.
                </p>
              )}
            </Stack>

            <Stack gap={1}>
              <Checkbox
                id="enabled"
                label="Enabled"
                checked={formData.enabled}
                onChange={(checked) => handleChange('enabled', checked)}
                disabled={loading || saving}
              />
              <p className="m-0 text-xs text-muted-foreground">Disabled providers cannot be used for API calls</p>
            </Stack>

            <Stack gap={3}>
              <div>
                <Button
                  variant="ghost"
                  size="sm"
                  aria-expanded={advancedOpen}
                  aria-controls="provider-advanced-options"
                  onClick={() => setAdvancedOpen((open) => !open)}
                >
                  <span aria-hidden="true">{advancedOpen ? '▾' : '▸'}</span>
                  Advanced options
                </Button>
              </div>
              {advancedOpen && (
                <div id="provider-advanced-options" className="flex flex-col gap-4 rounded-md border border-border p-3">
                  <Stack gap={1}>
                    <Select
                      id="auth_style"
                      label="Auth header style"
                      value={authStyle}
                      onChange={(value) => setConfigKey('auth_style', value === 'default' ? undefined : value)}
                      options={AUTH_STYLE_OPTIONS}
                      disabled={loading || saving}
                    />
                    <p className="m-0 text-xs text-muted-foreground">How the API key is sent. Leave on Default unless the service requires a specific header.</p>
                  </Stack>
                  <Stack gap={1}>
                    <Checkbox
                      id="allow_insecure_http"
                      label="Allow plain HTTP"
                      checked={allowInsecureHttp}
                      onChange={(checked) => setConfigKey('allow_insecure_http', checked ? true : undefined)}
                      disabled={loading || saving}
                    />
                    {allowInsecureHttp && (
                      <p role="note" className="m-0 rounded-md bg-warning-bg p-2 text-sm text-warning-text">
                        Plain HTTP sends your API key and prompts unencrypted. Anyone on the network path can read them. Enable this only for a service you trust on a network you control.
                      </p>
                    )}
                  </Stack>
                </div>
              )}
            </Stack>
          </Stack>
        </Card.Body>
      </Card>

      {/* 操作区 */}
      <Card>
        <Card.Header>
          <h3 className="m-0 text-base font-semibold">Actions</h3>
        </Card.Header>
        <Card.Body>
          <Stack gap={3}>
            {needsProbe && (
              <Stack gap={1}>
                <Input
                  id="probe_model"
                  label="Model name"
                  value={probeModel}
                  onChange={(e) => setProbeModel(e.target.value)}
                  placeholder="e.g. a model your service offers"
                  disabled={saving}
                />
                <p className="m-0 text-xs text-muted-foreground">
                  Testing sends one small request to this model, which consumes a few tokens.
                </p>
              </Stack>
            )}

            <div className="flex flex-wrap gap-3">
              <Button
                variant="secondary"
                onClick={handleTestConnection}
                loading={testing}
                disabled={saving || needsNewKey || !formData.base_url || (!isEditMode && !formData.api_key)}
              >
                {testing ? 'Testing...' : '🔌 Test Connection'}
              </Button>

              <Button
                variant="secondary"
                onClick={handleFetchModels}
                loading={fetchingModels}
                disabled={saving || endpointChanged || (isEditMode && !!formData.api_key) || (!isEditMode && !formData.api_key)}
              >
                {fetchingModels ? 'Fetching...' : '📥 Refresh Models'}
              </Button>
            </div>

            {/* 测试结果 */}
            {testResult && (
              <div
                className={cn(
                  'wrap-anywhere rounded-md p-3 text-sm',
                  testResult === 'success' ? 'bg-success-bg text-success-text' : 'bg-error-bg text-error-text',
                )}
              >
                <span aria-hidden="true">{testResult === 'success' ? '✓' : '✗'}</span> <span>{testMessage}</span>
                {testDetail && <div className="mt-1 text-xs">{testDetail}</div>}
              </div>
            )}

            {/* 模型错误 */}
            {modelsError && (
              <div className="wrap-anywhere rounded-md bg-error-bg p-3 text-sm text-error-text">✗ {modelsError}</div>
            )}
          </Stack>
        </Card.Body>
      </Card>

      {/* 模型列表 */}
      {(models.length > 0 || isEditMode) && (
        <Card>
          <Card.Header>
            <h3 className="m-0 text-base font-semibold">Models ({models.length})</h3>
          </Card.Header>
          <Card.Body>
            <Stack gap={3}>
              <div className="flex flex-wrap gap-3">
                {models.map((model) => (
                  <div
                    key={model.id}
                    className="flex min-w-0 grow basis-60 flex-col gap-1 rounded-lg border border-border bg-background p-3"
                  >
                    <h4 className="m-0 wrap-anywhere text-sm font-semibold text-foreground">{model.model_name}</h4>
                    <div className="wrap-anywhere font-mono text-xs text-muted-foreground">{model.model_id}</div>
                    <div className="flex flex-wrap gap-1">
                      <span className="rounded-sm bg-secondary px-2 py-1 text-xs text-secondary-foreground">
                        {model.model_type === 'chat' ? 'text-generation' : model.model_type}
                      </span>
                      {model.source === 'manual' && (
                        <span className="rounded-sm bg-warning-bg px-2 py-1 text-xs text-warning-text">manual</span>
                      )}
                      {model.supports_streaming && (
                        <span className="rounded-sm bg-info-bg px-2 py-1 text-xs text-info-text">Streaming</span>
                      )}
                      {model.supports_vision && (
                        <span className="rounded-sm bg-info-bg px-2 py-1 text-xs text-info-text">Vision</span>
                      )}
                    </div>
                    {model.context_window && (
                      <div className="text-xs text-muted-foreground">
                        Context: {model.context_window.toLocaleString()} tokens
                      </div>
                    )}
                    {isEditMode && model.source === 'manual' && (
                      <div>
                        <Button
                          variant="ghost"
                          size="sm"
                          aria-label={`Remove ${model.model_id}`}
                          loading={manualBusy === model.model_id}
                          disabled={manualBusy !== null}
                          onClick={() => void handleRemoveManualModel(model)}
                        >
                          Remove
                        </Button>
                      </div>
                    )}
                  </div>
                ))}
              </div>
              {isEditMode && (
                <div className="flex flex-wrap items-end gap-3">
                  <div className="min-w-0 grow basis-60">
                    <Input
                      id="manual_model_id"
                      label="Model ID"
                      value={manualModelId}
                      onChange={(e) => setManualModelId(e.target.value)}
                      placeholder="model-id"
                      disabled={saving}
                    />
                  </div>
                  <div className="w-56 max-w-full">
                    <Select
                      id="manual_model_kind"
                      label="Kind"
                      value={manualKind}
                      onChange={setManualKind}
                      options={KIND_OPTIONS}
                      disabled={saving}
                    />
                  </div>
                  <Button
                    variant="secondary"
                    onClick={() => void handleAddManualModel()}
                    loading={manualBusy === 'add'}
                    disabled={saving || manualBusy !== null || !manualModelId.trim()}
                  >
                    Add model
                  </Button>
                </div>
              )}
              {isNewMode && (
                <p className="m-0 text-xs text-muted-foreground">
                  These models will be saved when you create the provider
                </p>
              )}
              {isEditMode && (
                <p className="m-0 text-xs text-muted-foreground">
                  Models were updated in the database
                </p>
              )}
            </Stack>
          </Card.Body>
        </Card>
      )}

      {/* 底部操作 */}
      <footer className="flex flex-wrap items-center gap-3 border-t border-border pt-6">
        {isEditMode && onDelete && (
          <Button variant="danger" onClick={handleDelete} disabled={saving}>
            🗑️ Delete
          </Button>
        )}
        <Button
          className="ml-auto"
          onClick={handleSave}
          loading={saving}
          disabled={loading || needsNewKey || !formData.name || !formData.base_url || (!isEditMode && !formData.api_key)}
        >
          {saving ? 'Saving...' : isNewMode ? '💾 Create' : '💾 Save Changes'}
        </Button>
      </footer>
    </div>
  );
}
