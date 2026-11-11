import { Input, Select, Stack } from '@dgos/ui';
import { t } from '../i18n';
import type { CliProviderAuthStyle, CliProviderProtocol, CliProviderRouteConfig, CliProviderRoutes } from '../stores/providerStore';

interface ProviderCliRoutesProps {
  routes?: CliProviderRoutes;
  providerType: string;
  baseUrl: string;
  disabled: boolean;
  onChange: (routes: CliProviderRoutes | undefined) => void;
}

const CLI_ENGINES = [
  { protocol: 'responses', label: 'providerForm.cliCodex', providerType: 'openai_compatible' },
  { protocol: 'messages', label: 'providerForm.cliClaude', providerType: 'anthropic' },
] as const;

export default function ProviderCliRoutes({ routes, providerType, baseUrl, disabled, onChange }: ProviderCliRoutesProps) {
  const setRoute = (protocol: CliProviderProtocol, route: CliProviderRouteConfig | undefined) => {
    const next = { ...routes };
    if (route === undefined) delete next[protocol];
    else next[protocol] = route;
    onChange(Object.keys(next).length ? next : undefined);
  };

  return (
    <section aria-labelledby="provider-cli-routes-title" className="flex min-w-0 flex-col gap-3">
      <h4 id="provider-cli-routes-title" className="m-0 text-sm font-medium">{t('providerForm.cliRoutes')}</h4>
      <p className="m-0 text-xs text-ink-secondary">{t('providerForm.cliRoutesHelp')}</p>
      <p className="m-0 text-xs text-ink-secondary">{t('providerForm.cliSharedKey')}</p>
      {CLI_ENGINES.map((engine) => {
        const route = routes?.[engine.protocol];
        const mode = route ? (route.enabled ? 'enabled' : 'disabled') : 'auto';
        const prefix = `cli-${engine.protocol}`;
        const override = route?.base_url?.trim() ?? '';
        const sharesKey = override !== '' && override.replace(/\/+$/, '') !== baseUrl.trim().replace(/\/+$/, '');
        const statusKey = mode === 'enabled' ? 'providerForm.cliConfigured'
          : mode === 'disabled' ? 'providerForm.cliDisabled'
          : providerType === engine.providerType ? 'providerForm.cliAutoCandidate' : 'providerForm.cliAutoUnavailable';

        return (
          <fieldset key={engine.protocol} className="m-0 min-w-0 border-0 border-l border-border-subtle p-0 pl-3">
            <legend className="mb-2 px-0 text-sm font-medium">{t(engine.label)}</legend>
            <Stack gap={2}>
              <Select id={`${prefix}-mode`} label={t('providerForm.cliRouteMode')} value={mode}
                onChange={(value) => setRoute(engine.protocol, value === 'auto' ? undefined : { ...route, enabled: value === 'enabled' })}
                options={[
                  { value: 'auto', label: t('providerForm.cliAuto') },
                  { value: 'enabled', label: t('providerForm.cliEnabled') },
                  { value: 'disabled', label: t('providerForm.cliDisabledOption') },
                ]} disabled={disabled} />
              <p role="status" className="m-0 text-xs text-ink-secondary">{t(statusKey)}</p>
              {mode === 'enabled' && (
                <>
                  <Input id={`${prefix}-base-url`} label={t('providerForm.cliBaseUrl')} inputMode="url"
                    value={route?.base_url ?? ''} placeholder={baseUrl || t('providerForm.baseUrlExample')}
                    aria-describedby={`${prefix}-base-help${sharesKey ? ` ${prefix}-key-warning` : ''}`}
                    onChange={(event) => {
                      const next = { ...route, enabled: true };
                      if (event.target.value) next.base_url = event.target.value;
                      else delete next.base_url;
                      setRoute(engine.protocol, next);
                    }} disabled={disabled} />
                  <p id={`${prefix}-base-help`} className="m-0 text-xs text-ink-secondary">{t('providerForm.cliBaseUrlHelp')}</p>
                  {sharesKey && (
                    <p id={`${prefix}-key-warning`} role="note" className="wrap-anywhere m-0 text-xs text-status-warning-text">
                      {t('providerForm.cliKeySharingWarning', { url: override })}
                    </p>
                  )}
                  <Select id={`${prefix}-auth`} label={t('providerForm.cliAuthStyle')} value={route?.auth_style ?? 'default'}
                    onChange={(value) => {
                      const next = { ...route, enabled: true };
                      if (value === 'default') delete next.auth_style;
                      else next.auth_style = value as CliProviderAuthStyle;
                      setRoute(engine.protocol, next);
                    }}
                    options={[
                      { value: 'default', label: t('providerForm.cliAuthDefault') },
                      { value: 'bearer', label: t('providerForm.authBearer') },
                      ...(engine.protocol === 'messages' ? [
                        { value: 'x-api-key', label: t('providerForm.authApiKey') },
                        { value: 'both', label: t('provider.both') },
                      ] : []),
                    ]} disabled={disabled} />
                  <p className="m-0 text-xs text-ink-secondary">{t(engine.protocol === 'responses'
                    ? 'providerForm.cliResponsesAuthHelp' : 'providerForm.cliMessagesAuthHelp')}</p>
                </>
              )}
            </Stack>
          </fieldset>
        );
      })}
    </section>
  );
}
