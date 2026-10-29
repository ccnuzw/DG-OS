const probe = (globalThis.__dgosModuleProbe ??= { evaluated: 0, events: [] }); probe.evaluated += 1;
export default function createModule(host) {
  const React = host.React;
  function Component() {
    const [crash, setCrash] = React.useState(false);
    if (crash) {
      // The stack is longer than 8 KiB and carries no message text; the message is secret on purpose.
      const error = new Error('fixture secret message');
      error.name = 'FixtureCrash';
      error.stack = 'FixtureCrash\n' + '    at frame (module:1:1)\n'.repeat(1000);
      throw error;
    }
    return React.createElement('button', { 'data-testid': 'fixture-crash', onClick: () => setCrash(true) }, 'crash');
  }
  return {
    Component,
    onLoad() { probe.events.push('load:' + host.moduleId); },
    onUnload() { probe.events.push('unload:' + host.moduleId); },
  };
}
