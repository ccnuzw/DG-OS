import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { useDevelopment } from './developmentStore';
import { nativeError, nativeSelectionIdentity } from './nativeExtensions';
import type { NativeDraft, NativeResult, NativeSelection } from '../types/nativeExtensions';
import type { NativeMcpConfigProjection, NativeProbeInput, NativeProbeSnapshot, RegistryArgument, RegistryDeployment, RegistryEntry, RegistryInput, RegistryPage, RegistryParameter } from '../types/nativeMcpCatalog';

export function nativeCatalogKey(selection: NativeSelection): string {
  return JSON.stringify([useDevelopment.getState().epoch, selection.target, selection.scope, selection.project_id, selection.task_id ?? null, nativeSelectionIdentity(selection)]);
}
export function nativeProbeKey(input: NativeProbeInput): string {
  return JSON.stringify([nativeCatalogKey(input.selection), input.source.kind, input.source.kind === 'existing' ? input.source.server_name : input.source.draft.name]);
}
export function nativeProbeRecordKey(input: NativeProbeInput | null, probes: Record<string, ProbeRead>): string | null {
  if (!input) return null;
  const key = nativeProbeKey(input);
  if (probes[key]) return key;
  const selection = input.selection;
  return Object.entries(probes).find(([, record]) => {
    const original = record.snapshot.selection;
    return ['running', 'unknown'].includes(record.snapshot.state)
      && original.target === selection.target && original.scope === selection.scope && original.project_id === selection.project_id && (original.task_id ?? null) === (selection.task_id ?? null)
      && (record.input.source.kind === 'existing' && input.source.kind === 'existing' ? record.input.source.server_name === input.source.server_name : record.input.source.kind === 'draft' && input.source.kind === 'draft' && record.input.source.draft.name === input.source.draft.name);
  })?.[0] ?? key;
}
const errors: Record<string, true> = { registry_unavailable: true, registry_offline: true, registry_rate_limited: true, registry_access_denied: true, registry_invalid: true, registry_limit: true, probe_connect_failed: true, probe_protocol_failed: true, probe_auth_required: true, probe_timeout: true, probe_docker_invocation_unsupported: true };
interface CatalogRead { page: RegistryPage | null; query: string; loading: boolean; error: string | null }
export interface ProbeRead { snapshot: NativeProbeSnapshot; input: NativeProbeInput; error: string | null }
interface CatalogState {
  catalogs: Record<string, CatalogRead>; probes: Record<string, ProbeRead>; busy: Record<string, true>;
  search(selection: NativeSelection, query: string, next?: boolean): Promise<void>;
  resolve(selection: NativeSelection, entry: RegistryEntry): Promise<NativeResult<RegistryEntry>>;
  project(selection: NativeSelection, serverName: string): Promise<NativeResult<NativeMcpConfigProjection>>;
  preview(input: NativeProbeInput): Promise<NativeResult<NativeProbeSnapshot>>;
  operation(selection: NativeSelection, action: 'run' | 'cancel' | 'inspect', probeKey: string): Promise<NativeResult<NativeProbeSnapshot>>;
  follow(probeKey: string): () => void;
}
let sequence = 0;
const requests = new Map<string, number>();
interface ProbeFollowup { probeId: string; context: string; epoch: number; deadline: number; owners: Set<symbol>; timer?: number }
const followups = new Map<string, ProbeFollowup>();
function stopFollowup(key: string, followup: ProbeFollowup) {
  clearTimeout(followup.timer);
  if (followups.get(key) === followup) followups.delete(key);
}
async function call<T>(command: string, input: object, timeoutMs = 20_000): Promise<NativeResult<T>> {
  const epoch = useDevelopment.getState().epoch;
  let timer: number | undefined;
  try {
    // Production targets ES2020; Promise.withResolvers is unavailable there.
    const value = await Promise.race([invoke<T>(command, { input }), new Promise<never>((_, reject) => { timer = window.setTimeout(() => reject('native_extension_Unavailable'), timeoutMs); })]);
    return useDevelopment.getState().epoch === epoch ? { ok: true, value } : { ok: false, code: 'native_extension_StaleGeneration' };
  } catch (error) { return { ok: false, code: useDevelopment.getState().epoch === epoch ? typeof error === 'string' && errors[error] === true ? error : nativeError(error) : 'native_extension_StaleGeneration' }; }
  finally { clearTimeout(timer); }
}
export const useNativeMcpCatalog = create<CatalogState>((set, get) => ({
  catalogs: {}, probes: {}, busy: {},
  async search(selection, query, next = false) {
    const key = nativeCatalogKey(selection), previous = get().catalogs[key];
    if (previous?.loading || (next && (!previous?.page?.next_cursor || previous.query !== query))) return;
    const request = ++sequence; requests.set(key, request);
    set(state => ({ catalogs: { ...state.catalogs, [key]: { page: previous?.page ?? null, query: previous?.query ?? query, loading: true, error: null } } }));
    const result = await call<RegistryPage>('search_native_mcp_registry', { selection, search: query, cursor: next ? previous?.page?.next_cursor : null, refresh: true });
    if (nativeCatalogKey(selection) !== key || requests.get(key) !== request) return;
    set(state => ({ catalogs: { ...state.catalogs, [key]: result.ok ? { page: { ...result.value, servers: next && previous?.page ? [...previous.page.servers, ...result.value.servers.filter(entry => !previous.page?.servers.some(old => old.server.name === entry.server.name && old.server.version === entry.server.version))] : result.value.servers }, query, loading: false, error: result.value.error } : { page: previous?.page ? { ...previous.page, stale: true } : null, query: previous?.query ?? query, loading: false, error: result.code } } }));
  },
  async resolve(selection, entry) {
    const key = nativeCatalogKey(selection);
    const result = await call<RegistryEntry>('resolve_native_mcp_registry', { selection, name: entry.server.name, version: entry.server.version, expected_server: entry.server });
    return key === nativeCatalogKey(selection) ? result : { ok: false, code: 'native_extension_StaleGeneration' };
  },
  async project(selection, serverName) {
    const key = nativeCatalogKey(selection);
    const result = await call<NativeMcpConfigProjection>('project_native_mcp_config', { selection, server_name: serverName });
    return key === nativeCatalogKey(selection) ? result : { ok: false, code: 'native_extension_StaleGeneration' };
  },
  async preview(input) {
    const key = nativeProbeKey(input);
    if (Object.values(get().busy).some(Boolean) || Object.values(get().probes).some(probe => ['running', 'unknown'].includes(probe.snapshot.state))) return { ok: false, code: 'native_extension_Busy' };
    set(state => ({ busy: { ...state.busy, [key]: true } }));
    const result = await call<NativeProbeSnapshot>('preview_native_mcp_probe', input);
    if (nativeProbeKey(input) !== key) { set(state => { const busy = { ...state.busy }; delete busy[key]; return { busy }; }); return { ok: false, code: 'native_extension_StaleGeneration' }; }
    set(state => { const busy = { ...state.busy }; delete busy[key]; return { busy, probes: result.ok ? { ...state.probes, [key]: { snapshot: result.value, input, error: null } } : state.probes }; });
    return result;
  },
  async operation(selection, action, probeKey) {
    const key = probeKey, current = get().probes[key], epoch = useDevelopment.getState().epoch;
    if (!current || get().busy[key]) return { ok: false, code: 'native_extension_Busy' };
    if (action === 'run' && (key !== nativeProbeKey({ ...current.input, selection }) || current.snapshot.state !== 'ready')) return { ok: false, code: 'native_extension_Conflict' };
    set(state => ({ busy: { ...state.busy, [key]: true }, probes: action === 'run' ? { ...state.probes, [key]: { ...current, snapshot: { ...current.snapshot, state: 'unknown', cleanup: 'pending', phase: 'transport' } } } : state.probes }));
    // A pending inspection must not extend the automatic follow-up deadline.
    const followup = action === 'inspect' ? followups.get(key) : undefined;
    const timeout = followup ? Math.min(20_000, Math.max(1, followup.deadline - Date.now())) : 20_000;
    const result = await call<NativeProbeSnapshot>(`${action}_native_mcp_probe`, { probe_id: current.snapshot.probe_id, ...(action === 'run' ? { approved_digest: current.snapshot.digest } : {}) }, timeout);
    if (useDevelopment.getState().epoch !== epoch || get().probes[key]?.snapshot.probe_id !== current.snapshot.probe_id) return { ok: false, code: 'native_extension_StaleGeneration' };
    set(state => {
      const busy = { ...state.busy }; delete busy[key];
      const unknown = action === 'run' || action === 'cancel';
      return { busy, probes: { ...state.probes, [key]: result.ok ? { ...current, snapshot: result.value, error: null } : { ...current, snapshot: unknown ? { ...current.snapshot, state: 'unknown', cleanup: 'unknown', phase: 'transport' } : current.snapshot, error: result.code } } };
    });
    return result;
  },
  follow(key) {
    const current = get().probes[key];
    if (!current || current.snapshot.state !== 'running' || key !== nativeProbeKey(current.input)) return () => {};
    const owner = Symbol(), context = nativeCatalogKey(current.input.selection), epoch = useDevelopment.getState().epoch;
    let followup = followups.get(key);
    if (!followup || followup.probeId !== current.snapshot.probe_id || followup.context !== context || followup.epoch !== epoch) {
      if (followup) stopFollowup(key, followup);
      followup = { probeId: current.snapshot.probe_id, context, epoch, deadline: Date.now() + 30_000, owners: new Set() };
      followups.set(key, followup);
      const tracking = followup;
      const tick = async () => {
        const record = get().probes[key];
        if (followups.get(key) !== tracking || !tracking.owners.size || useDevelopment.getState().epoch !== tracking.epoch || !record || record.snapshot.probe_id !== tracking.probeId || record.snapshot.state !== 'running' || nativeCatalogKey(record.input.selection) !== tracking.context) {
          stopFollowup(key, tracking); return;
        }
        if (Date.now() >= tracking.deadline) {
          set(state => ({ probes: { ...state.probes, [key]: { ...record, error: 'probe_tracking_timeout' } } }));
          stopFollowup(key, tracking); return;
        }
        if (!get().busy[key]) {
          const result = await get().operation(record.snapshot.selection, 'inspect', key);
          if (!result.ok || result.value.state !== 'running') { stopFollowup(key, tracking); return; }
        }
        if (followups.get(key) === tracking && tracking.owners.size) tracking.timer = window.setTimeout(tick, 500);
      };
      tracking.timer = window.setTimeout(tick, 500);
    }
    followup.owners.add(owner);
    const tracking = followup;
    return () => { tracking.owners.delete(owner); if (!tracking.owners.size) stopFollowup(key, tracking); };
  },
}));
useDevelopment.subscribe((next, previous) => {
  if (next.epoch !== previous.epoch) {
    requests.clear();
    for (const [key, followup] of followups) stopFollowup(key, followup);
    useNativeMcpCatalog.setState({ catalogs: {}, probes: {}, busy: {} });
  }
});

