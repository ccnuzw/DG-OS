import { extractErrorCode } from '@dgos/ui';
import { invoke } from '@tauri-apps/api/core';

// V1-FR-010 host commands (interface contract §14.8): typed wrappers for the LAN management commands used by the
// Settings LAN tab. The host decides who is signed in; no command takes a user id or token. A failed command
// rejects with a `LanCommandError`, whose numeric `code` is what `handleError` reads. Access codes returned by
// `issueLanAccessCode` are plaintext and are returned once: callers keep them in memory only.

export type LanServiceState = 'stopped' | 'starting' | 'running' | 'failed';

export interface LanStatus {
  enabled: boolean;
  state: LanServiceState;
  /** `IP:port` while running. */
  address: string | null;
  preferred_port: number;
  port_fallback: boolean;
  /** 1010001 start failed, 1010002 no available port, 1010003 no network. */
  failure_code: number | null;
  session_count: number;
  /** Unix seconds. */
  code_expires_at: number | null;
}

export interface LanIssuedCode {
  code: string;
  expires_at: number;
  pair_url: string;
}

export interface LanDevice {
  session_id: string;
  short_id: string;
  label: string;
  peer_ip: string;
  created_at: number;
  last_active_at: number;
}

export type LanAuditEventKind =
  | 'pair_succeeded'
  | 'pair_rejected'
  | 'pair_rate_limited'
  | 'code_voided'
  | 'session_revoked'
  | 'request_rejected';

export interface LanAuditEvent {
  id: string;
  first_at: number;
  last_at: number;
  event: LanAuditEventKind;
  reason: string | null;
  peer_ip: string | null;
  session_short_id: string | null;
  error_code: number | null;
  count: number;
}

export interface LanAuditPage {
  items: LanAuditEvent[];
  next_cursor: string | null;
}

interface CommandResponse<T> {
  success: boolean;
  data?: T | null;
  error?: { code?: number; message?: string; detail?: unknown } | null;
}

export class LanCommandError extends Error {
  readonly code: number | null;

  constructor(code: number | null) {
    super(code === null ? 'lan command failed' : `lan command failed (${code})`);
    this.name = 'LanCommandError';
    this.code = code;
  }
}

/** `allowNull`: a successful response whose `data` is `null` is an answer, not a failure (`issue_lan_access_code`). */
async function call<T>(command: string, args?: Record<string, unknown>, allowNull = false): Promise<T> {
  let response: CommandResponse<T> | null | undefined;
  try {
    response = args === undefined ? await invoke<CommandResponse<T>>(command) : await invoke<CommandResponse<T>>(command, args);
  } catch (error) {
    throw new LanCommandError(extractErrorCode(error));
  }
  if (!response || response.success !== true || response.data === undefined || (response.data === null && !allowNull)) {
    const code = response?.error?.code;
    throw new LanCommandError(typeof code === 'number' ? code : null);
  }
  return response.data as T;
}

export const getLanStatus = (): Promise<LanStatus> => call('get_lan_status');
export const setLanEnabled = (enabled: boolean): Promise<LanStatus> => call('set_lan_enabled', { enabled });
export const setLanPort = (port: number): Promise<LanStatus> => call('set_lan_port', { port });
/** `null` when the service is not running or nobody is signed in. */
export const issueLanAccessCode = (): Promise<LanIssuedCode | null> => call('issue_lan_access_code', undefined, true);
export const listLanDevices = (): Promise<LanDevice[]> => call('list_lan_devices');
export const disconnectLanDevice = (sessionId: string): Promise<boolean> => call('disconnect_lan_device', { sessionId });
export const disconnectAllLanDevices = (): Promise<number> => call('disconnect_all_lan_devices');
export const listLanAuditEvents = (cursor?: string, limit?: number): Promise<LanAuditPage> =>
  call('list_lan_audit_events', { input: { ...(cursor === undefined ? {} : { cursor }), ...(limit === undefined ? {} : { limit }) } });

export const LAN_PORT_MIN = 1024;
export const LAN_PORT_MAX = 65535;
/** Host error for a port outside 1024..65535 or one the Fetch standard forbids (D107). */
export const LAN_PORT_UNAVAILABLE = 1010012;

export const LAN_FAILURE_TEXT: Record<number, string> = {
  1010001: '服务启动失败',
  1010002: '首选端口及其后 19 个端口都被占用',
  1010003: '没有可用的局域网连接',
};
