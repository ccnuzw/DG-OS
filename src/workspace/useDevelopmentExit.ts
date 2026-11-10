import { useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { toast } from '@dgos/ui';
import { t } from '../i18n';
import { confirmDiscard } from './closeCoordinator';
import { flushDesktopState } from '../stores/developmentDesktop';
import { useAgentRunner } from '../stores/agentRunnerStore';
import { processActive, useDevelopmentRuntime } from '../stores/developmentRuntime';

/** The host owns the exit nonce and performs process cleanup only after this receipt. */
export function useDevelopmentExit(): void {
  useEffect(() => {
    if (!('__TAURI_INTERNALS__' in window)) return;
    let disposed = false;
    let pending: string | null = null;
    const subscription = listen<{ request_id: string }>('development:prepare-exit', async event => {
      if (disposed || pending || !event.payload?.request_id) return;
      const requestId = event.payload.request_id;
      pending = requestId;
      let proceed = false;
      try {
        const runner = useAgentRunner.getState();
        const runtime = useDevelopmentRuntime.getState();
        const sessions = Object.entries(runner.live).filter(([, status]) => status !== 'closed').map(([id, status]) => `${runner.snapshots[id]?.engine ?? 'CLI'} · ${id} · ${t(`agent.state.${status}`)}`);
        const processes = [...new Map([...Object.values(runtime.details), ...runtime.processes].map(process => [process.id, process])).values()].filter(processActive).map(process => `${process.kind} · ${process.id} · ${process.status}`);
        const accepted = await confirmDiscard({ title: t('desktop.exitTitle'), content: [t('desktop.exitDetail'), ...sessions, ...processes].join('\n'), confirmText: t('desktop.exit') });
        if (accepted && !disposed) {
          proceed = await flushDesktopState();
          if (!proceed) toast.error(t('desktop.savedFailure'));
        }
      } finally {
        try { await invoke('reply_development_exit', { input: { request_id: requestId, proceed: proceed && !disposed } }); }
        catch { if (!disposed) toast.error(t('desktop.exitFailed')); }
        if (pending === requestId) pending = null;
      }
    });
    return () => { disposed = true; void subscription.then(unlisten => unlisten()); };
  }, []);
}
