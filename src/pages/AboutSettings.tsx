import { useCallback, useEffect, useRef, useState } from 'react';
import { getName, getTauriVersion, getVersion } from '@tauri-apps/api/app';
import { Button, Card, Loading } from '@dgos/ui';
import { usePageError } from '../pageError';
import { t, useLocale as usePageLocale } from '../i18n';

// V1-FR-007 §2.3 item 7 (D085). Only information the app already has: the name and version from the bundle
// configuration and the Tauri runtime version, read through Tauri's built-in `app` commands (granted by
// `core:default`; no DGOS host command). System information, the licence and update checks have no source yet,
// so the page says so instead of showing a guess.

interface AboutInfo {
  name: string;
  version: string;
  tauriVersion: string;
}

type Load = { state: 'loading' } | { state: 'ready'; info: AboutInfo } | { state: 'failed' };

export default function AboutSettings() {
  usePageLocale();
  const reportError = usePageError('AboutSettings');
  const report = useRef(reportError);
  report.current = reportError;
  const mounted = useRef(true);
  const [load, setLoad] = useState<Load>({ state: 'loading' });

  const read = useCallback(async () => {
    setLoad({ state: 'loading' });
    try {
      const [name, version, tauriVersion] = await Promise.all([getName(), getVersion(), getTauriVersion()]);
      if (mounted.current) setLoad({ state: 'ready', info: { name, version, tauriVersion } });
    } catch (error) {
      if (!mounted.current) return;
      setLoad({ state: 'failed' });
      report.current(error);
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    void read();
    return () => {
      mounted.current = false;
    };
  }, [read]);

  return (
    <div className="settings-content settings-sections text-foreground">
      <h1 className="m-0 text-lg font-semibold text-foreground">{t('about.title')}</h1>
      <Card>
        <Card.Header>
          <h2 className="m-0 text-base font-semibold">{t('about.version')}</h2>
        </Card.Header>
        <Card.Body>
          {load.state === 'loading' && <Loading label={t('about.loading')} />}
          {load.state === 'failed' && (
            <div role="alert" className="flex flex-wrap items-center justify-between gap-3 text-error-text">
              <p className="m-0">{t('about.readFailed')}</p>
              <Button variant="secondary" onClick={() => void read()}>{t('system.retry')}</Button>
            </div>
          )}
          {load.state === 'ready' && (
            <dl className="m-0 grid grid-cols-2 gap-x-4 gap-y-2">
              <dt className="text-muted-foreground">{t('about.app')}</dt>
              <dd className="m-0 wrap-anywhere">{load.info.name}</dd>
              <dt className="text-muted-foreground">{t('about.version')}</dt>
              <dd className="m-0 wrap-anywhere">{load.info.version}</dd>
              <dt className="text-muted-foreground">{t('about.runtime')}</dt>
              <dd className="m-0 wrap-anywhere">{load.info.tauriVersion}</dd>
            </dl>
          )}
        </Card.Body>
      </Card>
      <p className="m-0 text-sm text-muted-foreground">
         {t('about.unavailable')}
      </p>
    </div>
  );
}
