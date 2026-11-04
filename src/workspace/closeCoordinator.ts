import { Modal } from '@dgos/ui';
import { t } from '../i18n';
import { useChatStore } from '../stores/chatStore';
import { useTaskRunner } from '../stores/taskRunnerStore';
import { useAgentRunner } from '../stores/agentRunnerStore';

export function confirmDiscard(): Promise<boolean> {
  return new Promise(resolve => {
    Modal.confirm({ title: t('shell.unsavedTitle'), content: t('shell.unsavedConfirm'), confirmText: t('shell.discard'), cancelText: t('common.cancel'), confirmVariant: 'danger', onConfirm: () => { resolve(true); }, onCancel: () => { resolve(false); } });
  });
}
export function waitForChatTerminal(sessionIds: readonly string[], timeoutMs = 10_000): Promise<boolean> {
  const terminal = () => sessionIds.every(id => !useChatStore.getState().generations[id]);
  if (terminal()) return Promise.resolve(true);
  return new Promise(resolve => {
    const unsubscribe = useChatStore.subscribe(() => { if (terminal()) finish(true); });
    const timer = setTimeout(() => finish(false), timeoutMs);
    function finish(value: boolean) { clearTimeout(timer); unsubscribe(); resolve(value); }
  });
}
export async function prepareChatClose(): Promise<boolean> {
  const ids = Object.keys(useChatStore.getState().generations);
  if (ids.length) {
    const accepted = await new Promise<boolean>(resolve => {
      Modal.confirm({ title: t('chat.closeSession'), content: t('chat.closeRunningConfirm'), confirmText: t('shell.stopAndClose'), cancelText: t('common.cancel'), confirmVariant: 'danger', onConfirm: () => { resolve(true); }, onCancel: () => { resolve(false); } });
    });
    if (!accepted) return false;
    await Promise.all(ids.map(id => useChatStore.getState().stopSession(id)));
    if (!await waitForChatTerminal(ids)) return false;
    // A newly started generation while consent was pending was never approved for stopping.
    if (Object.keys(useChatStore.getState().generations).length) return false;
  }
  return true;
}

export async function prepareTaskClose(): Promise<boolean> {
  const runner = useTaskRunner.getState();
  const taskId = runner.taskId;
  if (!taskId) return true;
  await runner.cancel();
  const epoch = runner.epoch;
  return new Promise(resolve => {
    let finished = false;
    const terminal = () => {
      const current = useTaskRunner.getState();
      if (current.epoch !== epoch) { finish(false); return; }
      if (current.taskId === null && current.outcome?.record.task.id === taskId) finish(true);
    };
    const unsubscribe = useTaskRunner.subscribe(terminal);
    const poll = window.setInterval(() => { void useTaskRunner.getState().verify(); }, 250);
    const timer = window.setTimeout(() => finish(false), 10_000);
    function finish(value: boolean) {
      if (finished) return;
      finished = true;
      clearInterval(poll); clearTimeout(timer); unsubscribe(); resolve(value);
    }
    terminal();
  });
}

export async function prepareAgentClose(): Promise<boolean> {
  const { epoch } = useAgentRunner.getState();
  // An in-flight open can replace the selected session. Wait for it to settle before choosing which runtimes to close.
  if (useAgentRunner.getState().busy) {
    const ready = await new Promise<boolean>(resolve => {
      const unsubscribe = useAgentRunner.subscribe(state => {
        if (state.epoch !== epoch || !state.busy) finish(state.epoch === epoch);
      });
      const timer = window.setTimeout(() => finish(false), 10_000);
      function finish(value: boolean) { clearTimeout(timer); unsubscribe(); resolve(value); }
    });
    if (!ready) return false;
  }
  const current = useAgentRunner.getState();
  if (current.epoch !== epoch) return false;
  // Every runtime this account started or attached to, including projections detached by a project or task switch.
  const ids = [...new Set([...Object.keys(current.live), ...(current.session && current.session.status !== 'closed' ? [current.session.id] : [])])];
  if (!ids.length) return true;
  for (const id of ids) {
    if (!await useAgentRunner.getState().action('close_agent_session', { sessionId: id })) return false;
    if (useAgentRunner.getState().epoch !== epoch) return false;
  }
  const after = useAgentRunner.getState();
  return after.epoch === epoch && ids.every(id => !(id in after.live));
}
