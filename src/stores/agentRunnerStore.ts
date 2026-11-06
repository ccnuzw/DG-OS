import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { hostCode } from './hostError';
import type { CapabilitySnapshot } from './developmentCapabilities';
export type Engine = 'codex' | 'claude';
export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
export interface Pending { id: string; kind: string; title: string; data: Json }
export interface AgentEvent { seq: number; kind: string; text: string | null; data: Json; run?: string | null }
export interface AgentSession { id: string; engine: Engine; native_id: string | null; workspace: string; status: string; events: AgentEvent[]; pending: Pending[]; error: string | null; history_truncated?: boolean; active_run?: string | null; capabilities?: CapabilitySnapshot | null }
export interface AgentOpenInput { id: string; engine: Engine; task_id: string | null; capability_revision?: number | null; options: { workspace: string; resume_id: null; executable: null; model: string | null; trust_workspace: boolean } }
/** Host runtimes this account started or attached to in the current epoch, keyed by session id, with the last status seen. Closed runtimes are removed. */
export type LiveSessions = { [id: string]: string };

interface AgentRunnerState {
  session: AgentSession | null; selected: string | null;
  /** Stable host failure code (see hostError.ts) for the last operation, or null. */
  error: string | null; busy: boolean; epoch: number; live: LiveSessions;
  open(input: AgentOpenInput): Promise<void>; attach(id: string): Promise<void>;
  action(command: string, args: { [key: string]: unknown }): Promise<boolean>; detach(): void; reset(): void;
}

function track(live: LiveSessions, session: AgentSession): LiveSessions {
  const next = { ...live };
  if (session.status === 'closed') delete next[session.id]; else next[session.id] = session.status;
  return next;
}

export const useAgentRunner = create<AgentRunnerState>((set, get) => {
  let attempt = 0;
  let timer: number | undefined;
  const poll = async (id: string, ticket: number, epoch: number) => {
    if (ticket !== attempt || epoch !== get().epoch || get().selected !== id) return;
    clearTimeout(timer);
    try {
      const value = await invoke<AgentSession>('get_agent_session', { sessionId: id });
      if (ticket !== attempt || epoch !== get().epoch) return;
      if (get().session?.id === id && get().session?.status === 'closed' && value.status !== 'closed') return;
      set({ session: value, live: track(get().live, value) });
    } catch (error) { if (ticket === attempt && epoch === get().epoch) set({ error: hostCode(error), session: get().session ? { ...get().session!, pending: [] } : null }); }
    if (ticket === attempt && epoch === get().epoch && get().session?.status !== 'closed') timer = window.setTimeout(() => void poll(id, ticket, epoch), 600);
  };
  return {
    session: null, selected: null, error: null, busy: false, epoch: 0, live: {},
    async open(input) {
      if (get().busy) return;
      const ticket = ++attempt;
      const epoch = get().epoch;
      clearTimeout(timer);
      set({ session: null, selected: input.id, busy: true, error: null });
      try {
        const session = await invoke<AgentSession>('open_agent_session', { input });
        // A runtime started for a projection that was detached meanwhile still exists, so it stays tracked.
        if (epoch !== get().epoch) return;
        set({ live: track(get().live, session) });
        if (ticket !== attempt) return;
        set({ session, selected: session.id });
        void poll(session.id, ticket, epoch);
      } catch (error) { if (ticket === attempt && epoch === get().epoch) set({ error: hostCode(error) }); }
      finally { if (ticket === attempt && epoch === get().epoch) set({ busy: false }); }
    },
    // Reads the host snapshot of a session that is still running. It never starts a CLI process.
    async attach(id) {
      const ticket = ++attempt;
      const epoch = get().epoch;
      clearTimeout(timer);
      set({ session: null, selected: id, busy: true, error: null });
      await poll(id, ticket, epoch);
      if (ticket === attempt && epoch === get().epoch) set({ busy: false });
    },
    async action(command, args) {
      const epoch = get().epoch;
      const ticket = attempt;
      const input = args.input as { session_id?: unknown; request_id?: unknown } | undefined;
      const target = input?.session_id ?? args.sessionId;
      // A callback retained by an old conversation cannot send input or approve requests for the new projection.
      if (command !== 'close_agent_session' && (typeof target !== 'string' || target !== get().selected || target !== get().session?.id)) return false;
      if (command === 'respond_agent_request' && (get().session?.status === 'closed' || !get().session?.pending.some(item => item.id === input?.request_id))) return false;
      set({ error: null });
      try {
        const result = await invoke(command, args);
        if (epoch !== get().epoch) return false;
        if (command === 'close_agent_session') {
          if (result !== true) { set({ error: 'unknown' }); return false; }
          const next = { ...get().live };
          delete next[String(args.sessionId)];
          set({ live: next });
        }
        if (ticket !== attempt) return false;
        // interrupt/close acknowledgement is not a terminal-state event.
        if (get().selected) await poll(get().selected!, ticket, epoch);
        return ticket === attempt && epoch === get().epoch;
      } catch (error) { if (epoch === get().epoch && ticket === attempt) set({ error: hostCode(error) }); return false; }
    },
    // Stops only the projection and its polling. The host runtime keeps running and stays in `live`.
    detach() { ++attempt; clearTimeout(timer); set({ session: null, selected: null, error: null, busy: false }); },
    reset() { ++attempt; clearTimeout(timer); set({ session: null, selected: null, error: null, busy: false, live: {}, epoch: get().epoch + 1 }); },
  };
});
