import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button, Modal } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { readCliProviderRevision, type CliProviderRevision } from '../../stores/developmentCapabilities';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration } from '../../stores/developmentSelection';
import { errorText } from './shared';

type Preview = { diff: string; revision: CliProviderRevision; resolve: (accepted: boolean) => void };
export function useCommitMessageGenerator(projectId: string | null, taskId: string | null) {
  const { t } = useLocale();
  const accountEpoch = useDevelopment(state => state.epoch);
  const [preview, setPreview] = useState<Preview | null>(null);
  const pending = useRef<Preview | null>(null);
  const active = useRef<{ id: string; abort: () => void } | null>(null);
  const lifecycle = useRef(0);
  function decide(accepted: boolean) { pending.current?.resolve(accepted); pending.current = null; setPreview(null); }
  useEffect(() => { setPreview(null); return () => { ++lifecycle.current; pending.current?.resolve(false); pending.current = null; active.current?.abort(); }; }, [projectId, taskId, accountEpoch]);
  async function generate(diff: string, signal: AbortSignal): Promise<string | null> {
    if (!projectId || signal.aborted || active.current || pending.current) return null;
    if (!diff.trim() || new TextEncoder().encode(diff).length > 65536) throw Error(t('commitDraft.diffLimit'));
    const epoch = useDevelopment.getState().epoch, generation = contextGeneration(), lifetime = lifecycle.current;
    const valid = () => !signal.aborted && lifetime === lifecycle.current && epoch === useDevelopment.getState().epoch && generation === contextGeneration();
    const revision = await readCliProviderRevision(projectId);
    if (!valid()) return null;
    if (!revision) throw Error(t('commitDraft.providerRequired'));
    let cancelPreview = () => {};
    // Production targets ES2020; Promise.withResolvers is not available on that runtime.
    const accepted = await new Promise<boolean>(resolve => {
      const value = { diff, revision, resolve }; pending.current = value; setPreview(value);
      cancelPreview = () => { if (pending.current === value) decide(false); };
      signal.addEventListener('abort', cancelPreview, { once: true });
    });
    signal.removeEventListener('abort', cancelPreview);
    if (!accepted || !valid()) return null;
    const id = crypto.randomUUID();
    const abort = () => { void invoke('cancel_development_commit_message', { requestId: id }).catch(() => {}); };
    active.current = { id, abort }; signal.addEventListener('abort', abort, { once: true });
    try {
      const text = await invoke<string>('generate_development_commit_message', { input: { project_id: projectId, task_id: taskId, request_id: id, expected_staged_diff: diff, expected_provider_revision: revision.revision } });
      return valid() ? text : null;
    } catch (error) {
      if (!valid()) return null;
      const code = String(error);
      throw Error(code.startsWith('commit_generation_failed:') ? t('commitDraft.failed', { code: code.split(':')[1] }) : errorText(code));
    } finally {
      signal.removeEventListener('abort', abort);
      if (active.current?.id === id) active.current = null;
    }
  }
  return { generate, dialog: <Modal open={Boolean(preview)} onOpenChange={open => { if (!open) decide(false); }} title={t('commitDraft.title')} footer={<><Button variant="secondary" onClick={() => decide(false)}>{t('common.cancel')}</Button><Button onClick={() => decide(true)}>{t('commitDraft.generate')}</Button></>}>
    <p>{t('commitDraft.disclosure')}</p>
    <p className="break-all text-xs">{preview?.revision.configuration.provider.provider_id} / {preview?.revision.configuration.provider.model_id}</p>
    <pre className="dev-diff" tabIndex={0}>{preview?.diff}</pre>
  </Modal> };
}
