// Error code -> user-facing text (FR-005 §8.2, B05-27). Keyed by the numeric `error.code` the host returns.
//
// Only codes that the registry (docs/04-技术架构/当前版本/V1-错误码.md) marks "使用中" and that the host returns as
// a number over IPC are listed. Reserved and voided codes are deliberately absent, so they take the generic
// fallback instead of a message nobody can trigger yet. Skills codes (1002xxx) are absent too: Skills commands
// return `Result<_, String>`, not a numeric code (registry §3.2), and so do MCP commands. Module codes (1004xxx)
// reach the frontend but their text and severity are undecided (FR-005 §13.2). Add a code here only together with
// its registry status. The Provider codes 1001014, 1001017, 1001018, 1001021, 1001022 and 1001024 were added by D098;
// their advice is the registry's 处理建议 column.
//
// Severity is a presentation decision made here, not by the host: `minor` is a 3 s toast, `moderate` a 5 s toast
// with advice, `severe` a Modal that needs acknowledging (§8.3).

export type Severity = 'minor' | 'moderate' | 'severe';

export interface ErrorMessage {
  message: string;
  description?: string;
  severity: Severity;
  /** Shown only if the caller can navigate and the route exists in the application's route table. */
  action?: { label: string; route: string };
}

export const ERROR_MESSAGES: Readonly<Record<number, ErrorMessage>> = Object.freeze({
  1000000: {
    message: '操作失败',
    description: '请稍后重试，持续出现时查看日志',
    severity: 'moderate',
  },
  1001001: {
    message: 'Provider 不存在',
    description: '请检查 Provider ID 是否正确',
    severity: 'moderate',
    action: { label: '查看 Provider 列表', route: '/providers' },
  },
  1001002: {
    message: 'API Key 无效',
    description: 'Provider 返回认证错误，请检查 API Key 是否正确',
    severity: 'moderate',
  },
  1001003: {
    message: 'Provider 不可用',
    description: '请启用 Provider 或检查网络与配置',
    severity: 'moderate',
    action: { label: '查看 Provider 列表', route: '/providers' },
  },
  1001004: {
    message: '模型不存在',
    description: '请刷新模型列表或检查模型 ID',
    severity: 'moderate',
  },
  1001005: {
    message: '请求过于频繁',
    description: '上游触发速率限制，请稍后重试或切换 Provider',
    severity: 'moderate',
  },
  1001006: {
    message: '请求超时',
    description: '请检查网络后重试',
    severity: 'moderate',
  },
  1001007: {
    message: '流式输出中断',
    description: '请重新生成',
    severity: 'minor',
  },
  1001008: {
    message: '重试次数已用完',
    description: '请检查 Provider 状态或配置',
    severity: 'moderate',
  },
  1001012: {
    message: 'Provider 配置已变化',
    description: '请刷新模型后重试',
    severity: 'minor',
  },
  1001014: {
    message: '上游服务器错误',
    description: '请稍后重试，持续出现时请联系 Provider',
    severity: 'moderate',
  },
  1001015: {
    message: '系统凭据库不可用',
    description: '请解锁系统凭据库后重试',
    severity: 'severe',
  },
  1001016: {
    message: '缺少 API Key',
    description: '更换地址或类型后需要重新填写 API Key',
    severity: 'moderate',
  },
  1001017: {
    message: 'Provider 拒绝了请求',
    description: '请检查请求参数或模型设置',
    severity: 'moderate',
  },
  1001018: {
    message: '输入超出上下文长度',
    description: '请缩短输入，或换用上下文更长的模型',
    severity: 'moderate',
  },
  1001019: {
    message: 'Provider 权限不足',
    description: 'Key 有效但无权限，或所在地区受限，请检查 Key 权限与地区',
    severity: 'moderate',
  },
  1001020: {
    message: '请求不受支持',
    description: '该请求或配置不被此服务支持，请调整后重试',
    severity: 'moderate',
  },
  1001021: {
    message: '结构化输出无效',
    description: '请检查 schema，或换用支持结构化输出的模型',
    severity: 'moderate',
  },
  1001022: {
    message: '不支持的 Provider 类型',
    description: '请重新选择厂商和适配器',
    severity: 'moderate',
  },
  1001023: {
    message: '不允许使用明文 HTTP',
    description: '请改用 https，或在高级选项中确认明文传输',
    severity: 'moderate',
  },
  1001024: {
    message: 'Provider 额度已用尽',
    description: '请充值或提高额度',
    severity: 'moderate',
  },
  1001025: {
    message: 'Provider 响应无效',
    description: '请检查 Base URL 是否缺少 /v1，或填成了网页地址',
    severity: 'moderate',
  },
  1008001: {
    message: '用户名已存在',
    description: '请换一个用户名',
    severity: 'moderate',
  },
  1008002: {
    message: '密码不符合规则',
    description: '密码需为 8～128 个字符',
    severity: 'moderate',
  },
  1008003: {
    message: '用户名格式错误',
    description: '用户名需为 1～32 个字符，首尾不能有空白，不能含控制字符',
    severity: 'moderate',
  },
  1008005: {
    message: '用户名或密码错误',
    description: '请检查后重试',
    severity: 'moderate',
  },
  1008007: {
    message: '登录状态已失效',
    description: '请重新登录',
    severity: 'moderate',
  },
  1008009: {
    message: '账户正在删除',
    description: '删除将在应用重新启动后继续完成，完成前该账户无法登录',
    severity: 'moderate',
  },
  1008011: {
    message: '尝试次数过多',
    description: '请稍等片刻后再试',
    severity: 'moderate',
  },
});

/** Own entries only: a code such as `0` must not reach `Object.prototype`. */
export function lookupError(code: number | null): ErrorMessage | undefined {
  return code !== null && Object.prototype.hasOwnProperty.call(ERROR_MESSAGES, code) ? ERROR_MESSAGES[code] : undefined;
}
