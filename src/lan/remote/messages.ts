import { LanApiError, LanNetworkError } from './api';

// User-facing texts of the remote SPA. Error codes follow V1-错误码 (1006xxx chat, 1010xxx LAN).

export const NETWORK_ERROR_TEXT = '无法连接到主机，请检查网络后重试';

/** Text for a failed pairing request. */
export function pairErrorText(error: unknown): string {
  if (error instanceof LanNetworkError) return NETWORK_ERROR_TEXT;
  if (!(error instanceof LanApiError)) return '配对失败，请重试';
  switch (error.code) {
    case 1010004:
      return '访问凭据无效或已使用';
    case 1010005:
      return '访问凭据已过期，请在主机上重新生成';
    case 1010007:
      return '当前地址不被允许';
    case 1010011: {
      const scope = error.detail?.scope;
      if (scope === 'session_limit') return '已连接设备过多，请在主机上断开不用的设备';
      const seconds = error.detail?.retry_after_seconds;
      if (typeof seconds === 'number') return `尝试过于频繁，请 ${seconds} 秒后再试`;
      return '尝试过于频繁，请稍后再试';
    }
    default:
      return `配对失败（${error.code || error.status}）`;
  }
}

/** Text for a failed chat/session request. */
export function chatErrorText(error: unknown): string {
  if (error instanceof LanNetworkError) return NETWORK_ERROR_TEXT;
  if (!(error instanceof LanApiError)) return '操作失败，请重试';
  switch (error.code) {
    case 1006001:
      return '会话不存在';
    case 1006007:
      return '消息内容无效';
    case 1006008:
      return '上下文过长，请新建会话';
    case 1006010:
      return '正在生成中';
    case 1006011:
      return '请求标识冲突，请重新发送';
    case 1006012:
      return 'Provider 地址已变化，请在主机上确认后再发送';
    case 1006013:
      return '技能不可用';
    case 1006014:
      return '只能重新生成最后一条回复';
    case 1006015:
      return '会话没有可用的 Provider 或模型';
    case 1010006:
      return '连接已失效';
    case 1010007:
      return '当前地址不被允许';
    case 1010011:
      if (error.detail?.scope === 'chat_sessions') return '本次连接创建的会话已达上限（100）';
      return '请求过于频繁，请稍后再试';
    default:
      if (error.code >= 1001000 && error.code < 1002000) return `模型服务出错（${error.code}）`;
      return `操作失败（${error.code || error.status}）`;
  }
}

/** Why a session ended, from a `session_ended` frame. */
export function sessionEndedText(reason: string | null): string {
  switch (reason) {
    case 'idle_expired':
    case 'absolute_expired':
      return '连接已过期，请在主机上重新生成访问凭据';
    case 'logout':
      return '';
    case 'switch_off':
    case 'app_exit':
    case 'service_failed':
      return '主机已停止服务';
    default:
      return '已被主机断开连接';
  }
}

export const SESSION_LOST_TEXT = '已被主机断开连接';
export const REPLACED_TEXT = '已在其他标签页打开';
export const HOST_STOPPED_TEXT = '主机已停止服务';

/** One line under an assistant message that did not complete. */
export function failedMessageText(
  status: 'interrupted' | 'failed',
  errorCode: string | number | null,
  providerCode: number | null,
): string {
  const head = status === 'interrupted' ? '生成已中断' : '生成失败';
  const code = providerCode ?? (errorCode !== null && errorCode !== '' ? errorCode : null);
  return code === null ? head : `${head}（${code}）`;
}
