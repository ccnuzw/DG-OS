// V2-FR-009 AC15/AC16/AC17/AC19 (logic half): the workspace reducer.
// The UI half lives in a later slice; these cases fix tab identity, detach semantics, pin limit,
// focus snapshot, instance reconciliation and account reset before any component exists.
import { describe, expect, it } from 'vitest';
import { initialWorkspaceState, MAX_PINNED, viewIdFor, type AppInstanceState } from '../src/workspace/model';
import { reduce, type WorkspaceAction } from '../src/workspace/reducer';

const run = (state = initialWorkspaceState(), ...actions: WorkspaceAction[]) =>
  actions.reduce((current, action) => reduce(current, action).state, state);

const open = (moduleId: string, extra: Partial<Extract<WorkspaceAction, { type: 'app/open' }>> = {}) =>
  ({
    type: 'app/open',
    moduleId,
    title: moduleId,
    documentId: null,
    usable: true,
    runtimeReady: true,
    ...extra,
  }) as WorkspaceAction;

const instance = (moduleId: string, extra: Partial<AppInstanceState> = {}): AppInstanceState => ({
  moduleId,
  runtimeId: `rt-${moduleId}`,
  status: 'ready',
  crashed: false,
  permissionBlocked: false,
  views: [],
  ...extra,
});

describe('view identity', () => {
  it('derives one stable id per module and document', () => {
    expect(viewIdFor('canvas', null)).toBe('canvas');
    expect(viewIdFor('canvas', 'doc-1')).toBe('canvas::doc-1');
    expect(viewIdFor('canvas', 'doc-1')).toBe(viewIdFor('canvas', 'doc-1'));
  });
});

describe('open (AC15)', () => {
  it('creates a tab and activates it', () => {
    const { state, result } = reduce(initialWorkspaceState(), open('canvas'));
    expect(result).toBe('created');
    expect(state.surfaces).toHaveLength(1);
    expect(state.activeSurfaceId).toBe('canvas');
  });

  it('focuses the existing tab instead of creating a second one', () => {
    const first = run(initialWorkspaceState(), open('canvas'), open('skills'));
    const { state, result } = reduce(first, open('canvas'));
    expect(result).toBe('focused');
    expect(state.surfaces).toHaveLength(2);
    expect(state.activeSurfaceId).toBe('canvas');
  });

  it('treats a different document as a second view of the same module', () => {
    const state = run(initialWorkspaceState(), open('canvas', { documentId: 'a' }), open('canvas', { documentId: 'b' }));
    expect(state.surfaces).toHaveLength(2);
    expect(state.surfaces.every((surface) => surface.kind === 'app' && surface.moduleId === 'canvas')).toBe(true);
  });

  it('refuses a stale account generation', () => {
    const state = initialWorkspaceState(7);
    const { result, state: next } = reduce(state, open('canvas', { epoch: 6 }));
    expect(result).toBe('blocked');
    expect(next).toBe(state);
  });

  it('sends an unusable module to management instead of opening it', () => {
    const { result } = reduce(initialWorkspaceState(), open('canvas', { usable: false }));
    expect(result).toBe('needs-management');
  });

  it('does not open an app whose runtime is not live yet', () => {
    const { result, state } = reduce(initialWorkspaceState(), open('canvas', { runtimeReady: false }));
    expect(result).toBe('needs-management');
    expect(state.surfaces).toHaveLength(0);
  });

  it('never starts a module: no host call is modelled here', () => {
    const { state } = reduce(initialWorkspaceState(), open('canvas'));
    expect(state.instances.canvas).toBeUndefined();
  });
});

describe('close (AC15, AC19)', () => {
  it('detaches the view but keeps the module instance', () => {
    const opened = run(
      initialWorkspaceState(),
      { type: 'instances/reconcile', instances: [instance('canvas', { views: ['canvas'] })] },
      open('canvas'),
    );
    const { state, result } = reduce(opened, { type: 'surface/close', surfaceId: 'canvas' });
    expect(result).toBe('closed');
    expect(state.surfaces).toHaveLength(0);
    // The module is still running; only its view went away.
    expect(state.instances.canvas).toBeDefined();
    expect(state.instances.canvas.views).not.toContain('canvas');
  });

  it('activates the neighbouring tab and clears the last one', () => {
    const two = run(initialWorkspaceState(), open('a'), open('b'));
    const afterB = reduce(two, { type: 'surface/close', surfaceId: 'b' }).state;
    expect(afterB.activeSurfaceId).toBe('a');
    const afterA = reduce(afterB, { type: 'surface/close', surfaceId: 'a' }).state;
    expect(afterA.activeSurfaceId).toBeNull();
  });

  it('reports an already closed view', () => {
    expect(reduce(initialWorkspaceState(), { type: 'surface/close', surfaceId: 'nope' }).result).toBe('already-closed');
  });

  it('closing the focused tab also drops the layout snapshot', () => {
    const focused = run(
      initialWorkspaceState(),
      open('canvas'),
      { type: 'surface/focus-toggle', surfaceId: 'canvas', layout: { sidebar: true, inspector: false, history: true } },
    );
    const { state } = reduce(focused, { type: 'surface/close', surfaceId: 'canvas' });
    expect(state.focusedSurfaceId).toBeNull();
    expect(state.layoutSnapshot).toBeNull();
  });
});

