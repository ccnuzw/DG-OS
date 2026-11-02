import { LanApiError, LanNetworkError } from './api';
import { t } from './i18n';

// User-facing texts of the remote SPA. Error codes follow V1-错误码 (1006xxx chat, 1010xxx LAN).

export const networkErrorText = () => t('remote.networkError');

/** Text for a failed pairing request. */
export function pairErrorText(error: unknown): string {
  if (error instanceof LanNetworkError) return networkErrorText();
  if (!(error instanceof LanApiError)) return t('remote.pairFailed');
  switch (error.code) {
    case 1010004:
      return t('remote.codeInvalid');
    case 1010005:
      return t('remote.codeExpired');
    case 1010007:
      return t('remote.addressDenied');
    case 1010011: {
      const scope = error.detail?.scope;
      if (scope === 'session_limit') return t('remote.deviceLimit');
      const seconds = error.detail?.retry_after_seconds;
      if (typeof seconds === 'number') return t('remote.retrySeconds', { seconds });
      return t('remote.tooFrequent');
    }
    default:
      return t('remote.pairFailedCode', { code: error.code || error.status });
  }
}

/** Text for a failed chat/session request. */
export function chatErrorText(error: unknown): string {
  if (error instanceof LanNetworkError) return networkErrorText();
  if (!(error instanceof LanApiError)) return t('remote.operationFailed');
  switch (error.code) {
    case 1006001:
      return t('remote.sessionMissing');
    case 1006007:
      return t('remote.invalidMessage');
    case 1006008:
      return t('remote.contextLong');
    case 1006010:
      return t('remote.alreadyGenerating');
    case 1006011:
      return t('remote.requestConflict');
    case 1006012:
      return t('remote.destinationChanged');
    case 1006013:
      return t('remote.skillUnavailable');
    case 1006014:
      return t('remote.lastReplyOnly');
    case 1006015:
      return t('remote.noSessionTarget');
    case 1010006:
      return t('remote.connectionExpired');
    case 1010007:
      return t('remote.addressDenied');
    case 1010011:
      if (error.detail?.scope === 'chat_sessions') return t('remote.sessionLimit');
      return t('remote.tooFrequent');
    default:
      if (error.code >= 1001000 && error.code < 1002000) return t('remote.providerError', { code: error.code });
      return t('remote.operationFailedCode', { code: error.code || error.status });
  }
}

/** Why a session ended, from a `session_ended` frame. */
export function sessionEndedText(reason: string | null): string {
  switch (reason) {
    case 'idle_expired':
    case 'absolute_expired':
      return t('remote.expiredNotice');
    case 'logout':
      return '';
    case 'switch_off':
    case 'app_exit':
    case 'service_failed':
      return t('remote.hostStopped');
    default:
      return t('remote.disconnected');
  }
}

export const sessionLostText = () => t('remote.disconnected');
export const replacedText = () => t('remote.replaced');
export const hostStoppedText = () => t('remote.hostStopped');

/** One line under an assistant message that did not complete. */
export function failedMessageText(
  status: 'interrupted' | 'failed',
  errorCode: string | number | null,
  providerCode: number | null,
): string {
  const head = status === 'interrupted' ? t('remote.status.interrupted') : t('remote.status.failed');
  const code = providerCode ?? (errorCode !== null && errorCode !== '' ? errorCode : null);
  return code === null ? head : `${head}（${code}）`;
}
