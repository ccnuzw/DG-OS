import { create } from 'zustand';
import { useDevelopmentRuntime } from './developmentRuntime';
import { useDevelopment } from './developmentStore';
import { confirmAction } from '../components/development/shared';
import { t } from '../i18n';
import { desktopSlot, onDesktopStateRestored, rebaseDesktopKey, setDesktopSlot } from './developmentDesktop';

type DraftKind = 'task' | 'capabilities' | 'commit' | 'handoff' | 'delivery' | 'conversation';
interface Draft { kind: DraftKind; value: unknown; baseline: unknown }
interface Drafts { entries: Record<string, Draft>; handoffs: Record<string, string>; clear(): void }
export const useDevelopmentDrafts = create<Drafts>(set => ({ entries: {}, handoffs: {}, clear: () => set({ entries: {}, handoffs: {} }) }));
export function developmentDraftKey(kind: DraftKind, ...scope: (string | number | null)[]) {
  return JSON.stringify([useDevelopment.getState().epoch, kind, ...scope]);
}
export function useScopedDevelopmentDraft<T>(kind: DraftKind, scope: (string | number | null)[], initial: T) {
  const epoch = useDevelopment(state => state.epoch);
  const key = JSON.stringify([epoch, kind, ...scope]);
  const entry = useDevelopmentDrafts(state => state.entries[key]);
  const value = entry ? entry.value as T : initial;
  function setValue(next: T) {
    if (epoch !== useDevelopment.getState().epoch) return;
    useDevelopmentDrafts.setState(state => {
      const baseline = state.entries[key]?.baseline ?? initial;
      const entries = { ...state.entries };
      if (JSON.stringify(next) === JSON.stringify(baseline)) delete entries[key];
      else entries[key] = { kind, value: next, baseline };
      return { entries };
    });
  }
  function clear(expected?: T) {
    if (epoch !== useDevelopment.getState().epoch) return;
    useDevelopmentDrafts.setState(state => {
      if (expected !== undefined && state.entries[key] && JSON.stringify(state.entries[key].value) !== JSON.stringify(expected)) return state;
      const entries = { ...state.entries }; delete entries[key]; return { entries };
    });
  }
  async function discard() {
    const before = useDevelopmentDrafts.getState().entries[key];
    if (!before) return true;
    if (!await confirmAction(t('dev.discardDraft'), t(`dev.discardDraft.${kind}`), t('dev.discardDraft')) || epoch !== useDevelopment.getState().epoch || useDevelopmentDrafts.getState().entries[key] !== before) return false;
    clear(); return true;
  }
  return { value, setValue, clear, discard, dirty: Boolean(entry) };
}
export async function guardWorkbenchDrafts(projectId?: string) {
  const entries = useDevelopmentDrafts.getState().entries;
  const affected = Object.entries(entries).filter(([key]) => !projectId || JSON.parse(key)[2] === projectId);
  const runtimeDrafts = useDevelopmentRuntime.getState().drafts;
  const commands = Object.entries(runtimeDrafts).filter(([key, value]) => value.dirty && (!projectId || JSON.parse(key)[1] === projectId));
  if (!affected.length && !commands.length) return true;
  const descriptions = [...new Set(affected.map(([, entry]) => t(`dev.discardDraft.${entry.kind}`))), ...(commands.length ? [t('dev.discardDraft.command')] : [])];
  const epoch = useDevelopment.getState().epoch;
  return await confirmAction(t('dev.discardDraft'), descriptions.join('\n'), t('dev.discardDraft')) && epoch === useDevelopment.getState().epoch && entries === useDevelopmentDrafts.getState().entries && runtimeDrafts === useDevelopmentRuntime.getState().drafts;
}
useDevelopment.subscribe((state, previous) => {
  if (state.epoch !== previous.epoch) useDevelopmentDrafts.getState().clear();
});

// The first tuple member is a renderer lifetime, never an account identity. Rebase
// it on hydration; leave native generations and unknown request identities intact.
function recoveredDraftValue(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(recoveredDraftValue);
  if (!value || typeof value !== 'object') return value;
  return Object.fromEntries(Object.entries(value).map(([key, entry]) => [key,
    ['preparing', 'busy', 'saving'].includes(key) ? false
      : key === 'status' && entry === 'pending' ? 'unknown' : recoveredDraftValue(entry),
  ]));
}
onDesktopStateRestored(() => {
  const saved = desktopSlot<{ entries?: Record<string, Draft>; handoffs?: Record<string, string> }>('drafts', {});
  const epoch = useDevelopment.getState().epoch;
  const entries: Record<string, Draft> = {};
  for (const [key, entry] of Object.entries(saved.entries ?? {})) {
    const rebased = rebaseDesktopKey(key, epoch);
    if (!rebased || !entry || !['task', 'capabilities', 'commit', 'handoff', 'delivery', 'conversation'].includes(entry.kind)) continue;
    entries[rebased] = { kind: entry.kind, value: recoveredDraftValue(entry.value), baseline: recoveredDraftValue(entry.baseline) };
    const scope: unknown[] = JSON.parse(rebased);
    const value = entries[rebased].value;
    if (entry.kind === 'task' && scope[scope.length - 1] === 'workspace-preparation' && value && typeof value === 'object' && 'phase' in value && value.phase === 'pending') entries[rebased].value = { ...value, phase: 'unknown' };
  }
  const handoffs = Object.fromEntries(Object.entries(saved.handoffs ?? {}).flatMap(([key, id]) => {
    const rebased = rebaseDesktopKey(key, epoch);
    return rebased && typeof id === 'string' ? [[rebased, id]] : [];
  }));
  useDevelopmentDrafts.setState(state => ({ entries: { ...entries, ...state.entries }, handoffs: { ...handoffs, ...state.handoffs } }));
});
useDevelopmentDrafts.subscribe((state, previous) => {
  if (state.entries !== previous.entries || state.handoffs !== previous.handoffs) setDesktopSlot('drafts', { entries: state.entries, handoffs: state.handoffs });
});
