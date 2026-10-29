const probe = (globalThis.__dgosModuleProbe ??= { evaluated: 0, events: [] }); probe.evaluated += 1;
export default function createModule(host) {
  const React = host.React;
  function Component() {
    const [n, setN] = React.useState(0);
    return React.createElement('button', { 'data-testid': 'fixture-counter', onClick: () => setN(n + 1) }, 'count:' + n);
  }
  return {
    Component,
    onLoad() { probe.events.push('load:' + host.moduleId); },
    onUnload() { probe.events.push('unload:' + host.moduleId); throw new Error('fixture onUnload failure'); },
  };
}
