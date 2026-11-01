import type { ChatStreamEvent } from './api';

// WebSocket client for GET /api/v1/lan/stream (interface contract §14.7). The server only sends text frames and
// closes the socket when the client sends data, so this class never calls `send`.

export type LanStreamFrame =
  | { type: 'chat'; event: ChatStreamEvent }
  | { type: 'session_ended'; reason: string };

export type StreamStatus = 'connecting' | 'open' | 'reconnecting' | 'stopped';

/** Why the stream stopped for good. `replaced` and `host_stopped` can be restarted manually. */
export type StreamEnd =
  | { kind: 'session_lost'; reason: string | null }
  | { kind: 'replaced' }
  | { kind: 'host_stopped' };

export type SessionProbe = 'alive' | 'gone' | 'unknown';

export interface SocketLike {
  onopen: ((event: Event) => void) | null;
  onmessage: ((event: MessageEvent) => void) | null;
  onclose: ((event: CloseEvent) => void) | null;
  onerror: ((event: Event) => void) | null;
  close(code?: number): void;
}

export interface LanStreamOptions {
  url: string;
  createSocket: (url: string) => SocketLike;
  /** GET /session: 'gone' = 401, 'alive' = 200, 'unknown' = network failure. */
  probeSession: () => Promise<SessionProbe>;
  onFrame: (frame: LanStreamFrame) => void;
  onStatus: (status: StreamStatus) => void;
  /** Called on every open after the first one, and on a manual restart: events in between were dropped. */
  onResync: () => void;
  onEnd: (end: StreamEnd) => void;
  /** Reconnect delays in ms; the last entry repeats. */
  backoff?: readonly number[];
}

const DEFAULT_BACKOFF = [1000, 2000, 5000] as const;

function isFrame(value: unknown): value is LanStreamFrame {
  if (!value || typeof value !== 'object' || !('type' in value)) return false;
  if (value.type === 'chat') {
    if (!('event' in value) || !value.event || typeof value.event !== 'object') return false;
    return 'kind' in value.event && typeof value.event.kind === 'string';
  }
  return value.type === 'session_ended';
}


export class LanStream {
  private readonly options: LanStreamOptions;
  private readonly backoff: readonly number[];
  private socket: SocketLike | null = null;
  private timer: number | null = null;
  private attempt = 0;
  private stopped = true;
  private opened = false;
  private endedReason: string | null = null;
  private generation = 0;

  constructor(options: LanStreamOptions) {
    this.options = options;
    this.backoff = options.backoff ?? DEFAULT_BACKOFF;
  }

  /** Opens the connection. Starting again after `replaced`/`host_stopped` asks the owner to resync. */
  start(): void {
    if (!this.stopped) return;
    this.stopped = false;
    this.attempt = 0;
    this.endedReason = null;
    this.options.onStatus('connecting');
    this.connect();
  }

  /** Closes the connection without reporting an end. */
  stop(): void {
    this.stopped = true;
    this.generation += 1;
    this.clearTimer();
    this.detach();
    this.options.onStatus('stopped');
  }

  private clearTimer(): void {
    if (this.timer !== null) {
      window.clearTimeout(this.timer);
      this.timer = null;
    }
  }

  private detach(): void {
    const socket = this.socket;
    this.socket = null;
    if (!socket) return;
    socket.onopen = null;
    socket.onmessage = null;
    socket.onclose = null;
    socket.onerror = null;
    try {
      socket.close();
    } catch {
      // Already closed.
    }
  }

  private connect(): void {
    const socket = this.options.createSocket(this.options.url);
    this.socket = socket;
    socket.onopen = () => {
      if (this.socket !== socket) return;
      this.attempt = 0;
      this.options.onStatus('open');
      if (this.opened) this.options.onResync();
      this.opened = true;
    };
    socket.onmessage = (event) => {
      if (this.socket !== socket || typeof event.data !== 'string') return;
      let parsed: unknown;
      try {
        parsed = JSON.parse(event.data);
      } catch {
        return;
      }
      if (!isFrame(parsed)) return;
      if (parsed.type === 'session_ended') this.endedReason = parsed.reason;
      this.options.onFrame(parsed);
      if (parsed.type === 'session_ended') this.finish({ kind: 'session_lost', reason: parsed.reason });
    };
    socket.onclose = (event) => {
      if (this.socket !== socket) return;
      this.socket = null;
      this.handleClose(event.code);
    };
    // A failed handshake or a dropped connection ends in onclose, which carries the decision.
    socket.onerror = null;
  }

  private finish(end: StreamEnd): void {
    if (this.stopped) return;
    this.stopped = true;
    this.generation += 1;
    this.clearTimer();
    this.detach();
    this.options.onStatus('stopped');
    this.options.onEnd(end);
  }

  private handleClose(code: number): void {
    if (this.stopped) return;
    if (code === 4401) {
      this.finish({ kind: 'session_lost', reason: this.endedReason });
    } else if (code === 4409) {
      this.finish({ kind: 'replaced' });
    } else if (code === 1001) {
      this.finish({ kind: 'host_stopped' });
    } else {
      // 4413, 1006 (also a rejected handshake), 1008 and anything else: ask GET /session, then reconnect or re-pair.
      void this.recover();
    }
  }

  private async recover(): Promise<void> {
    const generation = this.generation;
    this.options.onStatus('reconnecting');
    const probe = await this.options.probeSession();
    if (generation !== this.generation || this.stopped) return;
    if (probe === 'gone') {
      this.finish({ kind: 'session_lost', reason: this.endedReason });
      return;
    }
    this.schedule(generation);
  }

  private schedule(generation: number): void {
    const delay = this.backoff[Math.min(this.attempt, this.backoff.length - 1)];
    this.attempt += 1;
    this.timer = window.setTimeout(() => {
      this.timer = null;
      if (generation !== this.generation || this.stopped) return;
      this.connect();
    }, delay);
  }
}
