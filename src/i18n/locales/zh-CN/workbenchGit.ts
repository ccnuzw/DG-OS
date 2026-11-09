export default {
  'workbenchGit.title': 'Git',
  'workbenchGit.workspaceScope': '显示当前工作区的整个仓库状态，可能包含本任务之外的变更。Git 差异比较索引、工作区与 Git 版本，不是任务基线差异。',
  'workbenchGit.taskBaseline': '任务基线仅用于受保护恢复，不能证明变更归属；原生 CLI 与外部编辑可能混合存在。',
  'workbenchGit.noTaskBaseline': '没有可用的任务基线，变更归属不确定，无法执行受保护恢复。',
  'workbenchGit.existingStaged': '任务基线记录的原有暂存路径',
  'workbenchGit.viewChanges': '查看变更',
  'workbenchGit.noChanges': '此工作区没有变更路径。',
  'workbenchGit.projectScope': '只读查看项目工作区；浏览 Git 历史和差异不会创建任务。',
  'workbenchGit.enableWrites': '准备 Git 写入',
  'workbenchGit.staged': '已暂存的更改',
  'workbenchGit.unstaged': '未暂存的更改',
  'workbenchGit.repositoryActions': '仓库操作',
} as const;
