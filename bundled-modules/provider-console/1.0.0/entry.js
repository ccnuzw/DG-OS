// Provider Console: a first-party module shipped with the host (D052).
// Single-file ES module, no build step: this file is the artifact the pin covers.
// Contract (D042): default export `createModule(host)` returning `{ Component, onLoad?, onUnload? }`.
export default function createModule(host) {
  const React = host.React;
  const h = React.createElement;

  function Component() {
    const [providerId, setProviderId] = React.useState('');
    const [modelId, setModelId] = React.useState('');
    const [prompt, setPrompt] = React.useState('');
    const [state, setState] = React.useState({ kind: 'idle' });

    const send = async () => {
      setState({ kind: 'pending' });
      try {
        const response = await host.providers.call(providerId.trim(), modelId.trim(), {
          messages: [{ role: 'user', content: prompt }],
          stream: false,
        });
        setState({ kind: 'done', content: response.content });
      } catch (error) {
        setState({ kind: 'error', code: error && error.code });
      }
    };

    const ready = providerId.trim() !== '' && modelId.trim() !== '' && prompt.trim() !== '' && state.kind !== 'pending';
    const field = (label, value, set, extra = {}) =>
      h('label', { style: { display: 'block', marginBottom: 8 } }, label, h('input', { value, onChange: (e) => set(e.target.value), ...extra }));

    return h(
      'section',
      { 'data-testid': 'provider-console' },
      h('h3', null, 'Provider Console'),
      field('Provider id ', providerId, setProviderId),
      field('Model id ', modelId, setModelId),
      h('textarea', { value: prompt, onChange: (e) => setPrompt(e.target.value), rows: 4, 'aria-label': 'Prompt' }),
      h('div', null, h('button', { disabled: !ready, onClick: send }, 'Send')),
      state.kind === 'pending' && h('p', { role: 'status' }, 'Waiting for the provider...'),
      state.kind === 'done' && h('pre', { 'data-testid': 'provider-console-reply' }, state.content),
      state.kind === 'error' && h('p', { role: 'alert' }, 'The call was refused or failed (code ' + String(state.code) + ').'),
    );
  }

  return { Component };
}
