const probe = (globalThis.__dgosModuleProbe ??= { evaluated: 0, events: [] }); probe.evaluated += 1;
export default function createModule(host) {
  (globalThis.__dgosHooks ??= {}).seenReact = host.React;
  function Component() {
    return host.React.createElement('div', null, 'react-identity');
  }
  return { Component };
}
