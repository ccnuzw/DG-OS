export default {
 'generation.reasoningBudget': '思考 token 预算（仅兼容画像支持）', 'generation.effortMedium': '中',
 'generation.reasoningCapability': '模型推理能力：{state}',
 'generation.supported': '支持', 'generation.unsupported': '不支持', 'generation.unknown': '未知',
 'generation.length': '回复长度', 'generation.short': '短 · 1,024 tokens', 'generation.medium': '中 · 4,096 tokens', 'generation.long': '长 · 8,192 tokens',
 'generation.temperature': '温度（留空使用服务默认值）', 'generation.reasoning': '思考强度', 'generation.default': '服务默认值', 'generation.low': '低', 'generation.high': '高',
 'generation.instruction': '自定义指导', 'generation.capabilityNotice': '服务画像不支持的参数会明确拒绝；指导内容不能授予权限。', 'generation.outputLimit': '已知输出上限：{count} tokens。', 'generation.unknownLimit': '输出上限未知。',
 'generation.moveUp': '上移', 'generation.moveDown': '下移', 'generation.combinationNotice': 'Skills 按此顺序发送并锁定版本。自然语言冲突不自动裁决，后项不保证覆盖前项。'
} as const;
