import { t, useLocale as usePageLocale } from '../i18n';
import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button, Checkbox, Input, Select, Stack, cn, extractErrorCode } from '@dgos/ui';
import { usePageError } from '../pageError';
import { resolveVendorId, useProviderStore } from '../stores/providerStore';
import type { ProviderConfig, ProviderVendor, Model, Provider } from '../stores/providerStore';
import ProviderModels from './ProviderModels';

interface ProviderFormProps {
  provider: Provider | null;  // null = 新建模式
  initialModels?: Model[];  // 编辑模式时的初始模型列表
  onSave: (config: ProviderConfig, models: Model[], probeModel?: string) => Promise<boolean>;
  onChange: (dirty: boolean) => void;
  loading?: boolean;
  onModelsRefreshed?: () => Promise<void>;
}

const EMPTY_MODELS: Model[] = [];

const CODE_REQUEST_NOT_SUPPORTED = 1001020;

// Actionable text for the error codes a connection test can return (V1-FR-001 §8). Other codes show the host message.
const DIAGNOSTIC_HINTS: Readonly<Record<number, string>> = {
  1001002: 'providerForm.invalidApiKey',
  1001019: 'providerForm.permissionRegion',
  1001025: 'providerForm.invalidJsonEndpoint',
  1001023: 'providerForm.httpNotAllowed',
};

const NO_MODEL_LIST_HINT = 'providerForm.noModelList';

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

function sameValue(left: unknown, right: unknown): boolean {
  if (left === right) return true;
  if (!left || !right || typeof left !== 'object' || typeof right !== 'object') return false;
  const a = left as Record<string, unknown>;
  const b = right as Record<string, unknown>;
  const keys = Object.keys(a);
  return keys.length === Object.keys(b).length
    && keys.every((key) => Object.prototype.hasOwnProperty.call(b, key) && sameValue(a[key], b[key]));
}

function sameDraft(left: ProviderConfig, right: ProviderConfig, includeEnabled = false): boolean {
  return left.name === right.name && left.provider_type === right.provider_type
    && left.base_url === right.base_url && left.api_key === right.api_key
    && (!includeEnabled || left.enabled === right.enabled)
    && sameValue(left.config ?? {}, right.config ?? {});
}

interface CommandResponse<T> {
  success: boolean;
  data?: T;
  error?: { code: number; message: string };
}

