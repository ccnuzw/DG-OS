// Page side of the real module-loading slice (D042-D046).
//
// The Rust host sends load/unload instructions over a Tauri Channel. This class imports the module
// source through a Blob URL, mounts its component with the host's React, runs the lifecycle hooks
// and reports a receipt back. Receipts carry only the stage enum: error messages, stacks, module
// ids and paths stay in the local console. Modules run in this same JS realm and are NOT sandboxed
// (D042); the only security boundary is the D041 admission done by the host.
import * as React from 'react';
import { flushSync } from 'react-dom';
import { createRoot, type Root } from 'react-dom/client';
import { Channel, invoke as tauriInvoke } from '@tauri-apps/api/core';
import { moduleHostUi, type ModuleHostUi } from './moduleHostUi';

export type LoadFailureStage = 'import' | 'contract' | 'render' | 'hook' | 'timeout' | 'cancelled';

export interface LoadInstruction {
  kind: 'load';
  ticket: string;
  runtimeId: string;
  moduleId: string;
  version: string;
  source: string;
  hookTimeoutMs: number;
  // runtimeId of the already-loaded instance of the same module that this load replaces (hot update, D055).
  replaces?: string | null;
}

export interface UnloadInstruction {
  kind: 'unload';
  ticket: string;
  runtimeId: string;
  hookTimeoutMs: number;
}

// Pushed by the host once the runtime is ready (D051). It carries the routing key only: no ticket,
// no receipt, nothing the module could present as identity or authority.
export interface RouteInstruction {
  kind: 'route';
  runtimeId: string;
  route: string;
}

export type HostInstruction = LoadInstruction | UnloadInstruction | RouteInstruction;

export interface ProviderChatRequest {
  messages: Array<{ role: string; content: string }>;
  temperature?: number;
  max_tokens?: number;
  stream?: boolean;
}

export interface ProviderChatResponse {
  content: string;
  [key: string]: unknown;
}

export class HostCallError extends Error {
  readonly code: number;

  constructor(code: number, message: string) {
    super(message);
    this.name = 'HostCallError';
    this.code = code;
  }
}

export type Receipt =
  | { kind: 'loaded' }
  | { kind: 'loadFailed'; stage: LoadFailureStage }
  | { kind: 'unloaded'; hookFailed: boolean };

interface CommandResult<T> {
  success: boolean;
  data: T | null;
  error: { code: number; message: string } | null;
}

export interface ModuleInstanceView {
  readonly runtimeId: string;
  readonly moduleId: string;
  readonly version: string;
  readonly container: HTMLElement;
  // True after a render error was caught by the module boundary once the instance was shown (D055).
  readonly crashed: boolean;
}

export interface InstructionChannel {
  onmessage: (message: HostInstruction) => void;
}

export interface ModuleRuntimeHostDeps {
  invoke?: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  createChannel?: () => InstructionChannel;
  importModule?: (url: string) => Promise<unknown>;
  createObjectUrl?: (source: string) => string;
  revokeObjectUrl?: (url: string) => void;
  document?: Pick<Document, 'createElement'>;
  setTimeout?: (callback: () => void, ms: number) => unknown;
  clearTimeout?: (handle: unknown) => void;
}

type HookOutcome = 'ok' | 'rejected' | 'timeout';

interface HookResult {
  outcome: HookOutcome;
  // Resolved value of the hook; only onBeforeUpdate's is read.
  value?: unknown;
}

const LIFECYCLE_HOOKS = ['onLoad', 'onUnload', 'onBeforeUpdate', 'onAfterUpdate'] as const;
type LifecycleHook = (typeof LIFECYCLE_HOOKS)[number];

interface ModuleLifecycle {
  Component: React.ComponentType;
  onLoad?: () => unknown;
  onUnload?: () => unknown;
  onBeforeUpdate?: () => unknown;
  onAfterUpdate?: (state: unknown) => unknown;
}

interface ModuleInstance extends Omit<ModuleInstanceView, 'crashed'> {
  readonly root: Root;
  readonly lifecycle: ModuleLifecycle;
  disposed: boolean;
  // True once the instance was shown. Only from then on a boundary catch counts as a crash.
  loaded: boolean;
  crashed: boolean;
}

