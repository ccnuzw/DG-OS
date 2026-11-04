// Stable failure codes the host returns as plain strings (D127 agents, D134 project/task domain). The UI maps each
// code to `dev.error.<code>` text. Any other string, or a non-string error, is normalized to `unknown`, so raw host
// messages (which may contain paths or tokens) never reach the screen.
export const HOST_CODES: readonly string[] = [
  'session_invalid', 'session_not_found', 'session_not_saved', 'session_index_invalid', 'session_index_unavailable',
  'workspace_untrusted', 'invalid_workspace', 'workspace_changed', 'workspace_busy', 'invalid_request_id', 'cancelled',
  'task_not_found', 'project_not_found', 'invalid_project', 'invalid_task', 'invalid_selection', 'project_archived', 'project_busy',
  'git_unavailable', 'git_failed', 'git_timeout', 'git_output_limit', 'storage_failed',
  'file_sensitive', 'file_access_denied', 'file_not_found', 'file_too_large', 'file_binary', 'file_changed', 'scan_limit',
  'invalid_command', 'secret_argument_denied', 'request_conflict', 'invalid_readiness', 'process_capacity', 'maintenance_active',
  'process_not_found', 'terminal_input_limit', 'terminal_closed', 'terminal_starting', 'invalid_terminal_size', 'terminal_unsupported',
  'process_wait_failed', 'environment_reference_missing', 'terminal_unavailable', 'process_spawn_failed', 'process_platform_unsupported',
  'service_port_in_use', 'service_readiness_timeout', 'service_exited_before_ready', 'cleanup_pending', 'external_open_failed', 'service_not_ready',
  'file_symlink', 'file_save_unknown',
  'program_unavailable', 'port_conflict',
  'capability_unavailable', 'capability_conflict', 'capability_revoked', 'provider_unavailable', 'provider_incompatible',
  'skill_unavailable', 'mcp_unavailable', 'schema_changed', 'adapter_unavailable', 'cli_version_changed', 'confirmation_expired', 'confirmation_not_found',
  'request_expired', 'run_not_active',
  'skill_load_failed', 'provider_load_failed',
];

export function hostCode(error: unknown): string {
  return typeof error === 'string' && HOST_CODES.includes(error) ? error : 'unknown';
}
