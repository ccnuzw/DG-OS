const probe = (globalThis.__dgosModuleProbe ??= { evaluated: 0, events: [] }); probe.evaluated += 1;
export default function createModule(host) {
  function Component() {
    throw new Error('fixture render failure');
  }
  return {
    Component,
    onLoad() { probe.events.push('load:' + host.moduleId); },
    onUnload() { probe.events.push('unload:' + host.moduleId); },
  };
}
