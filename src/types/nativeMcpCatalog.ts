import type { NativeSelection, NativeTransport } from './nativeExtensions';

export interface RegistryInput { description?: string; default?: string; value?: string; placeholder?: string; choices?: string[] | null; isRequired?: boolean; isSecret?: boolean; format?: string }
export interface RegistryArgument extends RegistryInput { type: string; name?: string; valueHint?: string; isRepeated?: boolean; variables?: Record<string, RegistryInput> }
export interface RegistryKeyValue extends RegistryInput { name: string; variables?: Record<string, RegistryInput> }
export interface RegistryTransport { type: string; url?: string; headers?: RegistryKeyValue[] | null; variables?: Record<string, RegistryInput> }
export interface RegistryPackage { registryType: string; registryBaseUrl?: string; identifier: string; version?: string; runtimeHint?: string; fileSha256?: string; transport: RegistryTransport; runtimeArguments?: RegistryArgument[] | null; packageArguments?: RegistryArgument[] | null; environmentVariables?: RegistryKeyValue[] | null }
export interface RegistryServer { name: string; title?: string; description: string; version: string; repository?: { url?: string; source?: string }; websiteUrl?: string; remotes?: RegistryTransport[] | null; packages?: RegistryPackage[] | null }
export interface RegistryEntry { server: RegistryServer; _meta: Record<string, { status?: string; isLatest?: boolean; publishedAt?: string; updatedAt?: string }> }
export interface RegistryPage { servers: RegistryEntry[]; next_cursor: string | null; fetched_at: number; stale: boolean; error: string | null; source: string }
export interface NativeMcpConfigProjection { name: string; transport: NativeTransport; program: string | null; args: string[]; url: string | null; cwd: string | null; credential_references: string[]; redacted: boolean; config_digest: string }
export interface NativeProbeDraft { name: string; transport: string; program: string; args: string[]; url: string; cwd: string; bearer_env: string; forwarded_env: string[]; header_env: [string, string][] }
export type NativeProbeSource = { kind: 'existing'; server_name: string } | { kind: 'draft'; draft: NativeProbeDraft };
export interface NativeProbeInput { selection: NativeSelection; source: NativeProbeSource }
export interface NativeProbeSnapshot {
  probe_id: string; selection: NativeSelection; config: NativeMcpConfigProjection; digest: string;
  probe_args: string[]; cleanup_plan: { executable: string; name: string; label: string; inspect_argv: string[]; remove_argv_prefix: string[]; remove_target: 'immutable_id_verified_by_exact_name_and_label' } | null;
  executable: string | null; cwd: string; root: string; platform_identity: string; executable_identity: string | null;
  expires_at: number; state: 'ready' | 'running' | 'succeeded' | 'failed' | 'cancelled' | 'unknown';
  phase: string; cleanup: 'not_started' | 'pending' | 'confirmed' | 'unknown'; error: string | null;
  observed_at: number | null; protocol_version: string | null; tools: Record<string, unknown>[];
  schema_source: 'independent_host_probe'; may_download_execute: boolean;
}
export interface RegistryParameter { key: string; label: string; description?: string; required: boolean; reference: boolean; repeated?: boolean; choices?: string[] | null; defaultValue: string }
export interface RegistryDeployment { id: string; label: string; transport: RegistryTransport; package?: RegistryPackage }
