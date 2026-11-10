import { useCallback, useEffect, useRef, useState } from 'react';
import { useLocale } from '../../i18n';
import { useAgentRunner, type AgentSession } from '../../stores/agentRunnerStore';
import { useDevelopment } from '../../stores/developmentStore';

export interface AgentDesktopNotificationOptions {
  onSelectSession: (sessionId: string) => void;
  onActivate?: () => void;
  activeSessionId?: string | null;
}

/** Opt-in OS notifications contain no conversation title, message, tool input or workspace path. */
export function useAgentDesktopNotifications({ onSelectSession, onActivate, activeSessionId = null }: AgentDesktopNotificationOptions) {
  const { t } = useLocale();
  const owner = useDevelopment(state => state.workspaceOwner);
  const epoch = useDevelopment(state => state.epoch);
  const supported = typeof Notification !== 'undefined';
  const storageKey = owner ? `dgos:agent-notifications:${owner}` : null;
  const [preference, setPreference] = useState<{ key: string | null; enabled: boolean }>({ key: null, enabled: false });
  const [permission, setPermission] = useState<NotificationPermission | 'unsupported'>(supported ? Notification.permission : 'unsupported');
  const latest = useRef({ onSelectSession, onActivate, activeSessionId, owner, epoch });
  latest.current = { onSelectSession, onActivate, activeSessionId, owner, epoch };
  const mounted = useRef(true);
  const permissionRequest = useRef(0);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++permissionRequest.current; }; }, []);
  const enabled = supported && permission === 'granted' && preference.key === storageKey && preference.enabled && Boolean(owner);

  useEffect(() => {
    let enabled = false;
    try { enabled = Boolean(storageKey && localStorage.getItem(storageKey) === 'true'); } catch { /* Storage can be disabled; opt-in still works for this mount. */ }
    setPreference({ key: storageKey, enabled });
    setPermission(supported ? Notification.permission : 'unsupported');
  }, [storageKey, supported]);

  useEffect(() => {
    if (!supported) return;
    const refreshPermission = () => setPermission(Notification.permission);
    document.addEventListener('visibilitychange', refreshPermission);
    window.addEventListener('focus', refreshPermission);
    return () => { document.removeEventListener('visibilitychange', refreshPermission); window.removeEventListener('focus', refreshPermission); };
  }, [supported]);

  const setEnabled = useCallback(async (value: boolean) => {
    if (!supported || !storageKey) return;
    const ticket = ++permissionRequest.current;
    let granted = Notification.permission;
    if (value && granted === 'default') {
      try { granted = await Notification.requestPermission(); } catch { granted = Notification.permission; }
    }
    if (!mounted.current || ticket !== permissionRequest.current || latest.current.owner !== owner || latest.current.epoch !== epoch) return;
    const enabled = value && granted === 'granted';
    setPermission(granted);
    setPreference({ key: storageKey, enabled });
    try { localStorage.setItem(storageKey, String(enabled)); } catch { /* Keep the explicit choice in memory only. */ }
  }, [supported, storageKey, owner, epoch]);

  useEffect(() => {
    if (!enabled) return;
    let previous = useAgentRunner.getState().snapshots;
    const notices = new Set<Notification>();
    const delivered = new Set<string>();
    const notify = (session: AgentSession, kind: 'approval' | 'completed', identity: string) => {
      const key = `${session.id}:${session.generation}:${kind}:${identity}`;
      if (delivered.has(key)) return;
      delivered.add(key);
      if (Notification.permission !== 'granted' || (!document.hidden && document.hasFocus() && latest.current.activeSessionId === session.id)) return;
      try {
        const notice = new Notification(t(kind === 'approval' ? 'workbenchNavigation.notificationApproval' : 'workbenchNavigation.notificationCompleted'), { tag: `dgos-agent:${key}` });
        notices.add(notice);
        notice.onclose = () => notices.delete(notice);
        notice.onclick = () => {
          notice.close();
          const state = useDevelopment.getState();
          if (state.epoch !== epoch || state.workspaceOwner !== owner || !state.sessions.some(row => row.id === session.id)) return;
          window.focus();
          latest.current.onActivate?.();
          latest.current.onSelectSession(session.id);
        };
      } catch { /* Unsupported platform delivery must never affect execution or approvals. */ }
    };
    const unsubscribe = useAgentRunner.subscribe(state => {
      if (latest.current.owner !== owner || latest.current.epoch !== epoch) return;
      for (const snapshot of Object.values(state.snapshots)) {
        const before = previous[snapshot.id];
        // First observations and new runtime generations establish a baseline, not historical alerts.
        if (!before || before.generation !== snapshot.generation || before === snapshot) continue;
        const pending = new Set(before.pending.map(request => request.id));
        for (const request of snapshot.pending) if (!pending.has(request.id)) notify(snapshot, 'approval', request.id);
        const lastSeq = before.events[before.events.length - 1]?.seq ?? 0;
        for (const event of snapshot.events) {
          if (event.seq <= lastSeq || !['result', 'turn/completed'].includes(event.kind)) continue;
          const data = event.data && typeof event.data === 'object' && !Array.isArray(event.data) ? event.data : {};
          if (['completed', 'failed', 'cancelled'].includes(String(data.run_status))) notify(snapshot, 'completed', `${event.run ?? ''}:${event.seq}`);
        }
      }
      previous = state.snapshots;
    });
    return () => { unsubscribe(); for (const notice of notices) notice.close(); };
  }, [enabled, epoch, owner, t]);

  return { supported, enabled, permission, setEnabled };
}
