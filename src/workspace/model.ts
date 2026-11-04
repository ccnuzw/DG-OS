/*
 * V2-FR-009 workspace state (D120, D124).
 *
 * Everything here is memory only: tabs, the launcher and the layout snapshot are not persisted and are
 * rebuilt on restart. The shape follows the approved technical design
 * (docs/03-功能规格/V2/03-产品设计/09-产品设计与UI改版-技术设计.md, §状态模型); keep the two in step.
 */

/** A surface is one entry in the outer tab bar. Only the AI surface carries inner session tabs. */
export type WorkspaceSurface =
  | { kind: 'ai'; id: 'ai' }
  | { kind: 'resource'; id: '/skills' | '/mcp'; title: string }
  | { kind: 'task'; id: 'single-task' | 'cli-agent'; title: string }
  | {
      kind: 'app';
      id: string;
      moduleId: string;
      viewId: string;
      documentId: string | null;
      title: string;
    };

/** Host-reported runtime for one module. The UI never invents a runtimeId (D082). */
export type AppInstanceState = {
  moduleId: string;
  runtimeId: string | null;
  status: 'not_loaded' | 'starting' | 'ready' | 'draining' | 'stopped' | 'failed';
  crashed: boolean;
  permissionBlocked: boolean;
  /** Surface ids that render this module. */
  views: string[];
};

export type LayoutSnapshot = { sidebar: boolean; inspector: boolean; history: boolean; sidebarWidth?: number; inspectorWidth?: number; historyWidth?: number };

export type LauncherState = {
  open: boolean;
  query: string;
  status: 'idle' | 'loading' | 'ready' | 'error';
  dragModuleId: string | null;
  dropTarget: 'none' | 'sidebar';
};

export type WorkspaceState = {
  /** Bumped on account switch; every async result is checked against it before it is written. */
  accountEpoch: number;
  surfaces: WorkspaceSurface[];
  activeSurfaceId: string | null;
  instances: Record<string, AppInstanceState>;
  pinnedModuleIds: string[];
  focusedSurfaceId: string | null;
  layoutSnapshot: LayoutSnapshot | null;
  launcher: LauncherState;
  pending: Record<string, 'opening' | 'closing'>;
};

/** Pin limit from D121: at most three apps in the sidebar app group. */
export const MAX_PINNED = 3;

/** A view is identified by what it shows, so reopening the same document focuses instead of duplicating. */
export function viewIdFor(moduleId: string, documentId: string | null): string {
  return documentId === null ? moduleId : `${moduleId}::${documentId}`;
}

export const AI_SURFACE_ID = 'ai' as const;

export function initialWorkspaceState(accountEpoch = 0): WorkspaceState {
  return {
    accountEpoch,
    surfaces: [],
    activeSurfaceId: null,
    instances: {},
    pinnedModuleIds: [],
    focusedSurfaceId: null,
    layoutSnapshot: null,
    launcher: { open: false, query: '', status: 'idle', dragModuleId: null, dropTarget: 'none' },
    pending: {},
  };
}

export function findSurface(state: WorkspaceState, id: string): WorkspaceSurface | undefined {
  return state.surfaces.find((surface) => surface.id === id);
}
