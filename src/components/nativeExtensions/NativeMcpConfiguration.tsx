import { useEffect, useRef, useState } from 'react';
import { Button } from '@dgos/ui';
import { t } from '../../i18n';
import { nativeCatalogKey, useNativeMcpCatalog } from '../../stores/nativeMcpCatalog';
import { useDevelopment } from '../../stores/developmentStore';
import type { NativeSelection } from '../../types/nativeExtensions';
import type { NativeMcpConfigProjection } from '../../types/nativeMcpCatalog';

/** Explicit read only: never starts a connection or exposes credential values. */
export default function NativeMcpConfiguration({ selection, name, disabled }: { selection: NativeSelection; name: string; disabled: boolean }) {
  const context = useDevelopment(() => nativeCatalogKey(selection));
  const [config, setConfig] = useState<NativeMcpConfigProjection | null>(null);
  const [busy, setBusy] = useState(false), [error, setError] = useState('');
  const generation = useRef(0), reading = useRef(false);
  useEffect(() => {
    generation.current++; reading.current = false; setConfig(null); setBusy(false); setError('');
    return () => { generation.current++; };
  }, [context, name]);
  const load = async () => {
    if (disabled || reading.current) return;
    const captured = generation.current;
    reading.current = true; setBusy(true); setError('');
    const result = await useNativeMcpCatalog.getState().project(selection, name);
    if (captured !== generation.current || context !== nativeCatalogKey(selection)) return;
    reading.current = false; setBusy(false);
    if (result.ok) setConfig(result.value);
    else { setConfig(null); setError(t(`nativeExtensions.error.${result.code}`)); }
  };
  return <section className="native-mcp-configuration" aria-label={t('nativeMcpCatalog.safeConfiguration')}>
    <Button variant="secondary" disabled={disabled || busy} onClick={() => void load()}>{t(busy ? 'nativeMcpCatalog.reading' : 'nativeMcpCatalog.readConfig')}</Button>
    {error && <p role="alert">{error}</p>}
    {config && <>
      <dl>
        <dt>{t(config.transport === 'Stdio' ? 'nativeExtensions.program' : 'nativeExtensions.url')}</dt><dd><code>{config.program ?? config.url ?? '—'}</code></dd>
        {config.transport === 'Stdio' && <><dt>{t('nativeExtensions.args')}</dt><dd><pre>{JSON.stringify(config.args, null, 2)}</pre></dd></>}
        {config.cwd && <><dt>{t('nativeExtensions.cwd')}</dt><dd><code>{config.cwd}</code></dd></>}
        <dt>{t('nativeMcpCatalog.credentialReferences')}</dt><dd>{config.credential_references.join('\n') || t('nativeMcpCatalog.noCredentials')}</dd>
      </dl>
      {config.redacted && <p>{t('nativeMcpCatalog.redacted')}</p>}
    </>}
  </section>;
}