export function registryDeployments(entry: RegistryEntry): RegistryDeployment[] {
  return [...(entry.server.remotes ?? []).map((transport, i) => ({ id: `remote:${i}`, label: `${transport.type} · ${transport.url ?? ''}`, transport })), ...(entry.server.packages ?? []).map((item, i) => ({ id: `package:${i}`, label: `${item.registryType} · ${item.identifier}${item.version ? ` @ ${item.version}` : ''}`, transport: item.transport, package: item }))];
}
export function registryParameters(deployment: RegistryDeployment): RegistryParameter[] {
  const parameters: RegistryParameter[] = [];
  const add = (key: string, label: string, input: RegistryInput, reference = false, defaultValue?: string, repeated = false) => parameters.push({ key, label, description: input.description, required: Boolean(input.isRequired), reference: reference || Boolean(input.isSecret), repeated, choices: reference || repeated ? undefined : input.choices, defaultValue: defaultValue ?? input.default ?? input.value ?? '' });
  for (const [name, input] of Object.entries(deployment.transport.variables ?? {})) add(`url:${name}`, name, input);
  for (const [index, header] of (deployment.transport.headers ?? []).entries()) add(`header:${index}`, header.name, { ...header, isRequired: header.isRequired !== false }, true, header.name === 'Authorization' ? 'NATIVE_MCP_AUTHORIZATION' : header.name.toUpperCase().replace(/[^A-Z0-9_]/g, '_'));
  for (const [index, variable] of (deployment.package?.environmentVariables ?? []).entries()) add(`env:${index}`, variable.name, { ...variable, isRequired: variable.isRequired !== false }, true, variable.name);
  for (const [group, arguments_] of [['runtime', deployment.package?.runtimeArguments], ['package', deployment.package?.packageArguments]] as const) {
    for (const [index, argument] of (arguments_ ?? []).entries()) {
      if (argument.variables && Object.keys(argument.variables).length) for (const [name, input] of Object.entries(argument.variables)) add(`${group}:${index}:${name}`, `${argument.name ?? argument.valueHint ?? group}: ${name}`, input, false, undefined, argument.isRepeated);
      else if (argument.value === undefined) add(`${group}:${index}`, argument.name ?? argument.valueHint ?? `${group} ${index + 1}`, argument, false, undefined, argument.isRepeated);
    }
  }
  if (deployment.package && ['pypi', 'cargo'].includes(deployment.package.registryType) && deployment.transport.type === 'stdio') add('entrypoint', 'Package executable', { isRequired: true }, false, deployment.package.identifier);
  return parameters;
}
export class RegistryConfigurationError extends Error { constructor(public readonly code: string) { super(code); } }
export function registryToDraft(entry: RegistryEntry, deployment: RegistryDeployment, values: Record<string, string>, selection: NativeSelection): NativeDraft {
  const fail = (code: string): never => { throw new RegistryConfigurationError(code); };
  const variable = (input: RegistryInput, key: string, supplied?: string): string => {
    if (input.isSecret) return fail('secretArgument');
    const value = supplied ?? values[key] ?? input.value ?? input.default ?? '';
    if (input.isRequired && !value || input.choices?.length && !input.choices.includes(value) || /[\u0000-\u001f]/.test(value)) return fail('invalidParameter');
    return value;
  };
  const interpolate = (template: string, inputs: Record<string, RegistryInput> | undefined, prefix: string, repeat?: number) => template.replace(/\{([^{}]+)\}/g, (_, name: string) => {
    if (!inputs?.[name]) return fail('invalidParameter');
    const repeated = repeat === undefined ? undefined : (values[`${prefix}:${name}`] ?? inputs[name].value ?? inputs[name].default ?? '').split('\n');
    return variable(inputs[name], `${prefix}:${name}`, repeated ? repeated.length === 1 ? repeated[0] : repeated[repeat!] : undefined);
  });
  const args = (arguments_: RegistryArgument[] | null | undefined, group: string): string[] => (arguments_ ?? []).flatMap((argument, index) => {
    if (argument.isSecret) return fail('secretArgument');
    if (!['positional', 'named'].includes(argument.type) || argument.type === 'named' && !argument.name) return fail('unsupportedDeployment');
    const prefix = `${group}:${index}`;
    const supplied = values[prefix] ?? argument.default ?? '';
    const count = argument.isRepeated ? argument.variables ? Math.max(1, ...Object.entries(argument.variables).map(([name, input]) => (values[`${prefix}:${name}`] ?? input.value ?? input.default ?? '').split('\n').length)) : supplied.split('\n').length : 1;
    if (count > 256) return fail('invalidParameter');
    if (argument.isRepeated && Object.entries(argument.variables ?? {}).some(([name, input]) => { const length = (values[`${prefix}:${name}`] ?? input.value ?? input.default ?? '').split('\n').length; return length !== 1 && length !== count; })) return fail('invalidParameter');
    return Array.from({ length: count }, (_, repeat) => {
      const value = argument.value !== undefined ? interpolate(argument.value, argument.variables, prefix, argument.isRepeated ? repeat : undefined) : variable(argument, prefix, argument.isRepeated ? supplied.split('\n')[repeat] : undefined);
      if (!value && !argument.isRequired && argument.value === undefined) return [];
      if (/[\u0000-\u001f]/.test(value) || argument.name && /[\u0000-\u001f]/.test(argument.name)) return fail('invalidParameter');
      return [...(argument.type === 'named' ? [argument.name!] : []), value];
    }).flat();
  });
  const draft: NativeDraft = { slug: '', source: `https://registry.modelcontextprotocol.io/v0/servers/${encodeURIComponent(entry.server.name)}/versions/${encodeURIComponent(entry.server.version)}`, name: entry.server.name.split('/')[1] ?? '', transport: 'Stdio', program: '', args: '', url: '', cwd: '', bearer_env: '', forwarded_env: '', header_env: '', modes: { transport: 'set' } };
  const headers = (deployment.transport.headers ?? []).flatMap((header, index) => { const name = values[`header:${index}`] ?? registryParameters(deployment).find(parameter => parameter.key === `header:${index}`)?.defaultValue ?? ''; if (!name && !header.isRequired) return []; if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) return fail('invalidParameter'); return [`${header.name}=${name}`]; });
  if (headers.length) { draft.header_env = headers.join('\n'); draft.modes.header_env = 'set'; }
  if (deployment.transport.type === 'streamable-http' || deployment.transport.type === 'sse') {
    if (deployment.transport.type === 'sse' && selection.target !== 'Claude') return fail('unsupportedDeployment');
    draft.transport = deployment.transport.type === 'sse' ? 'Sse' : 'Http';
    draft.url = interpolate(deployment.transport.url ?? '', deployment.transport.variables, 'url'); draft.modes.url = 'set';
    if (!draft.url) return fail('invalidParameter');
    return draft;
  }
  if (deployment.transport.type !== 'stdio' || !deployment.package) return fail('unsupportedDeployment');
  const item = deployment.package;
  if (!item.identifier || /[\s\u0000-\u001f]/.test(item.identifier) || item.version && /[\s\u0000-\u001f]/.test(item.version)) return fail('invalidParameter');
  if (item.fileSha256) return fail('integrityRequired');
  const forwarded = (item.environmentVariables ?? []).flatMap((variable_, index) => { const name = values[`env:${index}`] ?? variable_.name; if (!name && !variable_.isRequired) return []; if (name !== variable_.name || !/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) return fail('environmentName'); return [name]; });
  const runtime = args(item.runtimeArguments, 'runtime'), packageArgs = args(item.packageArguments, 'package');
  if (item.registryType === 'npm' && (!item.runtimeHint || item.runtimeHint === 'npx')) { draft.program = 'npx'; draft.args = ['--yes', ...runtime, `${item.identifier}${item.version ? `@${item.version}` : ''}`, ...packageArgs].join('\n'); }
  else if (item.registryType === 'pypi' && (!item.runtimeHint || item.runtimeHint === 'uvx')) { draft.program = 'uvx'; draft.args = [...runtime, '--from', `${item.identifier}${item.version ? `==${item.version}` : ''}`, values.entrypoint || item.identifier, ...packageArgs].join('\n'); }
  else if (item.registryType === 'oci' && (!item.runtimeHint || item.runtimeHint === 'docker')) { draft.program = 'docker'; draft.args = ['run', '--rm', '-i', ...forwarded.flatMap(name => ['-e', name]), ...runtime, `${item.identifier}${item.version && !item.identifier.includes('@') && !item.identifier.split('/').pop()?.includes(':') ? `:${item.version}` : ''}`, ...packageArgs].join('\n'); }
  else if (item.registryType === 'cargo' && !item.runtimeHint && !runtime.length) { draft.program = values.entrypoint || item.identifier; draft.args = packageArgs.join('\n'); }
  else return fail('unsupportedDeployment');
  if (item.registryBaseUrl && !['https://registry.npmjs.org', 'https://pypi.org', 'https://pypi.org/simple', 'https://registry-1.docker.io', 'https://docker.io'].includes(item.registryBaseUrl.replace(/\/$/, ''))) {
    if (item.registryType === 'npm') draft.args = ['--registry', item.registryBaseUrl, draft.args].join('\n');
    else if (item.registryType === 'pypi') draft.args = ['--index-url', item.registryBaseUrl, draft.args].join('\n');
    else return fail('unsupportedDeployment');
  }
  draft.forwarded_env = forwarded.join('\n'); draft.modes.program = 'set'; draft.modes.args = 'set'; if (forwarded.length) draft.modes.forwarded_env = 'set';
  return draft;
}
