export default {
 'cli.title': 'CLI 工具', 'cli.help': '路径留空时自动检测。配置仅属于本设备当前账户。Codex 和 Claude 支持运行；Gemini、Node 仅检测，不提供安装管理。', 'cli.path': '自动检测，或输入绝对可执行文件路径', 'cli.detect': '检测版本', 'cli.reload': '重新读取配置', 'cli.error': '配置保存失败、冲突或结果未确认。请重新读取后再修改。', 'cli.available': '可执行，版本查询成功。运行时仍须通过协议握手。', 'cli.not_installed': '未安装或找不到路径。', 'cli.not_executable': '文件不可执行。', 'cli.protocol_unsupported': '版本响应不支持。', 'cli.start_failed': '进程启动失败。', 'cli.execution_failed': '版本查询执行失败。', 'cli.output_limit': '版本输出超出限制。', 'cli.timeout': '版本查询超时。'
} as const;
