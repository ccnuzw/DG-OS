// Provider Console: a first-party module shipped with the host (D052).
// Single-file ES module, no build step: this file is the artifact the pin covers.
// Contract (D042): default export `createModule(host)` returning `{ Component, onLoad?, onUnload? }`.
// UI comes from `host.ui` (D067): shared components, no inline styles, no own controls.
export default function createModule(host) {
  const React = host.React;
  const h = React.createElement;
  const { Button, Input, Select, Loading, Stack } = host.ui;
  const catalogs = {
    'zh-CN': { title: '模型调试台', provider: '服务', model: '模型', prompt: '提示词', send: '发送', waiting: '正在等待 Provider…', failed: '调用被拒绝或失败（错误码 ', end: '）。', help: '选择当前账户已配置的服务和模型。目录可见不代表已获调用授权。', result: '调用结果', empty: '选择服务、模型并填写提示词后发送。', pending: '请求进行中，请等待结果。', refresh: '刷新目录', catalogFailed: '服务目录读取失败，请重试。', noProviders: '没有可用服务，请在设置中配置。', choose: '请选择' },
    'en-US': { title: 'Model console', provider: 'Provider', model: 'Model', prompt: 'Prompt', send: 'Send', waiting: 'Waiting for the provider...', failed: 'The call was refused or failed (code ', end: ').', help: 'Choose a configured provider and model. Catalog visibility does not grant permission to call.', result: 'Response', empty: 'Choose a provider and model and enter a prompt.', pending: 'Request in progress. Waiting for the result.', refresh: 'Refresh catalog', catalogFailed: 'Could not read the provider catalog. Try again.', noProviders: 'No providers available. Configure one in settings.', choose: 'Choose' },
  };
  const documents = new Map();

  function Component({ documentKey = 'default' }) {
    const locale = React.useSyncExternalStore(host.i18n.subscribe, () => host.i18n.locale);
    const text = catalogs[locale];
    const saved = documents.get(documentKey) || {};
    const [providerId, setProviderId] = React.useState(saved.providerId || '');
    const [modelId, setModelId] = React.useState(saved.modelId || '');
    const [prompt, setPrompt] = React.useState(saved.prompt || '');
    const [state, setState] = React.useState({ kind: 'idle' });
    const [catalog, setCatalog] = React.useState([]);
    const [catalogState, setCatalogState] = React.useState('loading');
    const [refresh, setRefresh] = React.useState(0);
    const alive = React.useRef(true);
    React.useEffect(() => host.providers.subscribe(() => setRefresh(value => value + 1)), []);
    React.useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
    React.useEffect(() => {
      let current = true;
      const abort = new AbortController();
      setCatalogState('loading');
      host.providers.list({ signal: abort.signal }).then(entries => {
        if (!current) return;
        setCatalog(entries);
        setCatalogState('ready');
      }).catch(() => { if (current) setCatalogState('error'); });
      return () => { current = false; abort.abort(); };
    }, [refresh]);
    const models = catalog.find(entry => entry.providerId === providerId)?.models.filter(model => model.kind === 'text-generation' || model.kind === 'chat') || [];
    documents.set(documentKey, { providerId, modelId, prompt, pending: state.kind === 'pending' });

    const send = async () => {
      setState({ kind: 'pending' });
      try {
        const response = await host.providers.call(providerId.trim(), modelId.trim(), {
          messages: [{ role: 'user', content: prompt }],
          stream: false,
        });
        if (alive.current) setState({ kind: 'done', content: response.content });
      } catch (error) {
        if (alive.current) setState({ kind: 'error', code: error && error.code });
      }
    };

    const ready = catalogState === 'ready' && catalog.some(entry => entry.providerId === providerId) && models.some(model => model.modelId === modelId) && prompt.trim() !== '' && state.kind !== 'pending';

    return h('main', { 'data-testid': 'provider-console', className: 'provider-console' },
      h('header', { className: 'provider-console-header' }, h('h1', null, text.title), h('p', null, text.help)),
      h('div', { className: 'provider-console-columns' },
        h('section', { className: 'provider-console-config', 'aria-label': text.title },
          h(Stack, { gap: 3 },
            h(Select, { label: text.provider, placeholder: text.choose, options: catalog.map(entry => ({ value: entry.providerId, label: entry.displayName })), value: providerId, disabled: state.kind === 'pending' || catalogState !== 'ready', onChange: value => { setProviderId(value); setModelId(''); } }),
            h(Select, { label: text.model, placeholder: text.choose, options: models.map(model => ({ value: model.modelId, label: model.displayName })), value: modelId, disabled: state.kind === 'pending' || catalogState !== 'ready', onChange: setModelId }),
            catalogState === 'loading' && h(Loading, { label: text.refresh }),
            catalogState === 'error' && h('p', { role: 'alert' }, text.catalogFailed),
            catalogState === 'ready' && !catalog.length && h('p', null, text.noProviders),
            h(Button, { variant: 'secondary', disabled: catalogState === 'loading' || state.kind === 'pending', onClick: () => setRefresh(value => value + 1) }, text.refresh),
            h(Input, { multiline: true, className: 'type-reading', rows: 7, label: text.prompt, value: prompt, disabled: state.kind === 'pending', onChange: (e) => setPrompt(e.target.value) }),
            h(Button, { disabled: !ready, loading: state.kind === 'pending', onClick: send }, text.send),
          ),
        ),
        h('section', { className: 'provider-console-result', 'aria-label': text.result, 'aria-busy': state.kind === 'pending' },
          h('h2', null, text.result),
          state.kind === 'idle' && h('p', { className: 'text-sm text-ink-secondary' }, text.empty),
          state.kind === 'pending' && h(Loading, { label: text.waiting }),
          state.kind === 'done' && h('pre', { 'data-testid': 'provider-console-reply', className: 'type-reading' }, state.content),
          state.kind === 'error' && h('p', { role: 'alert', className: 'text-sm text-status-error-text' }, text.failed + String(state.code) + text.end),
        ),
      ),
    );
  }

  return {
    Component,
    openView({ viewId, documentId }) {
      const documentKey = documentId || viewId;
      return {
        Component: () => h(Component, { documentKey }),
        beforeClose: () => ({ dirty: !!documents.get(documentKey)?.prompt, blocked: !!documents.get(documentKey)?.pending }),
        dispose: () => documents.delete(documentKey),
      };
    },
    onBeforeUpdate: () => Array.from(documents.entries()).map(([key, value]) => [key, { ...value, pending: false }]),
    onAfterUpdate: state => { for (const [key, value] of state || []) documents.set(key, value); },
  };
}
