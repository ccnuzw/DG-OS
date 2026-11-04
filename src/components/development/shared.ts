import { Modal } from '@dgos/ui';
import { t } from '../../i18n';
import { hostCode } from '../../stores/hostError';

// Resolves a host failure code to its bilingual message. Unknown codes fall back to `dev.error.unknown`.
export function errorText(code: string): string {
  return t(`dev.error.${hostCode(code)}`);
}

export function confirmAction(title: string, content: string, confirmText: string): Promise<boolean> {
  return new Promise<boolean>(resolve => {
    Modal.confirm({ title, content, confirmText, cancelText: t('dev.cancel'), confirmVariant: 'danger', onConfirm: () => { resolve(true); }, onCancel: () => { resolve(false); } });
  });
}
