export type NativeTarget = 'Codex' | 'Claude';
export type NativeScope = 'OsUser' | 'ProjectShared' | 'ProjectLocal';
export interface NativeSelection { target: NativeTarget; scope: NativeScope; project_id: string | null }
export type NativeOwner = 'Loose' | 'Plugin' | 'Managed' | 'Synced' | 'Symlink' | 'Unknown';
export type NativeState = 'Previewed' | 'Prepared' | 'Publishing' | 'Committed' | 'Conflict' | 'Unknown' | 'RecoveryRequired' | 'Cancelled';
export interface NativeReceipt { operation_id: string; state: NativeState; pending_activation: boolean; recovery_material_retained: boolean }
export interface NativePreview { receipt: NativeReceipt; target: NativeTarget; scope: NativeScope; expires_at: number }
export interface NativeInstance { id: string; revision: string }
export interface NativeSource { source_hash: string; commit: string | null }
export interface NativeResource { path: string; hash: string; bytes: number; executable: boolean }
export interface NativeDependencies { declared: boolean; tools: { program: string; version_requirement: string | null }[]; packages: string[]; required_tools: string[] }
export interface NativeMetadata { name: string; description: string; license: string | null; dependencies: NativeDependencies; dynamic_commands: boolean }
export interface NativeSkill { instance: NativeInstance; target: NativeTarget; scope: NativeScope; slug: string; owner: NativeOwner; metadata: NativeMetadata; resources: NativeResource[]; source: NativeSource | null; can_update: boolean }
export interface NativeReadonlyEntry { id: string; slug: string; link_path: string; target: string; resolved_target: string | null; status: 'within_authorized_root' | 'outside_authorized_root' | 'unresolved' }
export interface NativeSkills { instances: NativeSkill[]; readonly_entries: NativeReadonlyEntry[]; missing: boolean; incomplete: boolean }
// Session evidence is separate from NativeSkill: it carries no instance or file authorization.
export interface NativeObservedSkill {
  name: string;
  enabled: boolean | null;
  classification: 'skill' | 'builtin_command' | 'unknown';
  builtin: boolean | null;
  path: string | null;
  scope: 'user' | 'repo' | 'system' | 'admin' | null;
  plugin_id: string | null;
}
export interface DiscoverNativeSkills extends NativeSelection { query: string }
export interface GetNativeSkill extends NativeSelection { slug: string }
export type NativePackage = { kind: 'zip'; bytes: number[]; subpath: string | null } | { kind: 'github'; input: string };
export interface PreviewNativeSkill extends NativeSelection { action: 'install' | 'update' | 'remove'; slug: string; expected: NativeInstance | null; source: NativePackage | null }
export interface NativeSkillPreview extends NativeSelection { receipt: NativeReceipt; slug: string; before_hash: string | null; after_hash: string | null; source: NativeSource | null; resources: NativeResource[]; metadata: NativeMetadata | null; expires_at: number }
export interface PreviewNativeVisibility extends NativeSelection { instance: NativeInstance; visible: boolean }
export interface NativeVisibilityPreview { preview: NativePreview; affected: { selection: NativeSelection; instance: NativeInstance; slug: string }[]; coverage: string }
export type NativeTransport = 'Stdio' | 'Http' | 'Sse' | 'WebSocket' | 'Unknown';
export type NativeField<T> = { kind: 'keep' } | { kind: 'set'; value: T } | { kind: 'remove' };
export interface NativeServerPatch { transport?: NativeField<NativeTransport>; program?: NativeField<string>; args?: NativeField<string[]>; url?: NativeField<string>; cwd?: NativeField<string>; bearer_env?: NativeField<string>; forwarded_env?: NativeField<string[]>; header_env?: NativeField<[string, string][]> }
export type NativeMcpChange = { kind: 'upsert'; name: string; fields: NativeServerPatch } | { kind: 'remove'; name: string } | { kind: 'set_enabled'; name: string; enabled: boolean };
export interface PreviewNativeMcp extends NativeSelection { change: NativeMcpChange }
export interface NativeAuthentication { configured: boolean; credential_present: boolean | null; authenticated: boolean | null; connected: boolean | null; approved: boolean | null; active_session: boolean | null; login: 'Unsupported'; logout: 'Unsupported'; delete_credentials: 'Unsupported' }
export interface NativeMcpInstance { native_key: string; transport: NativeTransport; enabled: boolean | null; has_auth_configuration: boolean; unknown_fields_present: boolean; owner: NativeOwner; authentication: NativeAuthentication }
export interface NativeMcp { instances: NativeMcpInstance[]; incomplete: boolean }
export interface NativeOperation { operation_id: string }
export type NativeResult<T> = { ok: true; value: T } | { ok: false; code: string };

export type NativePatchMode = 'keep' | 'set' | 'remove';
export type NativeDraftField = 'slug' | 'source' | 'name' | 'program' | 'args' | 'url' | 'cwd' | 'bearer_env' | 'forwarded_env' | 'header_env';
export interface NativeDraft extends Record<NativeDraftField, string> {
  transport: 'Stdio' | 'Http' | 'Sse';
  modes: Partial<Record<keyof NativeServerPatch, NativePatchMode>>;
}
export interface NativePending {
  kind: 'skills' | 'mcp';
  selection: NativeSelection;
  mode: 'preview' | 'inspect' | 'result';
  receipt: NativeReceipt;
  preview: NativePreview | NativeSkillPreview | NativeVisibilityPreview;
  draft_key: string;
  draft: NativeDraft | undefined;
  summary: string;
}