describe('pin (AC16)', () => {
  it('pins, ignores duplicates and stops at three', () => {
    const pinned = run(
      initialWorkspaceState(),
      { type: 'app/pin', moduleId: 'a' },
      { type: 'app/pin', moduleId: 'b' },
      { type: 'app/pin', moduleId: 'c' },
    );
    expect(pinned.pinnedModuleIds).toEqual(['a', 'b', 'c']);
    expect(reduce(pinned, { type: 'app/pin', moduleId: 'a' }).result).toBe('already-pinned');
    expect(reduce(pinned, { type: 'app/pin', moduleId: 'd' }).result).toBe('limit');
    expect(pinned.pinnedModuleIds).toHaveLength(MAX_PINNED);
  });

  it('pins through a launcher drop and clears the drag state', () => {
    const dragging = run(
      initialWorkspaceState(),
      { type: 'launcher/open' },
      { type: 'launcher/drag-start', moduleId: 'canvas' },
    );
    const { state, result } = reduce(dragging, {
      type: 'launcher/drop-to-pin',
      moduleId: 'canvas',
      target: 'sidebar',
    });
    expect(result).toBe('pinned');
    expect(state.pinnedModuleIds).toEqual(['canvas']);
    expect(state.launcher.dragModuleId).toBeNull();
    expect(state.launcher.dropTarget).toBe('none');
  });

  it('ignores a drop outside the sidebar', () => {
    const dragging = run(
      initialWorkspaceState(),
      { type: 'launcher/open' },
      { type: 'launcher/drag-start', moduleId: 'canvas' },
    );
    expect(
      reduce(dragging, { type: 'launcher/drop-to-pin', moduleId: 'canvas', target: 'none' }).result,
    ).toBe('noop');
  });

  it('does not start a drag while the launcher is closed', () => {
    expect(reduce(initialWorkspaceState(), { type: 'launcher/drag-start', moduleId: 'canvas' }).result).toBe('blocked');
  });

  it('unpins only what is pinned', () => {
    const pinned = run(initialWorkspaceState(), { type: 'app/pin', moduleId: 'a' });
    expect(reduce(pinned, { type: 'app/unpin', moduleId: 'a' }).state.pinnedModuleIds).toEqual([]);
    expect(reduce(pinned, { type: 'app/unpin', moduleId: 'zzz' }).result).toBe('noop');
  });
});

describe('focus (AC17)', () => {
  const layout = { sidebar: true, inspector: true, history: false };

  it('stores the layout once and restores it on toggle off', () => {
    const on = reduce(run(initialWorkspaceState(), open('canvas')), {
      type: 'surface/focus-toggle',
      surfaceId: 'canvas',
      layout,
    });
    expect(on.result).toBe('focused-layout');
    expect(on.state.focusedSurfaceId).toBe('canvas');
    expect(on.state.layoutSnapshot).toEqual(layout);

    const off = reduce(on.state, { type: 'surface/focus-toggle', surfaceId: 'canvas', layout: { sidebar: false, inspector: false, history: false } });
    expect(off.result).toBe('restored-layout');
    expect(off.state.focusedSurfaceId).toBeNull();
    expect(off.state.layoutSnapshot).toBeNull();
    expect(off.restoreLayout).toEqual(layout);
  });

  it('does not overwrite the snapshot when switching focus to another tab', () => {
    const two = run(initialWorkspaceState(), open('a'), open('b'));
    const first = reduce(two, { type: 'surface/focus-toggle', surfaceId: 'a', layout }).state;
    const second = reduce(first, { type: 'surface/focus-toggle', surfaceId: 'b', layout: { sidebar: false, inspector: false, history: false } }).state;
    expect(second.focusedSurfaceId).toBe('b');
    expect(second.layoutSnapshot).toEqual(layout);
    const closed = reduce(second, { type: 'surface/close', surfaceId: 'b' });
    expect(closed.restoreLayout).toEqual(layout);
  });
});

describe('instances and account (AC19)', () => {
  it('replaces the instance map from a host snapshot', () => {
    const { state } = reduce(initialWorkspaceState(), {
      type: 'instances/reconcile',
      instances: [instance('canvas', { status: 'failed', crashed: true })],
    });
    expect(state.instances.canvas.crashed).toBe(true);
  });

  it('lists a view on the module that renders it', () => {
    const state = run(
      initialWorkspaceState(),
      { type: 'instances/reconcile', instances: [instance('canvas')] },
      open('canvas', { documentId: 'doc-1' }),
    );
    expect(state.instances.canvas.views).toEqual(['canvas::doc-1']);
  });

  it('clears tabs, pins, launcher and focus on account switch and bumps the epoch', () => {
    const busy = run(
      initialWorkspaceState(3),
      open('canvas'),
      { type: 'app/pin', moduleId: 'canvas' },
      { type: 'launcher/open' },
      { type: 'surface/focus-toggle', surfaceId: 'canvas', layout: { sidebar: true, inspector: false, history: false } },
    );
    const { state, result } = reduce(busy, { type: 'account/reset', epoch: 4 });
    expect(result).toBe('reset');
    expect(state.accountEpoch).toBe(4);
    expect(state.surfaces).toEqual([]);
    expect(state.pinnedModuleIds).toEqual([]);
    expect(state.instances).toEqual({});
    expect(state.launcher.open).toBe(false);
    expect(state.focusedSurfaceId).toBeNull();
  });
});

describe('purity', () => {
  it('never mutates the state it is given', () => {
    const before = run(initialWorkspaceState(), open('canvas'));
    const snapshot = JSON.stringify(before);
    reduce(before, open('skills'));
    reduce(before, { type: 'surface/close', surfaceId: 'canvas' });
    reduce(before, { type: 'app/pin', moduleId: 'canvas' });
    expect(JSON.stringify(before)).toBe(snapshot);
  });

  it('keeps the AI surface unique', () => {
    const state = run(initialWorkspaceState(), { type: 'ai/open' }, { type: 'ai/open' });
    expect(state.surfaces.filter((surface) => surface.kind === 'ai')).toHaveLength(1);
    expect(state.activeSurfaceId).toBe('ai');
  });
});
