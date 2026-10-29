// Error code -> user-facing text (FR-005 §8.2, B05-27). Keyed by the numeric `error.code` the host returns.
//
// Only codes that the registry (docs/04-技术架构/当前版本/V1-错误码.md) marks "使用中" and that the host returns as
// a number over IPC are listed. Reserved and voided codes are deliberately absent, so they take the generic
// fallback instead of a message nobody can trigger yet. Skills codes (1002xxx) are absent too: Skills commands
// return `Result<_, String>`, not a numeric code (registry §3.2). MCP and module codes carry no status in the
// registry. Add a code here only together with its registry status.
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
});

/** Own entries only: a code such as `0` must not reach `Object.prototype`. */
export function lookupError(code: number | null): ErrorMessage | undefined {
  return code !== null && Object.prototype.hasOwnProperty.call(ERROR_MESSAGES, code) ? ERROR_MESSAGES[code] : undefined;
}
