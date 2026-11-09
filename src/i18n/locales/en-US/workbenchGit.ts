export default {
  'workbenchGit.title': 'Git',
  'workbenchGit.workspaceScope': 'Repository-wide status for the displayed workspace, including changes outside this task. Git diffs compare the index and worktree with Git, not with the task baseline.',
  'workbenchGit.taskBaseline': 'A task baseline is available for guarded restore only. It does not establish who made these changes; native CLI and external edits may be mixed.',
  'workbenchGit.noTaskBaseline': 'No usable task baseline is available. Change ownership is uncertain and guarded restore is unavailable.',
  'workbenchGit.existingStaged': 'Staged paths recorded in the task baseline',
  'workbenchGit.viewChanges': 'View changes',
  'workbenchGit.projectScope': 'Read-only status for the project workspace. Git history and diffs do not create a task.',
  'workbenchGit.noChanges': 'No changed paths in this workspace.',
  'workbenchGit.enableWrites': 'Prepare Git writes',
  'workbenchGit.staged': 'Staged changes',
  'workbenchGit.unstaged': 'Unstaged changes',
  'workbenchGit.repositoryActions': 'Repository actions',
} as const;
