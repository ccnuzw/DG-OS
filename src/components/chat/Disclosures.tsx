import { t, useLocale as usePageLocale } from '../../i18n';
import { cn } from '@dgos/ui';
import { FOCUS } from './ChatNotice';

// Mandatory disclosures of FR-006 (D072, D074, D075, D022). Open by default so they are visible on the page.

export const DELETE_CONFIRM_TEXT = '确定删除？应用内将无法恢复。已删除内容可能残留在数据库空闲页和备份中。';

export const DISCLOSURES: readonly string[] = [
  t('chat.disclosure1'),
  t('chat.disclosure2'),
  t('chat.disclosure3'),
  t('chat.disclosure4'),
  t('chat.disclosure5'),
  t('chat.disclosure6'),
];

export default function Disclosures() {
  usePageLocale();
  return (
    <details open className="rounded-md border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">
      <summary className={cn('cursor-pointer font-medium text-foreground', FOCUS)}>{t('chat.privacy')}</summary>
      <ul className="m-0 mt-2 list-disc pl-5">
        {Array.from({ length: 6 }, (_, i) => `chat.disclosure${i + 1}`).map(key => <li key={key} className="my-0.5">{t(key)}</li>)}
      </ul>
    </details>
  );
}
