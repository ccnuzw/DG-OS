import { useCallback, useEffect, useRef, useState } from 'react';
import { getName, getTauriVersion, getVersion } from '@tauri-apps/api/app';
import { Button, Loading } from '@dgos/ui';
import { usePageError } from '../pageError';
import { t, useLocale as usePageLocale } from '../i18n';
import './about-settings.css';

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
    <div className="settings-content about-settings">
      <h1 className="about-title">{t('about.title')}</h1>
      <section className="about-information" aria-label={t('about.app')}>
        {load.state === 'loading' && <Loading label={t('about.loading')} />}
        {load.state === 'failed' && (
          <div role="alert" className="about-error">
            <p>{t('about.readFailed')}</p>
            <Button variant="secondary" onClick={() => void read()}>{t('system.retry')}</Button>
          </div>
        )}
        {load.state === 'ready' && (
          <>
            <header className="about-identity">
              <span className="about-mark" aria-hidden="true">{Array.from(load.info.name)[0]?.toLocaleUpperCase()}</span>
              <div className="about-name"><span>{t('about.app')}</span><h2>{load.info.name}</h2></div>
            </header>
            <dl className="about-versions">
              <div><dt>{t('about.version')}</dt><dd>{load.info.version}</dd></div>
              <div><dt>{t('about.runtime')}</dt><dd>{load.info.tauriVersion}</dd></div>
            </dl>
          </>
        )}
      </section>
      <p className="about-availability">{t('about.unavailable')}</p>
    </div>
  );
}