interface ModuleHostApi {
  readonly React: typeof React;
  readonly apiVersion: 1;
  readonly moduleId: string;
  readonly version: string;
  readonly providers: Readonly<{
    call: (providerId: string, modelId: string, request: ProviderChatRequest) => Promise<ProviderChatResponse>;
  }>;
  // D067: added alongside the existing fields; `apiVersion` stays 1.
  readonly ui: ModuleHostUi;
}

function newCallId(): string {
  const random = globalThis.crypto?.randomUUID?.();
  return random ?? Math.random().toString(36).slice(2) + Date.now().toString(36);
}

// D055: the state handed to the new instance is JSON-roundtripped, capped and kept in memory only.
const MAX_UPDATE_STATE_BYTES = 256 * 1024;
const MAX_CRASH_STACK_BYTES = 8 * 1024;
// Total attempts of one crash report and the wait before the second one; doubles after that.
const CRASH_REPORT_ATTEMPTS = 3;
const CRASH_REPORT_BACKOFF_MS = 250;

// Cuts at a character boundary, so the result never exceeds `maxBytes` of UTF-8.
function truncateUtf8(text: string, maxBytes: number): string {
  const bytes = new TextEncoder().encode(text);
  if (bytes.length <= maxBytes) return text;
  let end = maxBytes;
  while (end > 0 && (bytes[end] & 0xc0) === 0x80) end -= 1;
  return new TextDecoder().decode(bytes.subarray(0, end));
}

// Only the name and the stack leave the module. The message may hold user data and is never read.
function describeError(error: unknown): { errorName: string; stack: string } {
  let errorName = 'Error';
  let stack = '';
  try {
    if (typeof error === 'object' && error !== null) {
      const name: unknown = Reflect.get(error, 'name');
      const trace: unknown = Reflect.get(error, 'stack');
      if (typeof name === 'string') errorName = name;
      if (typeof trace === 'string') stack = trace;
    }
  } catch {
    // A hostile getter: report the defaults.
  }
  return { errorName, stack: truncateUtf8(stack, MAX_CRASH_STACK_BYTES) };
}

// Catches render errors of the module component, shows a small fallback and tells the loader.
interface BoundaryProps {
  onFailure: (error: unknown) => void;
  children?: React.ReactNode;
}

class ModuleBoundary extends React.Component<BoundaryProps, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError(): { failed: boolean } {
    return { failed: true };
  }

  componentDidCatch(error: unknown): void {
    console.error('module render failed', error);
    this.props.onFailure(error);
  }

  render(): React.ReactNode {
    if (this.state.failed) {
      return React.createElement('div', { role: 'alert', 'data-module-error': '' }, 'This module failed to render.');
    }
    return this.props.children;
  }
}

// Layout effects run synchronously inside flushSync, so "mounted" is known when flushSync returns.
function MountProbe({ onMounted }: { onMounted: () => void }): null {
  React.useLayoutEffect(() => {
    onMounted();
  }, [onMounted]);
  return null;
}

const CANCELLED: Receipt = { kind: 'loadFailed', stage: 'cancelled' };

export class ModuleRuntimeHost {
  private readonly invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  private readonly createChannel: () => InstructionChannel;
  private readonly importModule: (url: string) => Promise<unknown>;
  private readonly createObjectUrl: (source: string) => string;
  private readonly revokeObjectUrl: (url: string) => void;
  private readonly document: Pick<Document, 'createElement'> | undefined;
  private readonly setTimer: (callback: () => void, ms: number) => unknown;
  private readonly clearTimer: (handle: unknown) => void;

  private attachment: Promise<string | null> | null = null;
  private readonly instances = new Map<string, ModuleInstance>();
  private readonly loading = new Set<string>();
  // runtimeIds that received an unload. A late or duplicate load for one of them is cancelled.
  private readonly tombstones = new Set<string>();
  // runtimeId -> route pushed by the host (D051). The module never sees the value; `host.providers.call`
  // reads it on each call, so a route that was replaced or dropped stops working at once.
  private readonly routes = new Map<string, string>();
  private readonly listeners = new Set<() => void>();
  private snapshot: readonly ModuleInstanceView[] = [];

