const probe = (globalThis.__dgosModuleProbe ??= { evaluated: 0, events: [] }); probe.evaluated += 1;
// The harness steers the hooks through this shared object (modes are read when a hook runs).
const control = (globalThis.__dgosHooks ??= {});
export default function createModule(host) {
  const React = host.React;
  function Component() {
    return React.createElement('div', { 'data-testid': 'hooks-update' }, 'version:' + host.version);
  }
  return {
    Component,
    onLoad() { probe.events.push('load:' + host.version); },
    onUnload() { probe.events.push('unload:' + host.version); },
    onBeforeUpdate() {
      probe.events.push('before:' + host.version);
      switch (control.before) {
        case 'throw': throw new Error('fixture onBeforeUpdate failure');
        case 'reject': return Promise.reject(new Error('fixture onBeforeUpdate rejection'));
        case 'hang': return new Promise(() => {});
        case 'function': return () => 1;
        case 'huge': return { text: 'x'.repeat(256 * 1024 + 1) };
        case 'max': return 'x'.repeat(256 * 1024 - 2);
        case 'circular': { const loop = {}; loop.self = loop; return loop; }
        default: return { count: 3, list: [1, 2], when: new Date(0), skipped: undefined };
      }
    },
    onAfterUpdate(state) {
      probe.events.push('after:' + host.version);
      control.restored = state;
      switch (control.after) {
        case 'throw': throw new Error('fixture onAfterUpdate failure');
        case 'reject': return Promise.reject(new Error('fixture onAfterUpdate rejection'));
        case 'hang': return new Promise(() => {});
        default: return undefined;
      }
    },
  };
}
