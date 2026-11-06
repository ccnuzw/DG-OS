import { act, cleanup, render } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { createWorkspaceController } from '../src/workspace/controller';
import { SurfaceScope, useSurfaceCloseGuard, useSurfaceStatus } from '../src/workspace/SurfaceContext';

afterEach(cleanup);

function Editor({ allow, guard }: { allow: boolean; guard?: () => Promise<boolean> }) {
  useSurfaceCloseGuard(guard ?? (async () => allow));
  useSurfaceStatus({ dirty: !allow, running: true });
  return <input aria-label="draft" />;
}

it('uses the latest guard while hidden and retains running/dirty status across activity changes', async () => {
  const controller = createWorkspaceController();
  controller.open({ kind: 'ai', id: 'ai' });
  const view = render(<SurfaceScope id="ai" active controller={controller}><Editor allow /></SurfaceScope>);
  expect(await controller.canLeave('ai')).toBe(true);
  view.rerender(<SurfaceScope id="ai" active={false} controller={controller}><Editor allow={false} /></SurfaceScope>);
  expect(await controller.requestClose('ai')).toBe(false);
  expect(controller.getSurfaceStatuses().ai).toEqual({ dirty: true, running: true });
  view.rerender(<SurfaceScope id="ai" active controller={controller}><Editor allow={false} /></SurfaceScope>);
  expect(await controller.canLeave('ai')).toBe(false);
  view.rerender(<SurfaceScope id="ai" active={false} controller={controller}><Editor allow /></SurfaceScope>);
  await act(async () => { expect(await controller.requestClose('ai')).toBe(true); });
  expect(controller.getSnapshot().surfaces.some(surface => surface.id === 'ai')).toBe(false);
  view.unmount();
  expect(controller.getSurfaceStatuses().ai).toEqual({ dirty: false, running: false });
});

it('keeps a blocking guard in a close already underway when activity changes', async () => {
  const controller = createWorkspaceController();
  controller.open({ kind: 'ai', id: 'ai' });
  let resolve!: (allowed: boolean) => void;
  let entered!: () => void;
  const started = new Promise<void>(done => { entered = done; });
  const pending = new Promise<boolean>(done => { resolve = done; });
  const view = render(<SurfaceScope id="ai" active controller={controller}>
    <Editor allow guard={async () => { entered(); return pending; }} /><Editor allow={false} />
  </SurfaceScope>);
  const closing = controller.requestClose('ai');
  await started;
  view.rerender(<SurfaceScope id="ai" active={false} controller={controller}>
    <Editor allow guard={async () => true} /><Editor allow={false} />
  </SurfaceScope>);
  resolve(true);
  expect(await closing).toBe(false);
  expect(controller.getSnapshot().surfaces.some(surface => surface.id === 'ai')).toBe(true);
});
