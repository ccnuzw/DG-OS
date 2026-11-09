// Stable failure codes the host returns as plain strings (D127 agents, D134 project/task domain). The UI maps each
// code to `dev.error.<code>` text. Any other string, or a non-string error, is normalized to `unknown`, so raw host
// messages (which may contain paths or tokens) never reach the screen.
export const HOST_CODES: readonly string[] = [
  'session_invalid', 'session_not_found', 'session_not_saved', 'session_index_invalid', 'session_index_unavailable',
  'workspace_untrusted', 'invalid_workspace', 'workspace_changed', 'workspace_busy', 'invalid_request_id', 'cancelled',
  'task_not_found', 'project_not_found', 'invalid_project', 'invalid_task', 'invalid_selection', 'project_archived', 'project_busy',
  'git_unavailable', 'git_failed', 'git_timeout', 'git_output_limit', 'storage_failed',
  'file_sensitive', 'file_access_denied', 'file_not_found', 'file_too_large', 'file_binary', 'file_changed', 'scan_limit',
  'invalid_reference', 'reference_limit',
  'invalid_command', 'secret_argument_denied', 'request_conflict', 'invalid_readiness', 'process_capacity', 'maintenance_active',
  'process_not_found', 'terminal_input_limit', 'terminal_closed', 'terminal_starting', 'invalid_terminal_size', 'terminal_unsupported',
  'process_wait_failed', 'environment_reference_missing', 'terminal_unavailable', 'process_spawn_failed', 'process_platform_unsupported',
  'service_port_in_use', 'service_readiness_timeout', 'service_exited_before_ready', 'cleanup_pending', 'external_open_failed', 'service_not_ready',
  'boot_identity_unavailable',
  'file_symlink', 'file_save_unknown', 'file_exists', 'file_create_unknown',
  'program_unavailable', 'port_conflict',
  'capability_unavailable', 'capability_conflict', 'capability_revoked', 'provider_unavailable', 'provider_incompatible',
  'skill_unavailable', 'mcp_unavailable', 'schema_changed', 'adapter_unavailable', 'cli_version_changed', 'confirmation_expired', 'confirmation_not_found',
  'cli_incompatible', 'provider_required', 'provider_revision_changed', 'provider_binding_unconfirmed', 'credential_unavailable', 'legacy_session_read_only', 'session_closed', 'session_busy',
  'unsupported_reasoning_effort', 'unsupported_permission_mode', 'turn_controls_unavailable', 'invalid_turn_settings',
  'bridge_unavailable', 'session_exists', 'request_capacity',
  'native_extension_AccessDenied', 'native_extension_UnsafePath', 'native_extension_RootChanged', 'native_extension_InvalidFormat', 'native_extension_Limit', 'native_extension_Unsupported',
  'native_extension_Conflict', 'native_extension_Busy', 'native_extension_Expired', 'native_extension_NotFound', 'native_extension_Unavailable', 'native_extension_RecoveryRequired', 'native_extension_StaleGeneration',
  'request_expired', 'run_not_active', 'stale_generation',
  'skill_load_failed', 'provider_load_failed',
  'invalid_handoff', 'handoff_limit', 'handoff_already_accepted', 'handoff_stale',
  'workspace_immutable', 'workspace_not_found', 'not_git_repository', 'invalid_branch', 'invalid_commit', 'invalid_request',
  'git_review_stale', 'git_review_incomplete', 'git_full_index_confirmation_required', 'git_preexisting_staged_confirmation_required', 'git_push_trust_required',
  'git_pull_diverged', 'git_pull_preview_stale', 'git_pull_trust_required', 'invalid_git_history_limit', 'invalid_git_history_cursor', 'git_history_limit',
  'git_unmerged', 'git_worktree_dirty', 'git_detached_head', 'git_unborn_head', 'git_required_filter', 'git_url_rewrite_unsupported', 'unsafe_git_config', 'unsafe_git_remote',
  'invalid_git_remote', 'invalid_git_ref', 'invalid_git_paths', 'invalid_commit_message', 'git_special_entry', 'git_index_hidden_files', 'git_workspace_not_root', 'git_non_utf8_path', 'git_invalid_output', 'git_unknown',
  'discard_confirmation_required', 'baseline_missing', 'baseline_changed', 'baseline_incomplete', 'discard_preexisting_changes', 'discard_index_changed', 'discard_ownership_uncertain', 'discard_external_change', 'discard_partial_failure',
  'repository_busy', 'operation_not_found', 'invalid_git_operation',
  'foreign_workspace', 'workspace_remove_refused', 'workspace_in_use', 'workspace_dirty', 'invalid_operation',
];

export function hostCode(error: unknown): string {
  return typeof error === 'string' && HOST_CODES.includes(error) ? error : 'unknown';
}
