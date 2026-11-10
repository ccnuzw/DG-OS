import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { hostCode } from './hostError';
import type { SessionConfigurationView } from './developmentCapabilities';
import type { AgentControls } from './agentTurnSettings';
export type Engine = 'codex' | 'claude';
export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
export interface Pending { id: string; kind: string; title: string; data: Json }
export interface AgentEvent { seq: number; kind: string; text: string | null; data: Json; run?: string | null }
export interface AgentSkillInput { name: string; path: string }
interface EventSync { reset: boolean; after_seq: number; first_seq: number }
export interface AgentSession { id: string; generation: string; engine: Engine; native_id: string | null; workspace: string; status: string; events: AgentEvent[]; pending: Pending[]; error: string | null; persistence_error?: string | null; history_truncated?: boolean; active_run?: string | null; configuration?: SessionConfigurationView | null; controls?: AgentControls; cli_version?: string | null; event_sync?: EventSync }
export type AgentOpenInput = { intent: 'create'; id: string; engine: Engine; task_id: string; project_revision: number; provider: { provider_id: string; model_id: string; provider_revision: number }; options: { workspace: string; resume_id: null; executable: null; model: null; trust_workspace: true } } | { intent: 'resume'; session_id: string; trust_workspace: boolean };
/** Host runtimes this account started or attached to in the current epoch, keyed by session id, with the last status seen. Closed runtimes are removed. */
export type LiveSessions = { [id: string]: string };

interface AgentRunnerState {
  session: AgentSession | null; selected: string | null;
  /** Account-epoch runtime observations only; never persisted or implicitly selected. */
  snapshots: Record<string, AgentSession>; unread: Record<string, boolean>;
  /** Stop requests remain pending until the authoritative runtime moves beyond their run. */
  interrupts: Record<string, { generation: string; runId: string }>;
  /** Stable host failure code (see hostError.ts) for the last operation, or null. */
  error: string | null; busy: boolean; acting: boolean; epoch: number; live: LiveSessions;
  open(input: AgentOpenInput): Promise<void>; attach(id: string): Promise<void>;
  action(command: string, args: { [key: string]: unknown }, onFailure?: (code: string) => void): Promise<boolean>; markSeen(id: string): void; detach(): void; reset(): void;
}

function track(live: LiveSessions, session: AgentSession): LiveSessions {
  const next = { ...live };
  if (session.status === 'closed') delete next[session.id]; else next[session.id] = session.status;
  return next;
}

function mergeEvents(previous: AgentSession | null | undefined, incoming: AgentSession): AgentSession | null {
  const sync = incoming.event_sync;
  if (!sync || sync.reset) return incoming;
  if (!previous || previous.id !== incoming.id || previous.generation !== incoming.generation
    || (previous.events[previous.events.length - 1]?.seq ?? 0) !== sync.after_seq
    || !Number.isSafeInteger(sync.first_seq) || sync.first_seq < 0 || sync.first_seq > sync.after_seq + 1
    || incoming.events.some((event, index) => event.seq !== sync.after_seq + index + 1)) return null;
  const retained = sync.first_seq > (previous.events[0]?.seq ?? 0)
    ? previous.events.filter(event => event.seq >= sync.first_seq) : previous.events;
  return { ...incoming, events: incoming.events.length ? [...retained, ...incoming.events] : retained };
}

