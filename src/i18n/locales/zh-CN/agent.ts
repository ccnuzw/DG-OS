export default {
  'agent.engine.codex': 'Codex', 'agent.engine.claude': 'Claude',
  'agent.you': '你', 'agent.assistant': '智能体', 'agent.finished': '本轮结果', 'agent.activity': '工具与进度',
  'agent.truncated': '这里只显示最近的会话内容；完整历史由本机智能体保存。',
  'agent.title': '本地智能体会话', 'agent.subtitle': 'Codex 与 Claude 独立运行，保留各自的上下文与工具体系。',
  'agent.engine': '智能体', 'agent.workspace': '工作目录', 'agent.settings': '运行设置', 'agent.executable': '可执行文件绝对路径（留空自动查找）', 'agent.model': '模型（留空使用原生配置）',
  'agent.codexPolicy': 'Codex：工作区可写，按原生策略请求审批。', 'agent.claudePolicy': 'Claude：加载原生项目配置，按工具权限策略请求审批。',
  'agent.trust': '我信任所选目录及要恢复会话的目录，允许加载其中的指令、Hooks、Skills 和 MCP 配置。',
  'agent.new': '新建会话', 'agent.sessions': '已保存会话', 'agent.messages': '会话记录', 'agent.details': '查看详情', 'agent.empty': '发送消息开始对话。', 'agent.choose': '选择工作目录创建会话，或恢复已有会话。',
  'agent.prompt': '消息', 'agent.send': '发送', 'agent.interrupt': '中断当前任务', 'agent.close': '关闭运行', 'agent.approval': '待处理请求', 'agent.allow': '允许本次', 'agent.deny': '拒绝', 'agent.answer': '提交回答',
  'agent.failure': '操作未完成。请检查本机智能体的安装、登录与配置；执行结果不确定时请先核实，不要重复提交。',
  'agent.state.starting': '正在连接', 'agent.state.idle': '等待消息', 'agent.state.running': '执行中', 'agent.state.waiting': '等待你的回应', 'agent.state.closed': '已关闭，可恢复', 'agent.state.error': '运行失败', 'agent.state.unknown': '连接中断，结果待核实',
} as const;
