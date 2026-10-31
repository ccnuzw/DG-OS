import { useState } from 'react';
import { Tabs } from '@dgos/ui';
import ChatWorkbench from '../components/chat/ChatWorkbench';
import SingleTaskWorkbench from './workbench/SingleTaskWorkbench';

// `/workbench` stays one exact route (FR-006 §6.2). The tab lives in page state, not in the URL. Only the active
// panel is mounted, so leaving the chat tab unmounts it and cancels a running generation (B06-61).
type WorkbenchTab = 'chat' | 'single';

export default function Workbench() {
  const [tab, setTab] = useState<WorkbenchTab>('chat');
  return (
    <main className="flex h-full min-w-0 flex-col bg-background text-foreground">
      <header className="px-8 pt-6">
        <h1 className="m-0 text-3xl font-semibold text-foreground">AI 工作台</h1>
      </header>
      <Tabs value={tab} onValueChange={value => setTab(value === 'single' ? 'single' : 'chat')} className="min-h-0 grow gap-0 px-8">
        <Tabs.List aria-label="工作台模式">
          <Tabs.Trigger value="chat">对话</Tabs.Trigger>
          <Tabs.Trigger value="single">单任务</Tabs.Trigger>
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
