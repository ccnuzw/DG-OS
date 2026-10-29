import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export interface ModuleProbe {
  evaluated: number;
  events: string[];
}

const fixtureDir = join(dirname(fileURLToPath(import.meta.url)), '..', 'fixtures', 'modules');

export function readFixture(name: string): string {
  return readFileSync(join(fixtureDir, name), 'utf8');
}

export function moduleProbe(): ModuleProbe {
  const scope = globalThis as typeof globalThis & { __dgosModuleProbe?: ModuleProbe };
  return (scope.__dgosModuleProbe ??= { evaluated: 0, events: [] });
}

export function resetModuleProbe(): void {
  const probe = moduleProbe();
  probe.evaluated = 0;
  probe.events.length = 0;
  const scope = globalThis as typeof globalThis & { __dgosHooks?: HooksControl };
  delete scope.__dgosHooks;
}

// Steering and observation channel of the hooks-* fixtures; cleared by resetModuleProbe.
export interface HooksControl {
  before?: 'throw' | 'reject' | 'hang' | 'function' | 'huge' | 'max' | 'circular';
  after?: 'throw' | 'reject' | 'hang';
  restored?: unknown;
  seenReact?: unknown;
}

export function hooksControl(): HooksControl {
  const scope = globalThis as typeof globalThis & { __dgosHooks?: HooksControl };
  return (scope.__dgosHooks ??= {});
}

// Test double for `import(blobUrl)`: jsdom/Node cannot import blob: URLs, so the fixture source is
// turned into a module namespace by rewriting the single `export default function createModule`
// into `return function createModule` and evaluating it with `new Function`. A fixture that is not
// valid JS throws here, like a rejected import(). A bare specifier import (`from 'react'`) is
// rejected explicitly, as a Blob entry cannot resolve it (D055). The real import(blobUrl) path is
// covered by the WebView smoke test, not here.
export function evaluateFixtureSource(source: string): { default: unknown } {
  if (/^\s*import\s[^;]*?from\s+['"][^./'"][^'"]*['"]/m.test(source)) {
    throw new TypeError('Failed to resolve module specifier');
  }
  const body = source.replace(/^export default /m, 'return ');
  const value: unknown = new Function(body)();
  return { default: value };
}
