// Error code -> user-facing text (FR-005 §8.2, B05-27). Keyed by the numeric `error.code` the host returns.
//
// Only codes that the registry (docs/04-技术架构/当前版本/V1-错误码.md) marks "使用中" and that the host returns as
// a number over IPC are listed. Reserved and voided codes are deliberately absent, so they take the generic
// fallback instead of a message nobody can trigger yet. Skills codes (1002xxx) are absent too: Skills commands
// return `Result<_, String>`, not a numeric code (registry §3.2), and so do MCP commands. Module codes (1004xxx)
// reach the frontend as numbers: all 21 "使用中" codes are listed per D110; the six reserved ones (1004005, 1004008,
// 1004010, 1004011, 1004012, 1004014) take the fallback. Add a code here only together with its registry status.
// The Provider codes 1001014, 1001017, 1001018, 1001021, 1001022 and 1001024 were added by D098; their advice is
// the registry's 处理建议 column.
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

import { getLocale } from '../../i18n/runtime';

type BilingualMessage = { zh: ErrorMessage; en: ErrorMessage };

const BILINGUAL_ERRORS: Readonly<Record<number, BilingualMessage>> = Object.freeze({
  1000000: { zh: { message: '操作失败', description: '请稍后重试，持续出现时查看日志', severity: 'moderate' }, en: { message: 'Operation failed', description: 'Please try again later. Check the logs if the problem persists', severity: 'moderate' } },
  1001001: { zh: { message: 'Provider 不存在', description: '请检查 Provider ID 是否正确', severity: 'moderate', action: { label: '查看 Provider 列表', route: '/providers' } }, en: { message: 'Provider not found', description: 'Check that the Provider ID is correct', severity: 'moderate', action: { label: 'View Providers', route: '/providers' } } },
  1001002: { zh: { message: 'API Key 无效', description: 'Provider 返回认证错误，请检查 API Key 是否正确', severity: 'moderate' }, en: { message: 'Invalid API key', description: 'The Provider rejected authentication. Check the API key', severity: 'moderate' } },
  1001003: { zh: { message: 'Provider 不可用', description: '请启用 Provider 或检查网络与配置', severity: 'moderate', action: { label: '查看 Provider 列表', route: '/providers' } }, en: { message: 'Provider unavailable', description: 'Enable the Provider or check the network and configuration', severity: 'moderate', action: { label: 'View Providers', route: '/providers' } } },
  1001006: { zh: { message: '请求超时', description: '请检查网络后重试', severity: 'moderate' }, en: { message: 'Request timed out', description: 'Check the network and try again', severity: 'moderate' } },
  1001007: { zh: { message: '流式输出中断', description: '请重新生成', severity: 'minor' }, en: { message: 'Streaming output interrupted', description: 'Generate the response again', severity: 'minor' } },
  1001015: { zh: { message: '系统凭据库不可用', description: '请解锁系统凭据库；如果是开发版升级后首次使用，请重新保存 API Key 或令牌后重试', severity: 'severe' }, en: { message: 'OS credential store unavailable', description: 'Unlock the credential store. After a development upgrade, save the API key or token again', severity: 'severe' } },
  1001016: { zh: { message: '缺少 API Key', description: '更换地址或类型后需要重新填写 API Key', severity: 'moderate' }, en: { message: 'API key missing', description: 'Changing the endpoint or type requires a new API key', severity: 'moderate' } },
  1004001: { zh: { message: '模块不存在', description: '请检查模块 ID 或刷新模块列表', severity: 'moderate', action: { label: '查看模块列表', route: '/settings/modules' } }, en: { message: 'Module not found', description: 'Check the module ID or refresh the module list', severity: 'moderate', action: { label: 'View modules', route: '/settings/modules' } } },
  1004003: { zh: { message: '模块加载失败', description: '模块清单或入口无效，可在模块管理查看崩溃记录', severity: 'moderate' }, en: { message: 'Module failed to load', description: 'The manifest or entry point is invalid. Check the module crash log', severity: 'moderate' } },
  1004004: { zh: { message: '没有权限执行此操作', description: '请在模块管理确认权限后重试', severity: 'moderate' }, en: { message: 'Permission denied', description: 'Confirm the permission in Module Management and try again', severity: 'moderate' } },
  1008001: { zh: { message: '用户名已存在', description: '请换一个用户名', severity: 'moderate' }, en: { message: 'Username already exists', description: 'Choose a different username', severity: 'moderate' } },
  1008002: { zh: { message: '密码不符合规则', description: '密码需为 8～128 个字符', severity: 'moderate' }, en: { message: 'Password does not meet the requirements', description: 'The password must be 8–128 characters', severity: 'moderate' } },
  1008005: { zh: { message: '用户名或密码错误', description: '请检查后重试', severity: 'moderate' }, en: { message: 'Wrong username or password', description: 'Check your credentials and try again', severity: 'moderate' } },
  1008007: { zh: { message: '登录状态已失效', description: '请重新登录', severity: 'moderate' }, en: { message: 'Sign-in session expired', description: 'Sign in again', severity: 'moderate' } },
  1008012: { zh: { message: '无法打开本机数据库', description: '查看数据库文件位置，备份或移走该文件后重启应用', severity: 'severe' }, en: { message: 'Unable to open the local database', description: 'Check the database file location, back it up or move it, then restart the app', severity: 'severe' } },
});

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
    description: '请解锁系统凭据库；如果是开发版升级后首次使用，请重新保存 API Key 或令牌后重试',
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
  1004001: {
    message: '模块不存在',
    description: '请检查模块 ID 或刷新模块列表',
    severity: 'moderate',
    action: { label: '查看模块列表', route: '/settings/modules' },
  },
  1004002: {
    message: '模块已在运行',
    description: '无需重复启动',
    severity: 'minor',
  },
  1004003: {
    message: '模块加载失败',
    description: '模块清单或入口无效，可在模块管理查看崩溃记录',
    severity: 'moderate',
  },
  1004004: {
    message: '没有权限执行此操作',
    description: '请在模块管理确认权限后重试',
    severity: 'moderate',
  },
  1004006: {
    message: '模块版本冲突',
    description: '请更新到兼容版本',
    severity: 'moderate',
  },
  1004007: {
    message: '模块停止失败',
    description: '请稍后重试',
    severity: 'moderate',
  },
  1004009: {
    message: '模块已禁用',
    description: '请先在模块管理中启用',
    severity: 'moderate',
    action: { label: '查看模块列表', route: '/settings/modules' },
  },
  1004013: {
    message: '模块更新失败',
    description: '已保留当前版本，请稍后重试',
    severity: 'moderate',
  },
  1004015: {
    message: '模块包无效',
    description: '模块文件与清单不一致，请重新安装应用',
    severity: 'moderate',
  },
  1004016: {
    message: '模块未运行',
    description: '请先启动模块',
    severity: 'minor',
  },
  1004017: {
    message: '模块正在执行其他操作',
    description: '请等当前操作完成后重试',
    severity: 'minor',
  },
  1004018: {
    message: '操作记录不存在',
    description: '操作已过期或应用已重启，请刷新状态',
    severity: 'minor',
  },
  1004019: {
    message: '模块服务不可用',
    description: '启动恢复未成功，请重启应用或查看恢复日志',
    severity: 'severe',
  },
  1004020: {
    message: '模块服务内部错误',
    description: '请稍后重试，持续出现时查看日志',
    severity: 'moderate',
  },
  1004021: {
    message: '模块未通过准入',
    description: '该模块不在受信任清单中',
    severity: 'moderate',
  },
  1004022: {
    message: '模块与当前应用不兼容',
    description: '请安装兼容版本或升级应用',
    severity: 'moderate',
  },
  1004023: {
    message: '内置模块不能卸载',
    description: '可以改为禁用',
    severity: 'minor',
  },
  1004024: {
    message: '参数无效',
    description: '请检查输入后重试',
    severity: 'moderate',
  },
  1004025: {
    message: '模块存储空间已满',
    description: '存储超过 5 MiB，请删除不需要的数据',
    severity: 'moderate',
  },
  1004026: {
    message: '拒绝访问非本机地址',
    description: '模块只能访问本机服务',
    severity: 'moderate',
  },
  1004027: {
    message: '响应过大',
    description: '本机服务的响应超过 1 MiB',
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
  1008012: {
    message: '无法打开本机数据库',
    description: '查看数据库文件位置，备份或移走该文件后重启应用',
    severity: 'severe',
  },
});

/** Own entries only: a code such as `0` must not reach `Object.prototype`. */
export function lookupError(code: number | null): ErrorMessage | undefined {
  if (code === null) return undefined;
  const bilingual = BILINGUAL_ERRORS[code];
  if (getLocale() === 'zh-CN') {
    return Object.prototype.hasOwnProperty.call(ERROR_MESSAGES, code) ? ERROR_MESSAGES[code] : undefined;
  }
  if (bilingual) {
    return bilingual.en;
  }
  const base = ERROR_MESSAGES[code];
  return base
    ? { message: 'Operation failed', description: 'Please try again later. Check the logs if the problem persists', severity: base.severity }
    : undefined;
}
