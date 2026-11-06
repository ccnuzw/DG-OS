import { useState, type SetStateAction } from 'react';
import { useDevelopment } from '../../stores/developmentStore';

// Account-memory navigation positions only, never file contents or operation/approval state.
const locations = new Map<string, unknown>();
export function useWorkbenchLocation<T>(scope: string, initial: T) {
  const key = JSON.stringify([useDevelopment(state => state.epoch), scope]);
  const [value, update] = useState<T>(() => locations.has(key) ? locations.get(key) as T : initial);
  function setValue(next: SetStateAction<T>) {
    update(previous => {
      const value = typeof next === 'function' ? (next as (previous: T) => T)(previous) : next;
      locations.set(key, value);
      return value;
    });
  }
  return [value, setValue] as const;
}
useDevelopment.subscribe((state, previous) => { if (state.epoch !== previous.epoch) locations.clear(); });
