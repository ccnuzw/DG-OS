export default {
  'commitDraft.title': '生成提交信息',
  'commitDraft.generate': '发送差异并生成',
  'commitDraft.disclosure': '将下面的已暂存差异发送给当前项目配置的模型。生成结果只填入提交草稿，不会提交代码或发送到当前会话。',
  'commitDraft.diffLimit': '请先暂存改动；生成内容上限为 64 KiB，请缩小本次提交。',
  'commitDraft.providerRequired': '请先为当前项目配置 Provider 和模型。',
  'commitDraft.failed': '提交信息生成失败（{code}）。未自动重试，请检查模型配置后再操作。',
} as const;
