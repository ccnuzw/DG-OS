import { useSyncExternalStore } from 'react';
import { useAgentRunner } from './agentRunnerStore';
import { useDevelopment, type Outcome, type Selection } from './developmentStore';
import { discardDevelopmentDraft, useDevelopmentEditor } from './developmentEditor';

let snapshot = { generation: 0, pending: false };
const listeners = new Set<() => void>();
function publish(pending: boolean) {
  snapshot = { generation: snapshot.generation + 1, pending };
  listeners.forEach(listener => listener());
}
useDevelopment.subscribe((state, previous) => {
  if (state.epoch !== previous.epoch) publish(false);
});
export function contextGeneration(): number { return snapshot.generation; }
export function useContextSelection() {
  return useSyncExternalStore(listener => { listeners.add(listener); return () => { listeners.delete(listener); }; }, () => snapshot, () => snapshot);
}

// Invalidate the projection at selection intent, including an open/attach whose first snapshot has not arrived.
// The runtime itself stays tracked and keeps running. Only the latest selection may reveal its new projection.
export async function chooseContext(next: Selection): Promise<Outcome<null>> {
  const epoch = useDevelopment.getState().epoch;
  publish(true);
  const generation = snapshot.generation;
  const previous = useDevelopment.getState().selection;
  if (previous.project_id !== next.project_id || previous.task_id !== next.task_id) {
    if (!await discardDevelopmentDraft()) {
      if (epoch === useDevelopment.getState().epoch && snapshot.generation === generation) publish(false);
      return { ok: false, code: 'cancelled' };
    }
    if (epoch !== useDevelopment.getState().epoch || snapshot.generation !== generation) return { ok: false, code: 'cancelled' };
    useDevelopmentEditor.getState().reset();
  }
  const runner = useAgentRunner.getState();
  if (runner.selected !== next.session_id || next.session_id === null) runner.detach();
  const result = await useDevelopment.getState().select(next);
  if (snapshot.generation === generation) publish(false);
  return result;
}
