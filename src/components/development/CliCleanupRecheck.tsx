import { useEffect, useRef, useState } from 'react';
import { Button } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { recheckCliCleanup } from '../../stores/developmentCapabilities';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { hostCode } from '../../stores/hostError';
import { errorText } from './shared';

export function CliCleanupRecheck() {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const { generation } = useContextSelection();
  const [state, setState] = useState<{ epoch: number; generation: number; status: 'busy' | 'checked' | 'error'; error?: string } | null>(null);
  const pending = useRef(false);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const visible = state?.epoch === epoch && state.generation === generation ? state : null;
  async function recheck() {
    if (pending.current) return;
    pending.current = true;
    setState({ epoch, generation, status: 'busy' });
    try {
      await recheckCliCleanup();
      if (mounted.current && epoch === useDevelopment.getState().epoch && generation === contextGeneration()) setState({ epoch, generation, status: 'checked' });
    } catch (error) {
      if (mounted.current && epoch === useDevelopment.getState().epoch && generation === contextGeneration()) setState({ epoch, generation, status: 'error', error: hostCode(error) });
    } finally { pending.current = false; }
  }
  return <div className="dev-review-stack">
    <Button size="xs" variant="secondary" disabled={visible?.status === 'busy'} onClick={() => void recheck()}>{t('dev.recheckCleanup')}</Button>
    {visible?.status === 'checked' ? <p role="status" className="m-0 text-xs text-ink-secondary">{t('dev.cleanupRechecked')}</p> : null}
    {visible?.status === 'error' ? <p role="alert" className="m-0 text-xs text-status-error">{errorText(visible.error ?? 'unknown')}</p> : null}
  </div>;
}
