const probe = (globalThis.__dgosModuleProbe ??= { evaluated: 0, events: [] }); probe.evaluated += 1;
export default function createModule(host) {
  const React = host.React;
  function Component() {
    return React.createElement('button', { 'data-testid': 'fixture-counter' }, 'count:0');
  }
  return {
    Component,
    onLoad() { throw new Error('fixture onLoad failure'); },
    onUnload() { probe.events.push('unload:' + host.moduleId); },
  };
}
