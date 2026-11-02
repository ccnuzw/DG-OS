import { useState } from 'react';
import { Tabs } from '@dgos/ui';
import ChatWorkbench from '../components/chat/ChatWorkbench';
import SingleTaskWorkbench from './workbench/SingleTaskWorkbench';
import { useLocale } from '../i18n';

// `/workbench` stays one exact route (FR-006 §6.2). The tab lives in page state, not in the URL. Only the active
// panel is mounted, so leaving the chat tab unmounts it and cancels a running generation (B06-61).
type WorkbenchTab = 'chat' | 'single';

export default function Workbench() {
  const { t } = useLocale();
  const [tab, setTab] = useState<WorkbenchTab>('chat');
  return (
    <main className="flex h-full min-w-0 flex-col bg-background text-foreground">
      <header className="px-8 pt-6">
        <h1 className="m-0 text-3xl font-semibold text-foreground">{t('workbench.title')}</h1>
      </header>
      <Tabs value={tab} onValueChange={value => setTab(value === 'single' ? 'single' : 'chat')} className="min-h-0 grow gap-0 px-8">
        <Tabs.List aria-label={t('workbench.mode')}>
          <Tabs.Trigger value="chat">{t('workbench.chat')}</Tabs.Trigger>
          <Tabs.Trigger value="single">{t('workbench.single')}</Tabs.Trigger>
        </Tabs.List>
        <Tabs.Content value="chat" className="min-h-0 grow">
          <ChatWorkbench />
        </Tabs.Content>
        <Tabs.Content value="single" className="min-h-0 grow overflow-y-auto">
          <SingleTaskWorkbench />
        </Tabs.Content>
      </Tabs>
    </main>
  );
}
