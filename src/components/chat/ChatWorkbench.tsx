import { t, useLocale as usePageLocale } from '../../i18n';
import { useEffect, useRef, useState } from 'react';
import { Button, Input, Modal } from '@dgos/ui';
import { useChatStore, type ChatSession, type PendingDestination } from '../../stores/chatStore';
import { ChatNotice } from './ChatNotice';
import Composer from './Composer';
import MessageList from './MessageList';
import SessionSidebar from './SessionSidebar';
import { waitForChatTerminal } from '../../workspace/closeCoordinator';
import { useSurfaceStatus } from '../../workspace/SurfaceContext';


function SessionHeader({ onClose, onDelete }: { onClose: (session: ChatSession) => void; onDelete: (sessionId: string) => Promise<string | null> }) {
  usePageLocale();
  const session = useChatStore(state => state.activeSession);
  const rename = useChatStore(state => state.renameSession);
  const [renaming, setRenaming] = useState(false);
  const [title, setTitle] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const renameButton = useRef<HTMLButtonElement>(null);
  useEffect(() => { setRenaming(false); setError(null); }, [session?.id]);
  if (!session) return null;
  const finishRename = () => { setRenaming(false); setError(null); requestAnimationFrame(() => renameButton.current?.focus({ preventScroll: true })); };
  const submit = async () => { const failure = await rename(session.id, title); if (failure) { setError(failure); return; } finishRename(); };
  return <header className="chat-session-header chat-reading-column">
    {renaming ? <form className="flex min-w-0 flex-wrap justify-end gap-2" onSubmit={event => { event.preventDefault(); void submit(); }} onKeyDown={event => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); finishRename(); } }}><Input aria-label={t('chat.title')} value={title} error={error ?? undefined} autoFocus onChange={event => setTitle(event.target.value)} /><Button type="submit">{t('chat.saveTitle')}</Button><Button variant="secondary" onClick={finishRename}>{t('chat.cancel')}</Button></form> : <><Button ref={renameButton} size="sm" variant="ghost" onClick={() => { setTitle(session.title); setError(null); setRenaming(true); }}>{t('chat.rename')}</Button><Button size="sm" variant="ghost" onClick={() => onClose(session)}>{t('chat.closeSession')}</Button><Button size="sm" variant="ghost" onClick={() => setDeleting(true)}>{t('chat.delete')}</Button></>}
    {error && !renaming ? <ChatNotice tone="error" className="w-full">{error}</ChatNotice> : null}
    <Modal open={deleting} onOpenChange={setDeleting} title={t('chat.delete')} icon="warning" description={t('chat.deleteConfirm')} confirmText={t('chat.delete')} confirmVariant="danger" onConfirm={async () => { const failure = await onDelete(session.id); if (failure) setError(failure); else setDeleting(false); }} />
  </header>;
}
function DestinationDialog() {
  usePageLocale();
  const pending = useChatStore(state => state.pendingDestination);
  const confirm = useChatStore(state => state.confirmDestination);
  const dismiss = useChatStore(state => state.dismissDestination);
  // Keep the last request so the dialog can fade through a close without its text jumping.
  const shown = useRef<PendingDestination | null>(null);
  if (pending) shown.current = pending;
  return (
    <Modal
      open={pending !== null}
      onOpenChange={open => { if (!open) dismiss(pending ?? undefined); }}
      title={t('chat.destination')}
      icon="warning"
      description={<span>{t('chat.destinationBefore')}{shown.current?.host ? t('chat.destinationHost', { host: shown.current.host }) : ''}{t('chat.destinationAfter')}</span>}
      confirmText={t('chat.confirmSend')}
      onConfirm={confirm}
    />
  );
}

export default function ChatWorkbench() {
  usePageLocale();
  const session = useChatStore(state => state.activeSession);
  const stale = useChatStore(state => state.activeStale);
  const notice = useChatStore(state => state.notice);
  const loadSessions = useChatStore(state => state.loadSessions);
  const dismissNotice = useChatStore(state => state.dismissNotice);
  const createSession = useChatStore(state => state.createSession);
  const closeSession = useChatStore(state => state.closeSession);
  const deleteSession = useChatStore(state => state.deleteSession);
  const stopSession = useChatStore(state => state.stopSession);
  const generations = useChatStore(state => state.generations);
  const dirty = useChatStore(state => Object.values(state.sessionDrafts).some(Boolean) || Object.values(state.sessionTargetDrafts).some(Boolean));
  useSurfaceStatus({ dirty, running: Object.keys(generations).length > 0 });
  const [closeTarget, setCloseTarget] = useState<ChatSession | null>(null);
  useEffect(() => { void loadSessions(); }, [loadSessions]);
  const requestClose = (target: ChatSession) => { if (generations[target.id]) setCloseTarget(target); else closeSession(target.id); };
  return <div className="chat-workbench">
    <SessionSidebar onClose={requestClose} />
    <section id="chat-session-panel" role={session ? 'tabpanel' : undefined} aria-labelledby={session ? `chat-session-tab-${session.id}` : undefined} aria-label={session ? undefined : t('chat.chat')} className="chat-surface">
      {session ? <><div className="chat-session-actions"><SessionHeader onClose={requestClose} onDelete={deleteSession} /></div>{stale ? <div className="chat-feedback chat-reading-column" tabIndex={0}><ChatNotice tone="error">{t('chat.stale')}</ChatNotice></div> : null}<MessageList />{notice ? <div className="chat-feedback chat-reading-column" tabIndex={0}><ChatNotice tone={notice.tone} onDismiss={dismissNotice}>{notice.text}</ChatNotice></div> : null}<Composer key={session.id} /></> : <div className="chat-start chat-reading-column"><h2 className="m-0 text-lg font-semibold">{t('chat.start')}</h2><p className="m-0 text-sm text-ink-secondary">{t('chat.startHelp')}</p><Button onClick={() => void createSession()}>{t('chat.newSession')}</Button></div>}
    </section>
    <DestinationDialog />
    <Modal open={closeTarget !== null} onOpenChange={open => { if (!open) setCloseTarget(null); }} title={t('chat.closeSession')} icon="warning" description={t('chat.closeRunningConfirm')} confirmText={t('chat.closeSession')} confirmVariant="danger" onConfirm={async () => { if (closeTarget) { const id = closeTarget.id; await stopSession(id); if (!await waitForChatTerminal([id])) throw new Error('stop_unconfirmed'); closeSession(id); setCloseTarget(null); } }} />
  </div>;
}