export const useAgentRunner = create<AgentRunnerState>((set, get) => {
  let attempt = 0;
  const reads = new Map<string, number>();
  const timers = new Map<string, number>();
  const actions = new Set<string>();
  // Unknown admission blocks every new send in that runtime until matching run evidence arrives.
  const unknownSends = new Set<string>();
  const current = (id: string) => get().selected === id && get().session?.id === id ? get().session : get().snapshots[id];
  const isActing = (id: string | null) => id !== null && [...actions].some(key => key.startsWith(`${get().epoch}:${id}:${current(id)?.generation}:`));
  const invalidate = (id: string) => {
    const timer = timers.get(id);
    if (timer !== undefined) window.clearTimeout(timer);
    timers.delete(id);
    const read = (reads.get(id) ?? 0) + 1;
    reads.set(id, read);
    return read;
  };
  const changed = (before: AgentSession | null | undefined, after: AgentSession) => !before
    || before.generation !== after.generation || before.status !== after.status || before.error !== after.error
    || before.persistence_error !== after.persistence_error || before.active_run !== after.active_run
    || before.events.length !== after.events.length || before.events[before.events.length - 1]?.seq !== after.events[after.events.length - 1]?.seq
    || JSON.stringify(before.pending) !== JSON.stringify(after.pending);
  const observe = (value: AgentSession) => {
    const state = get();
    const previous = current(value.id);
    const sendPrefix = `${state.epoch}:${value.id}:${value.generation}:`;
    for (const key of unknownSends) {
      if (key.startsWith(sendPrefix) && (value.active_run === key.slice(sendPrefix.length) || value.events.some(event => event.run === key.slice(sendPrefix.length)))) unknownSends.delete(key);
    }
    // Neither an old read nor an eventual close acknowledgement can resurrect a closed generation.
    if (previous?.generation === value.generation && previous.status === 'closed' && value.status !== 'closed') return;
    const interrupts = { ...state.interrupts };
    const interrupted = interrupts[value.id];
    if (interrupted && (interrupted.generation !== value.generation || interrupted.runId !== value.active_run || value.status === 'closed')) delete interrupts[value.id];
    set({ snapshots: { ...state.snapshots, [value.id]: value }, live: track(state.live, value), interrupts,
      unread: changed(previous, value) ? { ...state.unread, [value.id]: true } : state.unread,
      ...(state.selected === value.id ? { session: value } : {}) });
  };
  const schedule = (id: string, epoch: number) => {
    if (epoch !== get().epoch || !(id in get().live) || timers.has(id)) return;
    const delay = ['starting', 'running'].includes(current(id)?.status ?? '') ? 150 : 600;
    timers.set(id, window.setTimeout(() => { timers.delete(id); void poll(id, epoch); }, delay));
  };
  const poll = async (id: string, epoch: number) => {
    if (epoch !== get().epoch) return;
    const read = invalidate(id);
    try {
      const previous = current(id);
      const cursor = previous ? { generation: previous.generation, after_seq: previous.events[previous.events.length - 1]?.seq ?? 0 } : undefined;
      let value = await invoke<AgentSession>('get_agent_session', { sessionId: id, ...(cursor ? { cursor } : {}) });
      if (epoch !== get().epoch || read !== reads.get(id)) return;
      let merged = value.id === id ? mergeEvents(current(id), value) : null;
      if (!merged) {
        // Read-only recovery: a missing event window never replays a user action.
        value = await invoke<AgentSession>('get_agent_session', { sessionId: id });
        if (epoch !== get().epoch || read !== reads.get(id)) return;
        merged = value.id === id ? mergeEvents(null, value) : null;
      }
      if (!merged) throw new Error('Invalid session snapshot');
      observe(merged);
    } catch (error) {
      if (epoch !== get().epoch || read !== reads.get(id)) return;
      const previous = current(id);
      if (previous) {
        const snapshot = { ...previous, pending: [] };
        set({ snapshots: { ...get().snapshots, [id]: snapshot }, ...(get().selected === id ? { session: snapshot } : {}) });
      }
      if (get().selected === id) set({ error: hostCode(error) });
    } finally {
      if (epoch === get().epoch && read === reads.get(id)) schedule(id, epoch);
    }
  };
  return {
    session: null, selected: null, snapshots: {}, unread: {}, interrupts: {}, error: null, busy: false, acting: false, epoch: 0, live: {},
    async open(input) {
      if (get().busy) return;
      const ticket = ++attempt;
      const epoch = get().epoch;
      const id = input.intent === 'create' ? input.id : input.session_id;
      set({ session: null, selected: id, busy: true, acting: false, error: null });
      try {
        const session = await invoke<AgentSession>('open_agent_session', { input });
        // A same-account open still belongs to this account after its surface detaches.
        if (epoch !== get().epoch) return;
        invalidate(session.id);
        observe(session);
        if (ticket === attempt) set({ session: get().snapshots[session.id], selected: session.id, acting: isActing(session.id) });
        void poll(session.id, epoch);
      } catch (error) { if (ticket === attempt && epoch === get().epoch) set({ error: hostCode(error) }); }
      finally { if (ticket === attempt && epoch === get().epoch) set({ busy: false }); }
    },
    // Attaching is observation only: it never starts or resumes a host runtime.
    async attach(id) {
      const ticket = ++attempt;
      const epoch = get().epoch;
      const cached = get().snapshots[id];
      // A cached transcript is useful immediately; approval authority requires this fresh read.
      set({ session: cached ? { ...cached, pending: [] } : null, selected: id, busy: true, acting: isActing(id), error: null });
      await poll(id, epoch);
      if (ticket === attempt && epoch === get().epoch) set({ busy: false, acting: isActing(id) });
    },
    async action(command, args, onFailure) {
      const epoch = get().epoch;
      const ticket = attempt;
      const input = args.input as { session_id?: unknown; generation?: unknown; request_id?: unknown; run_id?: unknown } | undefined;
      const target = input?.session_id ?? args.sessionId;
      if (typeof target !== 'string') return false;
      const closing = command === 'close_agent_session';
      const interrupting = command === 'interrupt_agent_session';
      const steering = command === 'steer_agent_turn';
      // Retained callbacks cannot send or approve against a different projection or generation.
      if (!closing && (target !== get().selected || target !== get().session?.id)) return false;
      const session = current(target);
      if (['send_agent_message', 'respond_agent_request', 'steer_agent_turn'].includes(command) && (typeof input?.generation !== 'string' || input.generation !== session?.generation)) return false;
      if (command === 'respond_agent_request' && (get().busy || session?.status === 'closed' || !session?.pending.some(item => item.id === input?.request_id))) return false;
      if (command === 'send_agent_message' && session?.persistence_error) { onFailure?.('storage_failed'); set({ error: 'storage_failed' }); return false; }
      if (interrupting && (get().busy || args.generation !== session?.generation || args.runId !== session?.active_run || !session?.active_run || !['running', 'waiting'].includes(session.status))) return false;
      if (steering && (get().busy || session?.engine !== 'codex' || !session.controls?.steering || input?.run_id !== session.active_run || !session.active_run || session.persistence_error || !['running', 'waiting'].includes(session.status))) return false;
      if (command === 'send_agent_message' && (isActing(target) || get().busy || session?.status !== 'idle' || session.pending.length)) return false;
      const runtime = session?.generation;
      const key = `${epoch}:${target}:${runtime}:${command === 'respond_agent_request' ? `request:${String(input?.request_id)}` : interrupting ? `interrupt:${String(args.runId)}` : steering ? `steer:${String(input?.run_id)}` : command}`;
      const sendKey = `${epoch}:${target}:${runtime}:${String(input?.request_id)}`;
      if (actions.has(key)) return false;
      if (interrupting && get().interrupts[target]?.generation === runtime && get().interrupts[target]?.runId === session?.active_run) return false;
      if (command === 'send_agent_message' && [...unknownSends].some(pending => pending.startsWith(`${epoch}:${target}:${runtime}:`))) { onFailure?.('unknown'); set({ error: 'unknown' }); return false; }
      actions.add(key);
      if (interrupting && session?.active_run) set({ interrupts: { ...get().interrupts, [target]: { generation: session.generation, runId: session.active_run } } });
      invalidate(target);
      if (get().selected === target) set({ error: null, acting: true });
      schedule(target, epoch);
      const sameRuntime = () => epoch === get().epoch && runtime === current(target)?.generation;
      const sameProjection = () => sameRuntime() && ticket === attempt && get().selected === target && (!interrupting || current(target)?.active_run === args.runId);
      try {
        const result = await invoke(command, args);
        if (!sameRuntime()) return false;
        if (closing) {
          if (result !== true) { if (sameProjection()) set({ error: 'unknown' }); return false; }
          invalidate(target);
          if (session) observe({ ...(current(target) ?? session), status: 'closed', pending: [] });
          else { const live = { ...get().live }; delete live[target]; set({ live }); }
          return true;
        }
        if (steering) {
          const receipt = result as { request_id?: string; run_id?: string; state?: string } | null;
          if (!receipt || receipt.request_id !== input?.request_id || receipt.run_id !== input?.run_id || receipt.state !== 'applied') {
            onFailure?.('unknown');
            if (sameProjection()) set({ error: 'unknown' });
            await poll(target, epoch);
            return false;
          }
        }
        // Refresh this runtime even if the user moved away, never the newly selected runtime.
        await poll(target, epoch);
        // Sending belongs to its original draft, not to whichever projection is visible at acknowledgement.
        return command === 'send_agent_message' || steering ? sameRuntime() : sameProjection();
      } catch (error) {
        if (sameRuntime()) {
          const code = hostCode(error);
          onFailure?.(code);
          if (command === 'send_agent_message' && code === 'unknown') unknownSends.add(sendKey);
          if (interrupting && code !== 'unknown' && get().interrupts[target]?.generation === runtime && get().interrupts[target]?.runId === args.runId) {
            const interrupts = { ...get().interrupts }; delete interrupts[target]; set({ interrupts });
          }
          if (sameProjection()) set({ error: code });
          void poll(target, epoch);
        }
        return false;
      } finally {
        actions.delete(key);
        if (epoch === get().epoch) set({ acting: isActing(get().selected) });
      }
    },
    markSeen(id) { if (get().unread[id]) set({ unread: { ...get().unread, [id]: false } }); },
    // Surface lifetime is independent of runtime subscriptions and account-scoped observations.
    detach() { ++attempt; set({ session: null, selected: null, error: null, busy: false, acting: false }); },
    reset() {
      ++attempt;
      for (const timer of timers.values()) window.clearTimeout(timer);
      timers.clear(); reads.clear(); actions.clear(); unknownSends.clear();
      set({ session: null, selected: null, snapshots: {}, unread: {}, interrupts: {}, error: null, busy: false, acting: false, live: {}, epoch: get().epoch + 1 });
    },
  };
});
