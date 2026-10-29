export default function createModule(host) {
  const h = host.React.createElement;
  return { Component: () => h('label', { style: { display: 'block' } }, 'x') };
}
