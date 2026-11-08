export default {
 'cli.busy-load': '正在读取配置…', 'cli.busy-save': '正在保存并确认…', 'cli.busy-probe': '正在检测…', 'cli.action-error': '操作未完成', 'cli.save-confirmed': '已保存并确认',
 'cli.detect-all': '自动检测', 'cli.detect-tool': '检测 {tool}', 'cli.detected': '已检测到', 'cli.path-note': '点击“自动检测”查找 CLI 路径、读取版本并检查更新，结果自动保存在本设备当前账户。路径仅可查看；Node.js 可通过文件夹按钮选择并保存。',
 'cli.probing-path': '正在查找路径…', 'cli.checking-update': '正在检查更新…',
 'cli.description.codex': 'OpenAI 的 Codex 编程代理', 'cli.description.claude': 'Anthropic 的 Claude Code CLI', 'cli.description.gemini': 'Google 的 Gemini CLI 工具', 'cli.description.node': 'JavaScript 运行时环境',
 'cli.busy-folder': '正在检查 Node.js 文件夹…', 'cli.busy-update': '正在等待确认或更新…',
 'cli.node-folder': '选择 Node.js 安装文件夹', 'cli.node-folder-error': '未能在此文件夹验证可运行的 Node.js。之前的路径和版本保持不变。',
 'cli.update': '更新', 'cli.update-tool': '更新 {tool}', 'cli.update-title': '更新 {tool}？',
 'cli.update-confirm': '使用 {method} 将 {tool} 从 {current} 更新到 {latest}？此操作会修改当前操作系统用户下各应用和 DG-OS 账户共用的安装，并非仅影响当前 DG-OS 账户。',
 'cli.check-unsupported': '此安装不支持在此更新，请使用原安装管理器。', 'cli.check-check_failed': '检查更新失败。已检测到的路径和版本保持不变。',
 'cli.update-updated': '已更新，并验证了新版本。', 'cli.update-failed': '更新失败，未验证到新版本。再次更新前请重新自动检测。',
 'cli.update-unknown': '更新结果未知，不会重试。请重新自动检测以确认安装版本。', 'cli.update-stale': '安装或更新检查已变化，请重新自动检测后再更新。', 'cli.update-busy': '应用仍有活动任务或安装正在更新。请等待活动结束后重新自动检测。',
 'cli.title': 'CLI 工具', 'cli.help': '显示上次保存的检测结果；点击“自动检测”刷新。', 'cli.tool.codex': 'Codex', 'cli.tool.claude': 'Claude', 'cli.tool.gemini': 'Gemini', 'cli.tool.node': 'Node.js', 'cli.can-run': '支持运行与版本检测', 'cli.detect-only': '仅支持版本检测，不提供运行或安装管理', 'cli.path-label': 'CLI 路径', 'cli.path': '尚未检测到路径', 'cli.detect': '检测版本', 'cli.saved-path': '已保存路径：', 'cli.automatic': '自动检测', 'cli.technical-details': '技术详情', 'cli.result-path': '检测路径', 'cli.version': '版本', 'cli.reload': '重新读取配置', 'cli.saved-state': '配置已读取', 'cli.unsaved': '有未保存的草稿', 'cli.load-error': '读取配置失败。草稿已保留，请重试读取。', 'cli.save-error': '保存未确认或发生冲突。草稿已保留；重新读取配置后才能再次保存。', 'cli.probe-error': '版本检测失败。保存路径没有改变。', 'cli.available': '可执行，版本查询成功。运行时仍须通过协议握手。', 'cli.not_installed': '未安装或找不到路径。', 'cli.not_executable': '文件不可执行。', 'cli.protocol_unsupported': '版本响应不支持。', 'cli.start_failed': '进程启动失败。', 'cli.execution_failed': '版本查询执行失败。', 'cli.output_limit': '版本输出超出限制。', 'cli.timeout': '版本查询超时。'
} as const;
