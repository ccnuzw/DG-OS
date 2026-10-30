// Provider Console: a first-party module shipped with the host (D052).
// Single-file ES module, no build step: this file is the artifact the pin covers.
// Contract (D042): default export `createModule(host)` returning `{ Component, onLoad?, onUnload? }`.
// UI comes from `host.ui` (D067): shared components, no inline styles, no own controls.
export default function createModule(host) {
  const React = host.React;
  const h = React.createElement;
  const { Button, Card, Input, Loading, Stack } = host.ui;

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

    return h(
      Card,
      { 'data-testid': 'provider-console' },
      h(Card.Header, null, h('h3', null, 'Provider Console')),
      h(
        Card.Body,
        null,
        h(
          Stack,
          { gap: 3 },
          h(Input, { label: 'Provider id', value: providerId, onChange: (e) => setProviderId(e.target.value) }),
          h(Input, { label: 'Model id', value: modelId, onChange: (e) => setModelId(e.target.value) }),
          h(Input, { multiline: true, rows: 4, 'aria-label': 'Prompt', value: prompt, onChange: (e) => setPrompt(e.target.value) }),
          h(Stack, { gap: 2, direction: 'horizontal' }, h(Button, { disabled: !ready, onClick: send }, 'Send')),
          state.kind === 'pending' && h(Loading, { label: 'Waiting for the provider...' }),
          state.kind === 'done' && h('pre', { 'data-testid': 'provider-console-reply' }, state.content),
          state.kind === 'error' && h('p', { role: 'alert' }, 'The call was refused or failed (code ' + String(state.code) + ').'),
        ),
      ),
    );
  }

  return { Component };
}