export default function ProviderForm({
  provider, initialModels = EMPTY_MODELS, onSave, onChange, loading = false, onModelsRefreshed,
}: ProviderFormProps) {
  usePageLocale();
  const isEditMode = !!provider;
  const isNewMode = !provider;
  const reportError = usePageError('ProviderManagement');
  const vendors = useProviderStore((state) => state.vendors);
  const vendorsStatus = useProviderStore((state) => state.vendorsStatus);
  const addManualModel = useProviderStore((state) => state.addManualModel);
  const removeManualModel = useProviderStore((state) => state.removeManualModel);
  const [formData, setFormData] = useState<ProviderConfig>(() =>
    provider ? savedProviderForm(provider) : newProviderForm(vendors),
  );
  const [baseline, setBaseline] = useState(formData);
  const baselineRef = useRef(baseline);
  const currentProvider = useRef(provider);
  currentProvider.current = provider;
  const previousId = useRef(provider?.id);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<'success' | 'error' | null>(null);
  const [testMessage, setTestMessage] = useState('');
  const [testDetail, setTestDetail] = useState('');
  const [needsProbe, setNeedsProbe] = useState(false);
  const [probeModel, setProbeModel] = useState('');
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [keyVisible, setKeyVisible] = useState(false);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [models, setModels] = useState<Model[]>(initialModels);
  const [draftModels, setDraftModels] = useState<Model[]>([]);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [manualBusy, setManualBusy] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const operationGeneration = useRef(0);
  const mounted = useRef(true);
  const dirty = !sameDraft(formData, baseline, isNewMode);
  const busy = loading || saving;
  const configVendorId = typeof formData.config?.vendor_id === 'string' ? formData.config.vendor_id : '';
  const vendorId = configVendorId || (provider ? resolveVendorId(provider, vendors) : '');
  const vendor = vendors.find((row) => row.vendor_id === vendorId);
  const endpoints = vendor?.endpoints ?? [];
  const vendorOptions = vendors.map((row) => ({ value: row.vendor_id, label: row.display_name }));
  if (vendorId && !vendor && (vendors.length > 0 || configVendorId)) vendorOptions.push({ value: vendorId, label: vendorId });
  const endpointChanged = !!provider && (
    formData.base_url !== provider.base_url || formData.provider_type !== provider.provider_type
    || (configVendorId !== '' && configVendorId !== resolveVendorId(provider, vendors))
  );
  const connectionDirty = !!provider && (endpointChanged || !!formData.api_key
    || !sameValue(formData.config ?? {}, provider.config ?? {}));
  const previewMode = isNewMode || connectionDirty;
  const modelFetchBlocked = previewMode && (!formData.base_url.trim() || !formData.api_key.trim() || !formData.provider_type);
  const needsNewKey = endpointChanged && !formData.api_key;
  const savedKeyTestBlocked = isEditMode && !formData.api_key && connectionDirty;
  const authStyle = typeof formData.config?.auth_style === 'string' ? formData.config.auth_style : 'default';
  const allowInsecureHttp = formData.config?.allow_insecure_http === true;

  useEffect(() => { onChange(dirty); }, [dirty, onChange]);

  // Toggle and refresh revisions are not navigation: retain the draft and adopt the saved switch state.
  useEffect(() => {
    const changedIdentity = previousId.current !== provider?.id;
    previousId.current = provider?.id;
    const next = provider ? savedProviderForm(provider) : newProviderForm(useProviderStore.getState().vendors);
    const previousBaseline = baselineRef.current;
    baselineRef.current = next;
    setBaseline(next);
    setFormData((current) => changedIdentity || sameDraft(current, previousBaseline, !provider)
      ? next : { ...current, enabled: provider?.enabled ?? current.enabled });
    if (changedIdentity) {
      setModels(provider ? initialModels : []);
      setDraftModels([]);
      setModelsError(null);
      setKeyVisible(false);
      setProbeModel('');
      setNeedsProbe(false);
      setAdvancedOpen(false);
    }
    setTestResult(null);
    setTestMessage('');
    setTestDetail('');
    operationGeneration.current += 1;
    setTesting(false);
    setFetchingModels(false);
    setManualBusy(null);
  }, [provider?.id, provider?.config_revision, provider?.enabled]);

  useEffect(() => { if (provider) setModels(initialModels); }, [initialModels, provider?.id]);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; operationGeneration.current += 1; };
  }, []);
  useEffect(() => {
    if (!isNewMode) return;
    const { vendorsStatus: status, fetchVendors } = useProviderStore.getState();
    if (status === 'idle' || status === 'error') void fetchVendors();
  }, [isNewMode]);
  useEffect(() => {
    if (!isNewMode || vendors.length === 0) return;
    setFormData((current) => current.config?.vendor_id ? current
      : withVendor(current, vendors[0], current.provider_type, current.base_url ? 'keep' : 'default'));
    if (!baselineRef.current.config?.vendor_id) {
      const next = withVendor(baselineRef.current, vendors[0], baselineRef.current.provider_type, 'default');
      baselineRef.current = next;
      setBaseline(next);
    }
  }, [isNewMode, vendors]);

  const invalidateChecks = (resetProbe = true) => {
    operationGeneration.current += 1;
    setTesting(false);
    setFetchingModels(false);
    setTestResult(null);
    setTestMessage('');
    setTestDetail('');
    setDraftModels([]);
    setModelsError(null);
    if (resetProbe) { setProbeModel(''); setNeedsProbe(false); }
  };
  const handleChange = <K extends keyof ProviderConfig>(field: K, value: ProviderConfig[K]) => {
    if (field === 'base_url' || field === 'provider_type' || field === 'api_key') invalidateChecks();
    if (field === 'api_key' && !value) setKeyVisible(false);
    setFormData((current) => ({ ...current, [field]: value }));
  };
  const handleVendorChange = (id: string) => {
    const next = vendors.find((row) => row.vendor_id === id);
    if (!next || id === vendorId) return;
    invalidateChecks();
    setFormData((current) => withVendor(current, next, current.provider_type, isNewMode ? 'default' : 'keep'));
  };
  const handleEndpointChange = (type: string) => {
    if (type === formData.provider_type) return;
    const next = endpoints.find((row) => row.provider_type === type);
    const previousDefault = endpoints.find((row) => row.provider_type === formData.provider_type)?.default_base_url ?? null;
    invalidateChecks();
    setFormData((current) => ({ ...current, provider_type: type,
      base_url: isNewMode ? (next?.default_base_url ?? (current.base_url === previousDefault ? '' : current.base_url)) : current.base_url,
    }));
  };
  const setConfigKey = (key: string, value: unknown) => {
    invalidateChecks(false);
    setFormData((current) => {
      const next = { ...(current.config ?? {}) };
      if (value === undefined) delete next[key];
      else next[key] = value;
      return { ...current, config: Object.keys(next).length ? next : undefined };
    });
  };

  const handleTestConnection = async () => {
    if (busy || testing || savedKeyTestBlocked || needsNewKey || (!provider && !formData.api_key)) return;
    setTesting(true);
    setTestResult(null);
    setTestMessage('');
    setTestDetail('');
    const generation = operationGeneration.current;
    const probe = probeModel.trim();
    const probeArgs = probe ? { probeModel: probe } : {};
    try {
      const response = provider && !formData.api_key
        ? await invoke<CommandResponse<boolean>>('test_provider_by_id', { providerId: provider.id, ...probeArgs })
        : await invoke<CommandResponse<boolean>>('test_provider_connection', { config: formData, ...probeArgs });
      if (generation !== operationGeneration.current) return;
      if (response.success && response.data) {
        setTestResult('success');
        setTestMessage(t(probe ? 'providerForm.probeSuccess' : 'providerForm.listSuccess'));
      } else {
        const hostMessage = response.error?.message || t('providerForm.failed');
        const code = response.error?.code;
        setTestResult('error');
        if (code === CODE_REQUEST_NOT_SUPPORTED && !probe) {
          setNeedsProbe(true);
          setTestMessage(t(NO_MODEL_LIST_HINT));
          setTestDetail(hostMessage);
        } else if (code !== undefined && DIAGNOSTIC_HINTS[code]) {
          setTestMessage(t(DIAGNOSTIC_HINTS[code]));
          setTestDetail(hostMessage);
        } else setTestMessage(hostMessage);
        if (response.error) reportError(response.error);
      }
    } catch (err) {
      if (generation !== operationGeneration.current) return;
      setTestResult('error');
      setTestMessage(err instanceof Error ? err.message : t('providerForm.unknownError'));
      reportError(err);
    } finally {
      if (generation === operationGeneration.current) setTesting(false);
    }
  };

  const handleFetchModels = async () => {
    if (busy || fetchingModels || manualBusy || modelFetchBlocked) return;
    setFetchingModels(true);
    setModelsError(null);
    const generation = operationGeneration.current;
    try {
      const response = !previewMode && provider
        ? await invoke<CommandResponse<Model[]>>('refresh_models', { providerId: provider.id })
        : await invoke<CommandResponse<Model[]>>('list_models_for_config', { config: formData });
      if (generation !== operationGeneration.current) return;
      if (response.success && response.data) {
        if (previewMode) setDraftModels(response.data.map((model) => ({ ...model, provider_id: 'preview' })));
        else {
          setModels(response.data);
          await onModelsRefreshed?.();
        }
      } else {
        setModelsError(t('providerForm.fetchFailed'));
        if (response.error) reportError(response.error);
      }
    } catch (err) {
      if (generation !== operationGeneration.current) return;
      setModelsError(t('providerForm.fetchFailed'));
      reportError(err);
    } finally {
      if (generation === operationGeneration.current) setFetchingModels(false);
    }
  };

  const handleSave = async () => {
    if (busy || !dirty || needsNewKey || manualBusy || fetchingModels) return;
    const targetId = provider?.id;
    const submitted = { ...formData, enabled: currentProvider.current?.enabled ?? formData.enabled };
    const probe = isNewMode ? probeModel.trim() : '';
    setSaving(true);
    try {
      const success = await onSave(submitted, isNewMode ? draftModels : models, probe || undefined);
      if (!mounted.current || currentProvider.current?.id !== targetId) return;
      if (!success) {
        if (isNewMode && !probe && extractErrorCode(useProviderStore.getState().lastFailure) === CODE_REQUEST_NOT_SUPPORTED) {
          setNeedsProbe(true);
          setTestResult('error');
          setTestMessage(t(NO_MODEL_LIST_HINT));
          setTestDetail('');
        }
        return;
      }
      const next = targetId
        ? { ...submitted, api_key: '', enabled: currentProvider.current?.enabled ?? submitted.enabled }
        : newProviderForm(useProviderStore.getState().vendors);
      baselineRef.current = next;
      setBaseline(next);
      // A save may finish after another edit: acknowledge only the submitted values.
      setFormData((current) => ({
        name: current.name === submitted.name ? next.name : current.name,
        base_url: current.base_url === submitted.base_url ? next.base_url : current.base_url,
        provider_type: current.provider_type === submitted.provider_type ? next.provider_type : current.provider_type,
        api_key: current.api_key === submitted.api_key ? '' : current.api_key,
        config: sameValue(current.config ?? {}, submitted.config ?? {}) ? next.config : current.config,
        enabled: next.enabled,
      }));
      setKeyVisible(false);
      invalidateChecks();
    } finally {
      if (mounted.current) setSaving(false);
    }
  };
  const handleDiscard = () => {
    if (busy) return;
    const next = provider ? savedProviderForm(provider) : newProviderForm(vendors);
    baselineRef.current = next;
    setBaseline(next);
    setFormData(next);
    setKeyVisible(false);
    invalidateChecks();
  };
  const handleModelMutation = async (action: 'add' | 'remove', modelId: string, kind?: string): Promise<boolean> => {
    if (!provider || busy || connectionDirty || manualBusy || fetchingModels || !modelId.trim()) return false;
    const generation = operationGeneration.current;
    setManualBusy(action === 'add' ? 'add' : modelId);
    setModelsError(null);
    try {
      const ok = action === 'add'
        ? await addManualModel(provider.id, modelId.trim(), kind)
        : await removeManualModel(provider.id, modelId);
      if (generation !== operationGeneration.current) return false;
      if (!ok) {
        const failure = useProviderStore.getState().lastFailure;
        setModelsError(t('providerForm.modelMutationFailed'));
        reportError(failure);
      }
      return ok;
    } catch (err) {
      if (generation !== operationGeneration.current) return false;
      setModelsError(t('providerForm.modelMutationFailed'));
      reportError(err);
      return false;
    } finally {
      if (generation === operationGeneration.current) setManualBusy(null);
    }
  };

  return (
    <div className="provider-form">
      <h3 className="settings-section-title">{t('providerForm.configuration')}</h3>
      <section className="provider-connection-grid" aria-label={t('providerForm.configuration')}>
        <Input id="name" label={<>{t('detail.name')}{' '}<span className="text-status-error-text">*</span></>}
          value={formData.name} onChange={(e) => handleChange('name', e.target.value)}
          placeholder={t('detail.myProvider')} disabled={busy} />
        <Stack gap={1}>
          <Select id="vendor" label={t('providerForm.vendor')} value={vendorId} onChange={handleVendorChange}
            options={vendorOptions} placeholder={t('providerForm.chooseVendor')} disabled={busy} />
          {vendorsStatus === 'error' && vendors.length === 0 && (
            <p className="m-0 text-xs text-status-warning-text">{t('providerForm.vendorUnavailable')}</p>
          )}
        </Stack>
        {endpoints.length > 1 && (
          <div className="provider-connection-wide">
            <Select id="provider_type" label={t('providerForm.type')} value={formData.provider_type}
              onChange={handleEndpointChange} options={endpoints.map((row) => ({ value: row.provider_type, label: row.display_name }))}
              disabled={busy} />
          </div>
        )}
        <div className="provider-connection-wide">
          <Input id="base_url" label={<>{t('providerForm.baseUrl')}{' '}<span className="text-status-error-text">*</span></>}
            inputMode="url" value={formData.base_url} onChange={(e) => handleChange('base_url', e.target.value)}
            placeholder={t('providerForm.baseUrlExample')} disabled={busy} />
          {isLocalCleartext(formData.base_url) && (
            <p role="note" className="mb-0 mt-1 text-xs text-ink-secondary">{t('providerForm.localHttp')}</p>
          )}
        </div>
        <div className="provider-connection-wide">
          <Input id="api_key" type={keyVisible ? 'text' : 'password'}
            label={<>{t('providerForm.apiKey')}{' '}{isEditMode && !needsNewKey
              ? <span className="text-ink-secondary">{t('providerForm.optional')}</span>
              : <span className="text-status-error-text">*</span>}</>}
            value={formData.api_key} onChange={(e) => handleChange('api_key', e.target.value)}
            placeholder={isEditMode ? t('providerForm.keepKey') : 'sk-...'} disabled={busy} />
          <div className="mt-1 flex flex-wrap items-center justify-between gap-2">
            <p className="m-0 text-xs text-ink-secondary">{t(isEditMode ? 'providerForm.editKeyHelp' : 'providerForm.keyStorage')}</p>
            {!!formData.api_key && (
              <Button variant="ghost" size="sm" aria-pressed={keyVisible} disabled={busy}
                onClick={() => setKeyVisible((visible) => !visible)}>
                {t(keyVisible ? 'providerForm.hideKey' : 'providerForm.showKey')}
              </Button>
            )}
          </div>
          {needsNewKey && <p role="status" className="mb-0 mt-2 text-sm text-status-warning-text">{t('providerForm.newKeyRequired')}</p>}
          {isEditMode && (endpointChanged || !!formData.api_key) && (
            <p role="note" className="mb-0 mt-2 text-xs text-status-warning-text">{t('providerForm.modelCacheWarning')}</p>
          )}
        </div>
        {isNewMode && (
          <Checkbox id="enabled" label={t('providerForm.enabled')} checked={formData.enabled}
            onChange={(checked) => handleChange('enabled', checked)} disabled={busy} />
        )}
        <div className="provider-connection-wide">
          <Button variant="ghost" size="sm" aria-expanded={advancedOpen} aria-controls="provider-advanced-options"
            onClick={() => setAdvancedOpen((open) => !open)}>
            <span aria-hidden="true">{advancedOpen ? '▾' : '▸'}</span>{t('providerForm.advanced')}
          </Button>
          {advancedOpen && (
            <div id="provider-advanced-options" className="mt-2 flex flex-col gap-3 border-l border-border-subtle pl-3">
              <Select id="auth_style" label={t('providerForm.authStyle')} value={authStyle}
                onChange={(value) => setConfigKey('auth_style', value === 'default' ? undefined : value)}
                options={[
                  { value: 'default', label: t('provider.default') },
                  { value: 'bearer', label: t('providerForm.authBearer') },
                  { value: 'x-api-key', label: t('providerForm.authApiKey') },
                  { value: 'both', label: t('provider.both') },
                ]}
                disabled={busy} />
              <p className="m-0 text-xs text-ink-secondary">{t('providerForm.authStyleHelp')}</p>
              <Checkbox id="allow_insecure_http" label={t('providerForm.allowHttp')} checked={allowInsecureHttp}
                onChange={(checked) => setConfigKey('allow_insecure_http', checked ? true : undefined)} disabled={busy} />
              {allowInsecureHttp && <p role="note" className="m-0 text-sm text-status-warning-text">{t('providerForm.httpWarning')}</p>}
            </div>
          )}
        </div>
      </section>
      <section className="provider-connection-check" aria-label={t('providerForm.testConnection')}>
        <Stack gap={2}>
          {needsProbe && (
            <>
              <Input id="probe_model" label={t('providerForm.modelName')} value={probeModel}
                onChange={(e) => { invalidateChecks(false); setProbeModel(e.target.value); }}
                placeholder={t('providerForm.probePlaceholder')} disabled={busy || testing} />
              <p className="m-0 text-xs text-ink-secondary">{t('providerForm.testOneSmallRequest')}</p>
            </>
          )}
          <div className="flex flex-wrap items-center gap-3">
            <Button variant="secondary" onClick={() => void handleTestConnection()} loading={testing}
              disabled={busy || savedKeyTestBlocked || needsNewKey || !formData.base_url || (!isEditMode && !formData.api_key)}>
              {testing ? t('providerForm.testing') : t('providerForm.testConnection')}
            </Button>
            <span className="text-xs text-ink-secondary">{t(isEditMode && !formData.api_key
              ? 'providerForm.testSavedTarget' : 'providerForm.testDraftTarget')}</span>
          </div>
          {savedKeyTestBlocked && <p role="status" className="m-0 text-xs text-status-warning-text">{t('providerForm.saveBeforeTest')}</p>}
          {testResult && (
            <div role={testResult === 'error' ? 'alert' : 'status'} className={cn('wrap-anywhere rounded-md p-3 text-sm',
              testResult === 'success' ? 'bg-status-success-surface text-status-success-text' : 'bg-status-error-surface text-status-error-text')}>
              {testMessage}{testDetail && <div className="mt-1 text-xs">{testDetail}</div>}
            </div>
          )}
        </Stack>
      </section>
      <ProviderModels models={previewMode ? draftModels : models} editMode={!previewMode} busy={busy} refreshing={fetchingModels}
        mutationBusy={manualBusy} blockedReason={modelFetchBlocked ? t('providerForm.modelPreviewRequired') : undefined}
        error={modelsError} onRefresh={() => void handleFetchModels()}
        onAdd={(modelId, kind) => handleModelMutation('add', modelId, kind)}
        onRemove={(model) => handleModelMutation('remove', model.model_id)} />
      {isNewMode && draftModels.length > 0 && <p className="m-0 text-xs text-ink-secondary">{t('providerForm.modelsSavedOnCreate')}</p>}
      <footer className="settings-actions provider-form-footer">
        <Button variant="secondary" onClick={handleDiscard} disabled={busy || !dirty}>{t('providerForm.discard')}</Button>
        <Button onClick={() => void handleSave()} loading={saving}
          disabled={busy || !dirty || needsNewKey || !!manualBusy || fetchingModels || !formData.name.trim()
            || !formData.base_url.trim() || (!isEditMode && !formData.api_key)}>
          {saving ? t('providerForm.saving') : isNewMode ? t('providerForm.create') : t('providerForm.saveChanges')}
        </Button>
      </footer>
    </div>
  );
}
