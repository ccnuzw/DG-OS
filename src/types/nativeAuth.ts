export type NativeAuthKind = 'login' | 'logout';
export interface NativeAuthRequest { session_id: string; generation: string; request_id: string }
export interface NativeAuthSnapshot extends NativeAuthRequest {
  server_name: string; kind: NativeAuthKind;
  state: 'starting' | 'waiting_callback' | 'stopping' | 'succeeded' | 'failed' | 'stopped' | 'auth_unknown';
  reason: 'cancelled' | 'expired' | 'revoked' | 'command_failed' | 'interaction_unavailable' | 'cleanup_pending' | null;
  credential_outcome: 'confirmed' | 'unknown'; authorization_url: string | null;
  callback_required: boolean; expires_at: number; exit_code: number | null;
}
