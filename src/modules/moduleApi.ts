// Typed wrappers over the module management commands (D055). Every command answers
// `{ success, data, error }`; the wrappers return `data` or throw a ModuleApiError carrying the
// host's code and message, so the UI never has to show a raw transport error.
import { invoke } from '@tauri-apps/api/core';

export type RuntimeState = 'not_loaded' | 'starting' | 'ready' | 'draining' | 'stopped' | 'failed';

export interface RuntimeStatus {
  module_id: string;
  state: RuntimeState;
  runtime_id: string | null;
  version: string | null;
  integrity: string | null;
  in_flight: number;
  pending_releases: number;
  active_operation_id: string | null;
  executes_module_code: boolean;
}

export interface ModulePermission {
  permission: string;
  granted: boolean;
}

export interface ModuleSummary {
  module_id: string;
  name: string;
  version: string;
  integrity: string;
  official: boolean;
  enabled: boolean;
  permissions: ModulePermission[];
  runtime: RuntimeStatus;
  recent_crashes: number;
}

export type ModuleOperationKind = 'start' | 'stop' | 'update' | 'enable' | 'disable' | 'uninstall';
export type ModuleOperationState = 'queued' | 'running' | 'succeeded' | 'failed';
export type ModuleOperationOutcome =
  | 'started'
  | 'stopped'
  | 'updated'
  | 'updated_pending_release'
  | 'unchanged'
  | 'enabled'
  | 'disabled'
  | 'uninstalled';

export interface ModuleOperationFailure {
  kind: string;
  code: number;
  retryable: boolean;
}

export interface ModuleOperationView {
  operation_id: string;
  module_id: string;
  kind: ModuleOperationKind;
  state: ModuleOperationState;
  outcome: ModuleOperationOutcome | null;
  failure: ModuleOperationFailure | null;
  submitted_at: number;
  finished_at: number | null;
}

export interface ModuleCrashRecord {
  crash_id: string;
  module_id: string;
  module_version: string;
  source: string;
  error_name: string;
  stack: string;
  created_at: number;
}

export type ModuleLifecycleCommand =
  | 'start_module'
  | 'stop_module'
  | 'update_module'
  | 'enable_module'
  | 'disable_module'
  | 'uninstall_module';

export const CODE_MODULE_INCOMPATIBLE = 1004022;
export const CODE_UPDATE_FAILED = 1004013;

export const OPERATION_POLL_INTERVAL_MS = 300;
export const OPERATION_TIMEOUT_MS = 30_000;

export class ModuleApiError extends Error {
  readonly code: number;
  readonly kind: string | null;

  constructor(code: number, message: string, kind: string | null = null) {
    super(message);
    this.name = 'ModuleApiError';
    this.code = code;
    this.kind = kind;
  }

  get incompatible(): boolean {
    return this.code === CODE_MODULE_INCOMPATIBLE || this.kind === 'module_incompatible';
  }

  get updateFailed(): boolean {
    return this.code === CODE_UPDATE_FAILED || this.kind === 'update_failed';
  }
}

interface CommandResult<T> {
  success: boolean;
  data: T | null;
  error: { code: number; message: string } | null;
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  let result: CommandResult<T> | null;
  try {
    result = await invoke<CommandResult<T> | null>(command, args);
  } catch {
    // Transport-level failure (no host answer). The raw error stays out of the UI.
    throw new ModuleApiError(0, 'The host did not answer the request');
  }
  if (result === null || typeof result !== 'object' || !result.success || result.data === null) {
    const error = result?.error;
    throw new ModuleApiError(error?.code ?? 0, error?.message ?? 'The host rejected the request');
  }
  return result.data;
}

export const listModules = (): Promise<ModuleSummary[]> => call('list_modules');

export const getModuleRuntimeStatus = (moduleId: string): Promise<RuntimeStatus> =>
  call('get_module_runtime_status', { moduleId });

export const grantModulePermission = (moduleId: string, permission: string): Promise<ModuleSummary> =>
  call('grant_module_permission', { moduleId, permission });

export const revokeModulePermission = (moduleId: string, permission: string): Promise<ModuleSummary> =>
  call('revoke_module_permission', { moduleId, permission });

export const listModuleCrashes = (moduleId: string): Promise<ModuleCrashRecord[]> =>
  call('list_module_crashes', { moduleId });

// Submits a lifecycle command, then polls the operation until it reaches a terminal state.
// A `failed` operation is returned, not thrown: use `operationError` or `runOperations` for that.
export async function runOperation(command: ModuleLifecycleCommand, moduleId: string): Promise<ModuleOperationView> {
  const { operation_id: operationId } = await call<{ operation_id: string }>(command, { moduleId });
  const deadline = Date.now() + OPERATION_TIMEOUT_MS;
  for (;;) {
    const view = await call<ModuleOperationView>('get_module_operation', { operationId });
    if (view.state === 'succeeded' || view.state === 'failed') return view;
    if (Date.now() >= deadline) {
      throw new ModuleApiError(0, 'The operation did not finish in time; check the module state and try again');
    }
    await new Promise<void>((resolve) => setTimeout(resolve, OPERATION_POLL_INTERVAL_MS));
  }
}

export function operationError(view: ModuleOperationView): ModuleApiError | null {
  if (view.state !== 'failed') return null;
  const failure = view.failure;
  if (failure === null) return new ModuleApiError(0, `The ${view.kind} operation failed`);
  return new ModuleApiError(failure.code, failureMessage(view.kind, failure), failure.kind);
}

function failureMessage(kind: ModuleOperationKind, failure: ModuleOperationFailure): string {
  if (failure.kind === 'update_failed' || failure.code === CODE_UPDATE_FAILED) {
    return 'Update failed, the previous version was restored';
  }
  if (failure.kind === 'module_incompatible' || failure.code === CODE_MODULE_INCOMPATIBLE) {
    return 'The host reported this module as incompatible';
  }
  return `The ${kind} operation failed (${failure.kind.replace(/_/g, ' ')})`;
}

// Runs lifecycle commands one after another and stops at the first failure, which is thrown.
// Returns the last operation view.
export async function runOperations(
  steps: ReadonlyArray<ModuleLifecycleCommand>,
  moduleId: string,
): Promise<ModuleOperationView> {
  let view: ModuleOperationView | null = null;
  for (const step of steps) {
    view = await runOperation(step, moduleId);
    const error = operationError(view);
    if (error !== null) throw error;
  }
  if (view === null) throw new ModuleApiError(0, 'No operation was run');
  return view;
}
