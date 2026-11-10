import type { SetStateAction } from 'react';
import { useDevelopment } from '../../stores/developmentStore';
import { desktopSlot, useDesktopSlot } from '../../stores/developmentDesktop';

// Account-private navigation positions only, never file contents or approval state.
export function useWorkbenchLocation<T>(scope: string, initial: T) {
  const epoch = useDevelopment(state => state.epoch);
  const key = `workbench-location:${scope}`;
  const [value, update] = useDesktopSlot<T>(key, initial);
  function setValue(next: SetStateAction<T>) {
    if (epoch !== useDevelopment.getState().epoch) return;
    const previous = desktopSlot<T>(key, initial);
    update(typeof next === 'function' ? (next as (previous: T) => T)(previous) : next);
  }
  return [value, setValue] as const;
}
