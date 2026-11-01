import { useEffect, useMemo, useRef, useState } from 'react';
import { Button } from '@dgos/ui/components/Button';
import { LanApiError, LanNetworkError, createLanApi, type LanSessionStatus } from './api';
import { ChatView } from './ChatView';
import { NETWORK_ERROR_TEXT, SESSION_LOST_TEXT, pairErrorText } from './messages';
import { PairingView } from './PairingView';

export interface AppProps {
  /** Code taken from the URL fragment before rendering; the fragment is already gone from the address bar. */
  initialCode?: string | null;
}

type View =
  | { kind: 'boot' }
  | { kind: 'offline' }
  | { kind: 'pairing'; notice?: string; autoSubmit?: boolean; code?: string }
  | { kind: 'chat'; session: LanSessionStatus };

export function App({ initialCode = null }: AppProps) {
  const [view, setView] = useState<View>(
    initialCode === null ? { kind: 'boot' } : { kind: 'pairing', autoSubmit: true, code: initialCode },
  );
  const [attempt, setAttempt] = useState(0);
  const booted = useRef<number | null>(null);

  // Any 401 with code 1010006 from a chat request lands here.
  const onLost = useRef<() => void>(() => {});
  useEffect(() => {
    onLost.current = () => setView((current) => (current.kind === 'chat' ? { kind: 'pairing', notice: SESSION_LOST_TEXT } : current));
  });
  const api = useMemo(() => createLanApi(() => onLost.current()), []);

  useEffect(() => {
    if (initialCode !== null || booted.current === attempt) return;
    booted.current = attempt;
    api.getSession().then(
      (session) => setView({ kind: 'chat', session }),
      (failure: unknown) => {
        if (failure instanceof LanApiError && failure.status === 401) setView({ kind: 'pairing' });
        else if (failure instanceof LanApiError && failure.code === 1010007) setView({ kind: 'pairing', notice: pairErrorText(failure) });
        else if (failure instanceof LanNetworkError || failure instanceof LanApiError) setView({ kind: 'offline' });
        else setView({ kind: 'offline' });
      },
    );
  }, [api, initialCode, attempt]);

  async function logout() {
    try {
      await api.logout();
    } catch {
      // The cookie may already be gone; either way this device goes back to pairing.
    }
    setView({ kind: 'pairing' });
  }

  if (view.kind === 'boot') {
    return (
      <main className="flex flex-1 items-center justify-center p-6 text-muted-foreground" role="status">
        正在连接…
      </main>
    );
  }
  if (view.kind === 'offline') {
    return (
      <main className="mx-auto flex w-full max-w-md flex-1 flex-col items-center justify-center gap-4 p-6">
        <p role="alert">{NETWORK_ERROR_TEXT}</p>
        <Button
          onClick={() => {
            setView({ kind: 'boot' });
            setAttempt((n) => n + 1);
          }}
        >
          重试
        </Button>
      </main>
    );
  }
  if (view.kind === 'pairing') {
    return (
      <PairingView
        api={api}
        initialCode={view.code}
        autoSubmit={view.autoSubmit}
        notice={view.notice}
        onPaired={(session) => setView({ kind: 'chat', session })}
      />
    );
  }
  return (
    <ChatView
      api={api}
      session={view.session}
      onSessionEnded={(notice) => setView({ kind: 'pairing', notice: notice || undefined })}
      onLogout={() => void logout()}
    />
  );
}
