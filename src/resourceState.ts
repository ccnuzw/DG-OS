// Read metadata only. Resource data stays with its existing owner.
export type ResourceRefresh =
  | { status: 'current' }
  | { status: 'refreshing'; error?: string }
  | { status: 'stale' }
  | { status: 'error'; error: string };

export type ResourceReadState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'error'; error: string }
  | { status: 'ready'; refresh: ResourceRefresh };

export type ResourceViewState =
  | Exclude<ResourceReadState, { status: 'ready' }>
  | { status: 'ready'; empty: boolean; refresh: ResourceRefresh };

export function resourceView(read: ResourceReadState, empty: boolean): ResourceViewState {
  return read.status === 'ready' ? { ...read, empty } : read;
}

export function beginResourceRead(state: ResourceReadState): ResourceReadState {
  return state.status === 'ready'
    ? { status: 'ready', refresh: { status: 'refreshing', ...(state.refresh.status === 'error' || state.refresh.status === 'refreshing' ? { error: state.refresh.error } : {}) } }
    : { status: 'loading' };
}

export function failResourceRead(state: ResourceReadState, error: string): ResourceReadState {
  return state.status === 'ready'
    ? { status: 'ready', refresh: { status: 'error', error } }
    : { status: 'error', error };
}