  constructor(deps: ModuleRuntimeHostDeps = {}) {
    this.invoke = deps.invoke ?? tauriInvoke;
    this.createChannel = deps.createChannel ?? (() => new Channel<HostInstruction>());
    this.importModule = deps.importModule ?? ((url) => import(/* @vite-ignore */ url));
    this.createObjectUrl =
      deps.createObjectUrl ?? ((source) => URL.createObjectURL(new Blob([source], { type: 'text/javascript' })));
    this.revokeObjectUrl = deps.revokeObjectUrl ?? ((url) => URL.revokeObjectURL(url));
    this.document = deps.document;
    this.setTimer = deps.setTimeout ?? ((callback, ms) => window.setTimeout(callback, ms));
    this.clearTimer =
      deps.clearTimeout ??
      ((handle) => {
        if (typeof handle === 'number') window.clearTimeout(handle);
      });
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  getSnapshot = (): readonly ModuleInstanceView[] => this.snapshot;

  // Idempotent and single-flight: concurrent and repeated calls share one attach. A failed attach
  // is forgotten so a later call retries. Never rejects.
  ensureAttached(): Promise<void> {
    return this.beginAttach().then(() => undefined);
  }

  private beginAttach(): Promise<string | null> {
    if (this.attachment === null) {
      const attempt: Promise<string | null> = this.attach().then((attachmentId) => {
        if (attachmentId === null && this.attachment === attempt) this.attachment = null;
        return attachmentId;
      });
      this.attachment = attempt;
    }
    return this.attachment;
  }

  private async attach(): Promise<string | null> {
    try {
      const channel = this.createChannel();
      channel.onmessage = (instruction) => {
        void this.handleInstruction(instruction);
      };
      const result = await this.invoke<CommandResult<{ attachmentId: string }>>('attach_module_host', {
        onInstruction: channel,
      });
      if (!result || !result.success || !result.data) {
        console.warn('module host: attach rejected', result?.error);
        return null;
      }
      return result.data.attachmentId;
    } catch (error) {
      console.warn('module host: attach failed', error);
      return null;
    }
  }

  private async handleInstruction(instruction: HostInstruction): Promise<void> {
    if (instruction.kind === 'route') {
      if (typeof instruction.runtimeId === 'string' && typeof instruction.route === 'string') {
        this.routes.set(instruction.runtimeId, instruction.route);
      }
      return;
    }
    let receipt: Receipt;
    try {
      if (instruction.kind === 'load') {
        receipt = await this.load(instruction);
      } else if (instruction.kind === 'unload') {
        receipt = await this.unload(instruction);
      } else {
        console.warn('module host: ignoring instruction with unknown kind');
        return;
      }
    } catch (error) {
      console.error('module host: instruction handling failed', error);
      receipt =
        instruction.kind === 'load'
          ? { kind: 'loadFailed', stage: 'import' }
          : { kind: 'unloaded', hookFailed: true };
    }
    await this.report(instruction.ticket, receipt);
  }

  private async load(instruction: LoadInstruction): Promise<Receipt> {
    const { runtimeId, moduleId, version, source, hookTimeoutMs, replaces } = instruction;
    if (this.tombstones.has(runtimeId)) return CANCELLED;
    if (this.loading.has(runtimeId) || this.instances.has(runtimeId)) {
      console.warn('module host: duplicate load for a live runtime ignored');
      return CANCELLED;
    }
    this.loading.add(runtimeId);

    let blobUrl: string | null = null;
    // D045: revoking releases the Blob bytes only. The engine keeps the evaluated module record,
    // its closures and global side effects until the page reloads; onUnload is the module's only
    // chance to clean up after itself. Revoking is therefore NOT a memory release.
    const releaseBlob = (): void => {
      if (blobUrl === null) return;
      const url = blobUrl;
      blobUrl = null;
      try {
        this.revokeObjectUrl(url);
      } catch (error) {
        console.warn('module host: revoking blob url failed', error);
      }
    };

    let instance: ModuleInstance | null = null;
    const failed = (stage: LoadFailureStage): Receipt => {
      if (instance !== null) this.dispose(instance);
      return { kind: 'loadFailed', stage };
    };

    let stage: LoadFailureStage = 'import';
    try {
      blobUrl = this.createObjectUrl(source);
      let namespace: unknown;
      let importFailed = false;
      try {
        namespace = await this.importModule(blobUrl);
      } catch (error) {
        importFailed = true;
        console.error('module import failed', error);
      } finally {
        releaseBlob();
      }
      if (this.tombstones.has(runtimeId)) return failed('cancelled');
      if (importFailed) return failed('import');

      stage = 'contract';
      const lifecycle = this.readContract(namespace, runtimeId, moduleId, version);
      if (lifecycle === null) return failed('contract');

      stage = 'render';
      const container = (this.document ?? globalThis.document).createElement('div');
      container.setAttribute('data-module-runtime', runtimeId);
      container.setAttribute('data-module-id', moduleId);
      const created: ModuleInstance = {
        runtimeId,
        moduleId,
        version,
        container,
        root: createRoot(container),
        lifecycle,
        disposed: false,
        loaded: false,
        crashed: false,
      };
      instance = created;
      this.instances.set(runtimeId, created);

      const status = { mounted: false, failed: false };
      try {
        flushSync(() => {
          created.root.render(
            React.createElement(
              ModuleBoundary,
              {
                onFailure: (error) => {
                  if (created.loaded) this.markCrashed(created, error);
                  else status.failed = true;
                },
              },
              React.createElement(MountProbe, {
                onMounted: () => {
                  status.mounted = true;
                },
              }),
              React.createElement(lifecycle.Component),
            ),
          );
        });
      } catch (error) {
        console.error('module render threw', error);
        status.failed = true;
      }
      if (status.failed || !status.mounted) return failed('render');

      stage = 'hook';
      // The replaced instance saves its state right before the new one loads, so the state is as
      // fresh as possible. It is not unloaded here; the host sends its own unload later.
      const handoff = await this.captureUpdateState(replaces, moduleId, hookTimeoutMs);
      const loadHook = await this.runHook(lifecycle.onLoad, hookTimeoutMs);
      if (this.tombstones.has(runtimeId) || created.disposed) return failed('cancelled');
      if (loadHook.outcome === 'timeout') return failed('timeout');
      if (loadHook.outcome === 'rejected') return failed('hook');

      if (handoff !== null) {
        const { state } = handoff;
        const restore = lifecycle.onAfterUpdate;
        const restored = await this.runHook(restore === undefined ? undefined : () => restore(state), hookTimeoutMs);
        if (this.tombstones.has(runtimeId) || created.disposed) return failed('cancelled');
        // A restore that throws or times out fails the whole load (D055); the old instance stays.
        if (restored.outcome !== 'ok') return failed('hook');
      }

      this.show(created);
      return { kind: 'loaded' };
    } catch (error) {
      console.error('module load failed unexpectedly', error);
      return failed(stage);
    } finally {
      releaseBlob();
      this.loading.delete(runtimeId);
    }
  }

  private async unload(instruction: UnloadInstruction): Promise<Receipt> {
    const { runtimeId, hookTimeoutMs } = instruction;
    this.tombstones.add(runtimeId);
    const instance = this.instances.get(runtimeId);
    let hookFailed = false;
    if (instance !== undefined) {
      // Forget first so a duplicate unload during the hook neither reruns it nor sees the instance.
      this.instances.delete(runtimeId);
      this.hide(instance);
      hookFailed = (await this.runHook(instance.lifecycle.onUnload, hookTimeoutMs)).outcome !== 'ok';
      this.dispose(instance);
    }
    return { kind: 'unloaded', hookFailed };
  }

  // D055: asks the replaced instance for its state. Any failure only means "no state": the update
  // goes on without onAfterUpdate. The state is never logged and never sent to the host.
  private async captureUpdateState(
    replaces: string | null | undefined,
    moduleId: string,
    hookTimeoutMs: number,
  ): Promise<{ state: unknown } | null> {
    const previous = replaces ? this.instances.get(replaces) : undefined;
    const hook = previous?.lifecycle.onBeforeUpdate;
    if (previous === undefined || hook === undefined) return null;
    if (!previous.loaded || previous.disposed || previous.moduleId !== moduleId) return null;
    const { outcome, value } = await this.runHook(hook, hookTimeoutMs);
    if (outcome !== 'ok') return null;
    try {
      const json = JSON.stringify(value);
      if (json === undefined || new TextEncoder().encode(json).length > MAX_UPDATE_STATE_BYTES) return null;
      return { state: JSON.parse(json) as unknown };
    } catch {
      console.warn('module host: update state is not serializable');
      return null;
    }
  }

  // The only fields sent are the host-issued attachment and route plus the call's own business
  // fields. Never a module id, runtime id, user, version, digest, path or permission (D051).
  private async callProvider(
    runtimeId: string,
    providerId: string,
    modelId: string,
    request: ProviderChatRequest,
  ): Promise<ProviderChatResponse> {
    const route = this.routes.get(runtimeId);
    const attachmentId = this.attachment === null ? null : await this.attachment;
    if (route === undefined || attachmentId === null) {
      throw new HostCallError(1004004, 'Host permission denied');
    }
    // One id per page call; a transport-level resend of the same call carries the same id, which
    // the host uses to drop the duplicate (D054). It merges nothing and grants nothing.
    const callId = newCallId();
    const result = await this.invoke<CommandResult<ProviderChatResponse>>('module_call_provider', {
      attachmentId,
      route,
      callId,
      providerId,
      modelId,
      request,
    });
    if (!result || !result.success || !result.data) {
      throw new HostCallError(result?.error?.code ?? 1000000, result?.error?.message ?? 'Host call failed');
    }
    return result.data;
  }

  private readContract(namespace: unknown, runtimeId: string, moduleId: string, version: string): ModuleLifecycle | null {
    const factory = typeof namespace === 'object' && namespace !== null ? Reflect.get(namespace, 'default') : undefined;
    if (typeof factory !== 'function') {
      console.error('module contract: default export is not a function');
      return null;
    }
    const host: ModuleHostApi = Object.freeze({
      React,
      apiVersion: 1 as const,
      moduleId,
      version,
      providers: Object.freeze({
        call: (providerId: string, modelId: string, request: ProviderChatRequest) =>
          this.callProvider(runtimeId, providerId, modelId, request),
      }),
      ui: moduleHostUi,
    });
    let produced: unknown;
    try {
      produced = (factory as (host: ModuleHostApi) => unknown)(host);
    } catch (error) {
      console.error('module contract: factory threw', error);
      return null;
    }
    if (typeof produced !== 'object' || produced === null) {
      console.error('module contract: factory did not return an object');
      return null;
    }
    const Component: unknown = Reflect.get(produced, 'Component');
    if (typeof Component !== 'function') {
      console.error('module contract: Component is not a function or class component');
      return null;
    }
    const hooks: Partial<Record<LifecycleHook, (...args: unknown[]) => unknown>> = {};
    for (const name of LIFECYCLE_HOOKS) {
      const hook: unknown = Reflect.get(produced, name);
      if (hook === undefined) continue;
      if (typeof hook !== 'function') {
        console.error('module contract: lifecycle hooks must be functions when present');
        return null;
      }
      hooks[name] = (...args) => Reflect.apply(hook, produced, args) as unknown;
    }
    return { Component: Component as React.ComponentType, ...hooks };
  }

  // Races the hook against a timer so a promise that never settles cannot hang the host.
  // Never rejects; the timer is cleared as soon as the hook settles.
  private runHook(hook: (() => unknown) | undefined, ms: number): Promise<HookResult> {
    if (hook === undefined) return Promise.resolve({ outcome: 'ok' });
    return new Promise<HookResult>((resolve) => {
      let settled = false;
      let handle: unknown;
      const finish = (outcome: HookOutcome, value?: unknown): void => {
        if (settled) return;
        settled = true;
        this.clearTimer(handle);
        resolve({ outcome, value });
      };
      handle = this.setTimer(() => finish('timeout'), ms);
      try {
        Promise.resolve(hook()).then(
          (value) => finish('ok', value),
          (error: unknown) => {
            console.error('module hook rejected', error);
            finish('rejected');
          },
        );
      } catch (error) {
        console.error('module hook threw', error);
        finish('rejected');
      }
    });
  }

  // Idempotent and never throws, so a failing step cannot skip the rest of the cleanup or the receipt.
  private dispose(instance: ModuleInstance): void {
    if (instance.disposed) return;
    instance.disposed = true;
    if (this.instances.get(instance.runtimeId) === instance) this.instances.delete(instance.runtimeId);
    this.routes.delete(instance.runtimeId);
    this.hide(instance);
    try {
      instance.root.unmount();
    } catch (error) {
      console.error('module root unmount failed', error);
    }
    try {
      instance.container.remove();
    } catch (error) {
      console.error('module container removal failed', error);
    }
  }

  private view(instance: ModuleInstance): ModuleInstanceView {
    return {
      runtimeId: instance.runtimeId,
      moduleId: instance.moduleId,
      version: instance.version,
      container: instance.container,
      crashed: instance.crashed,
    };
  }

  private show(instance: ModuleInstance): void {
    instance.loaded = true;
    this.snapshot = [...this.snapshot, this.view(instance)];
    this.emit();
  }

  // A render error caught by the boundary after the instance was shown (D055). At most once per
  // instance. The route is read now, before an unload can drop it.
  private markCrashed(instance: ModuleInstance, error: unknown): void {
    if (instance.crashed || instance.disposed) return;
    instance.crashed = true;
    const view = this.view(instance);
    this.snapshot = this.snapshot.map((current) => (current.container === instance.container ? view : current));
    this.emit();
    void this.reportCrash(this.routes.get(instance.runtimeId), error);
  }

  // A failed report (invoke throws or the host answers `success: false`) is retried, at most
  // CRASH_REPORT_ATTEMPTS in total (D059). Every attempt carries the same callId, which the host
  // dedups (D054), so a report whose answer was lost is not recorded twice. Other lifecycle
  // receipts are only logged and never retried. A missing route or attachment skips the report
  // silently.
  private async reportCrash(route: string | undefined, error: unknown): Promise<void> {
    if (route === undefined) return;
    const callId = newCallId();
    const { errorName, stack } = describeError(error);
    for (let attempt = 1; attempt <= CRASH_REPORT_ATTEMPTS; attempt += 1) {
      try {
        const attachmentId = this.attachment === null ? null : await this.attachment;
        if (attachmentId === null) return;
        const result = await this.invoke<CommandResult<{ recorded: boolean }>>('report_module_crash', {
          attachmentId,
          route,
          callId,
          source: 'render',
          errorName,
          stack,
        });
        if (result && result.success) return;
        console.warn('module host: crash report rejected', result?.error);
      } catch (reportError) {
        console.warn('module host: crash report failed', reportError);
      }
      if (attempt < CRASH_REPORT_ATTEMPTS) {
        // lib is ES2020 (no Promise.withResolvers), so the executor form stays.
        await new Promise<void>((resolve) =>
          this.setTimer(resolve, CRASH_REPORT_BACKOFF_MS * 2 ** (attempt - 1)),
        );
      }
    }
  }

  private hide(instance: ModuleInstance): void {
    const next = this.snapshot.filter((view) => view.container !== instance.container);
    if (next.length === this.snapshot.length) return;
    this.snapshot = next;
    this.emit();
  }

  private emit(): void {
    for (const listener of [...this.listeners]) listener();
  }

  // A failed report is only logged and never retried: the host times out by itself (D044).
  private async report(ticket: string, receipt: Receipt): Promise<void> {
    try {
      const attachmentId = this.attachment === null ? null : await this.attachment;
      if (attachmentId === null) {
        console.warn('module host: not attached, receipt dropped');
        return;
      }
      const result = await this.invoke<CommandResult<{ disposition: string }>>('report_module_host', {
        attachmentId,
        ticket,
        receipt,
      });
      if (!result || !result.success) {
        console.warn('module host: receipt rejected', result?.error);
        return;
      }
      const disposition = result.data?.disposition;
      if (disposition !== 'applied') console.warn('module host: receipt not applied:', disposition);
    } catch (error) {
      console.warn('module host: receipt report failed', error);
    }
  }
}

export const moduleRuntimeHost = new ModuleRuntimeHost();
