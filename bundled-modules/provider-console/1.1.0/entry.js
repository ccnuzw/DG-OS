// Provider Console 1.1: public host.ai consumer. Drafts and output belong to this module.
// A document's run survives hiding/unmounting; explicit disposal cancels only that document.
export default function createModule(host) {
  const React = host.React;
  const h = React.createElement;
  const { Button, Input, Select, Checkbox, Loading, Stack } = host.ui;
  const copy = {
    'en-US': {
      title: 'Model console', help: 'Run a configured model with selected skills and tools. Each tool call requires host approval. Permissions are managed in module settings.',
      provider: 'Provider', model: 'Model', prompt: 'Prompt', choose: 'Choose', send: 'Send', cancel: 'Cancel run', cancelling: 'Cancellation requested. Waiting for the final state.',
      refresh: 'Refresh selections', loading: 'Reading catalog…', catalogFailed: 'Catalog unavailable or permission denied. Check module permissions and refresh.', noProviders: 'No providers available. Configure one in settings.',
      skills: 'Content skills', tools: 'Tools', optional: 'Optional. Nothing is selected automatically.', emptyCatalog: 'No available entries.', stale: 'A selected revision is no longer available. Refresh selections and choose again.',
      result: 'Response', empty: 'Choose a provider and model and enter a prompt.', modelPhase: 'Generating…', awaiting_confirmation: 'Waiting for host approval. Review the host dialog for this tool call.', tool: 'Running approved tool…',
      succeeded: 'Completed', failed: 'Failed', cancelled: 'Cancelled', interrupted: 'Interrupted', running: 'Running', error: 'The host refused or could not finish the request. Code: ',
      unknown: 'The external result is unknown. Check status before deciding whether to send a new request.', auditWarning: 'The host could not confirm the audit write. Do not assume this request can safely be repeated.',
      reasoning: 'Reasoning', history: 'This module’s runs', historyHelp: 'Status only; history does not restore prompts, output or approvals. Drafts and output are kept in this window’s module memory, not guaranteed after reload.',
      refreshHistory: 'Refresh history', more: 'Load earlier runs', check: 'Check status', checking: 'Checking…', historyFailed: 'Could not read this module’s run status.', missing: 'Run not found in this module’s history.', noRuns: 'No recorded runs.', schema: 'Input schema', pin: 'Content hash', revision: 'Provider revision',
    },
    'zh-CN': {
      title: '模型调试台', help: '使用已配置模型及所选技能、工具生成内容。每次工具调用均需宿主审批；请在模块设置中管理权限。',
      provider: '服务', model: '模型', prompt: '提示词', choose: '请选择', send: '发送', cancel: '取消运行', cancelling: '已请求取消，正在等待最终状态。',
      refresh: '刷新选择目录', loading: '正在读取目录…', catalogFailed: '目录不可用或权限被拒绝。请检查模块权限后刷新。', noProviders: '没有可用服务，请在设置中配置。',
      skills: '内容技能', tools: '工具', optional: '可选，不会自动选择。', emptyCatalog: '没有可用条目。', stale: '所选修订已不可用，请刷新目录并重新选择。',
      result: '调用结果', empty: '选择服务、模型并填写提示词后发送。', modelPhase: '正在生成…', awaiting_confirmation: '正在等待宿主审批，请在宿主弹框中核对本次工具调用。', tool: '正在执行已批准工具…',
      succeeded: '已完成', failed: '失败', cancelled: '已取消', interrupted: '已中断', running: '运行中', error: '宿主拒绝调用或未能完成请求，错误码：',
      unknown: '外部结果未知。请先核实状态，再决定是否重新发送。', auditWarning: '宿主未能确认审计写入，不能据此认定可以安全重试。',
      reasoning: '思考过程', history: '本模块的运行记录', historyHelp: '历史仅核实状态，不恢复提示词、输出或审批。草稿和输出保存在当前窗口模块内存中，重载后不保证保留。',
      refreshHistory: '刷新记录', more: '加载更早记录', check: '核实状态', checking: '正在核实…', historyFailed: '无法读取本模块的运行状态。', missing: '本模块历史中未找到此运行。', noRuns: '暂无运行记录。', schema: '输入结构', pin: '内容摘要', revision: '服务修订',
    },
  };
  const documents = new Map();
  const listeners = new Set();
  const controllers = new Map();
  const keyOfSkill = entry => JSON.stringify([entry.skill_id, entry.version, entry.content_hash]);
  const keyOfTool = entry => JSON.stringify([entry.server_id, entry.tool_name, entry.schema_hash]);
  const skillRef = ({ skill_id, version, content_hash }) => ({ skill_id, version, content_hash });
  const toolRef = ({ server_id, tool_name, schema_hash }) => ({ server_id, tool_name, schema_hash });
  const initial = value => ({ providerId: '', modelId: '', providerRevision: null, prompt: '', skills: [], tools: [], content: '', reasoning: '', status: 'idle', pending: false, runId: null, unknown: false, auditWarning: false, code: null, phase: 'model', ...value });
  const ensure = key => { if (!documents.has(key)) documents.set(key, initial()); return documents.get(key); };
  const update = (key, patch) => {
    if (!documents.has(key)) return;
    documents.set(key, { ...documents.get(key), ...patch });
    for (const listener of listeners) listener();
  };
  const subscribe = listener => { listeners.add(listener); return () => listeners.delete(listener); };
  const dispose = key => { controllers.get(key)?.abort(); controllers.delete(key); documents.delete(key); };

  function Component({ documentKey = 'default' }) {
    ensure(documentKey);
    const locale = React.useSyncExternalStore(host.i18n.subscribe, () => host.i18n.locale);
    const text = copy[locale];
    const draft = React.useSyncExternalStore(subscribe, () => ensure(documentKey));
    const [catalog, setCatalog] = React.useState({ providers: [], skills: [], tools: [] });
    const [catalogState, setCatalogState] = React.useState({ providers: 'loading', skills: 'loading', tools: 'loading' });
    const [refresh, setRefresh] = React.useState(0);
    const [history, setHistory] = React.useState({ items: [], next_before: null });
    const [historyState, setHistoryState] = React.useState('idle');
    const [checking, setChecking] = React.useState(null);
    const [lookupError, setLookupError] = React.useState(null);
    const alive = React.useRef(false);
    const historyBusy = React.useRef(false);
    React.useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
    React.useEffect(() => host.providers.subscribe(() => setRefresh(value => value + 1)), []);
    React.useEffect(() => {
      let current = true;
      const abort = new AbortController();
      for (const [kind, method] of [['providers', 'listProviders'], ['skills', 'listSkills'], ['tools', 'listTools']]) {
        setCatalogState(value => ({ ...value, [kind]: 'loading' }));
        host.ai[method]({ signal: abort.signal }).then(entries => {
          if (!current) return;
          setCatalog(value => ({ ...value, [kind]: entries }));
          setCatalogState(value => ({ ...value, [kind]: 'ready' }));
        }).catch(() => {
          if (!current) return;
          setCatalog(value => ({ ...value, [kind]: [] }));
          setCatalogState(value => ({ ...value, [kind]: 'error' }));
        });
      }
      return () => { current = false; abort.abort(); };
    }, [refresh]);

    const readHistory = async (earlier = false) => {
      if (historyBusy.current) return;
      historyBusy.current = true;
      setHistoryState('loading');
      try {
        const page = await host.ai.listRuns({ limit: 20, before: earlier ? history.next_before : null });
        if (alive.current) {
          setHistory(previous => ({ items: earlier ? [...previous.items, ...page.items.filter(item => !previous.items.some(old => old.run_id === item.run_id))] : page.items, next_before: page.next_before }));
          setHistoryState('ready');
        }
      } catch { if (alive.current) setHistoryState('error'); }
      finally { historyBusy.current = false; }
    };
    React.useEffect(() => { void readHistory(); }, []);
    const checkRun = async runId => {
      if (checking) return;
      setChecking(runId);
      setLookupError(null);
      try {
        const summary = await host.ai.getRun(runId);
        if (!alive.current) return;
        if (!summary) { setLookupError('missing'); return; }
        setHistory(previous => ({ ...previous, items: previous.items.map(item => item.run_id === runId ? summary : item) }));
        if (documents.get(documentKey)?.runId === runId) {
          update(documentKey, { unknown: summary.result_unknown, ...(documents.get(documentKey).pending ? {} : { status: summary.status, code: summary.error?.code ?? null }) });
        }
      } catch { if (alive.current) setLookupError('historyFailed'); }
      finally { if (alive.current) setChecking(null); }
    };
    const provider = catalog.providers.find(entry => entry.provider_id === draft.providerId);
    const models = provider?.models.filter(model => model.kind === 'text-generation' || model.kind === 'chat') || [];
    const stale = (draft.providerId && provider?.config_revision !== draft.providerRevision)
      || draft.skills.some(pin => !catalog.skills.some(entry => keyOfSkill(entry) === keyOfSkill(pin)))
      || draft.tools.some(pin => !catalog.tools.some(entry => keyOfTool(entry) === keyOfTool(pin)));
    const ready = !draft.pending && catalogState.providers === 'ready' && provider && !stale
      && models.some(model => model.model_id === draft.modelId) && draft.prompt.trim() !== '';
    const send = async () => {
      if (!ready || documents.get(documentKey)?.pending) return;
      const abort = new AbortController();
      controllers.set(documentKey, abort);
      update(documentKey, { pending: true, status: 'running', phase: 'model', content: '', reasoning: '', runId: null, unknown: false, auditWarning: false, code: null, cancelling: false });
      const active = () => controllers.get(documentKey) === abort && documents.has(documentKey);
      let terminalSeen = false;
      const settle = terminal => {
        if (!active() || terminalSeen) return;
        terminalSeen = true;
        update(documentKey, { status: terminal.status, content: terminal.content ?? documents.get(documentKey).content, unknown: terminal.result_unknown, auditWarning: terminal.audit_warning, code: terminal.error?.code ?? null });
      };
      try {
        const terminal = await host.ai.run({ provider_id: draft.providerId, model_id: draft.modelId, provider_revision: draft.providerRevision,
          prompt: draft.prompt, skill_refs: draft.skills.map(skillRef), tools: draft.tools.map(toolRef), generation: {} }, event => {
          if (!active() || terminalSeen) return;
          if (event.kind === 'started') update(documentKey, { runId: event.run_id });
          if (event.kind === 'delta') update(documentKey, { content: documents.get(documentKey).content + event.text });
          if (event.kind === 'reasoning_delta') update(documentKey, { reasoning: documents.get(documentKey).reasoning + event.text });
          if (event.kind === 'phase') update(documentKey, { phase: event.phase });
          if (event.kind === 'terminal') settle(event.terminal);
        }, { signal: abort.signal });
        settle(terminal);
      } catch (error) {
        if (active() && !terminalSeen) update(documentKey, { status: 'failed', code: error?.code ?? null, unknown: !!documents.get(documentKey).runId || error?.code == null || error.code === 1000000 });
      } finally {
        if (active()) { update(documentKey, { pending: false }); controllers.delete(documentKey); }
        if (alive.current) void readHistory();
      }
    };
    const cancel = async () => {
      if (!draft.pending || draft.cancelling) return;
      update(documentKey, { cancelling: true });
      try {
        if (draft.runId) {
          await host.ai.cancel(draft.runId);
          // An acknowledgement does not settle the original run or release the close block.
        } else controllers.get(documentKey)?.abort();
      } catch (error) { update(documentKey, { cancelling: false, code: error?.code ?? null }); }
    };
    const reset = () => {
      update(documentKey, { providerId: '', providerRevision: null, modelId: '', skills: [], tools: [] });
      setRefresh(value => value + 1);
    };
    const toggle = (kind, entry, checked) => {
      const key = kind === 'skills' ? keyOfSkill : keyOfTool;
      const ref = kind === 'skills' ? skillRef : toolRef;
      update(documentKey, { [kind]: checked ? [...draft[kind], ref(entry)] : draft[kind].filter(pin => key(pin) !== key(entry)) });
    };
    const choices = kind => h('section', { className: 'provider-console-choices', 'aria-label': text[kind] },
      h('h2', null, text[kind]), h('p', null, text.optional),
      catalogState[kind] === 'loading' && h(Loading, { label: text.loading }),
      catalogState[kind] === 'error' && h('p', { role: 'alert' }, text.catalogFailed),
      catalogState[kind] === 'ready' && !catalog[kind].length && h('p', null, text.emptyCatalog),
      catalog[kind].map(entry => {
        const skill = kind === 'skills';
        const key = skill ? keyOfSkill : keyOfTool;
        const label = skill ? `${entry.name} (${entry.version})` : `${entry.server_name}: ${entry.tool_name}`;
        return h('div', { key: key(entry), className: 'provider-console-choice' },
          h(Checkbox, { label, checked: draft[kind].some(pin => key(pin) === key(entry)), disabled: draft.pending || catalogState[kind] !== 'ready', onChange: checked => toggle(kind, entry, checked) }),
          !skill && entry.description && h('p', null, entry.description),
          h('details', null, h('summary', null, skill ? text.pin : text.schema),
            h('pre', null, skill ? entry.content_hash : JSON.stringify(entry.input_schema, null, 2)),
            !skill && h('code', null, entry.schema_hash)));
      }));

    return h('main', { 'data-testid': 'provider-console', className: 'provider-console' },
      h('header', { className: 'provider-console-header' }, h('h1', null, text.title), h('p', null, text.help)),
      h('div', { className: 'provider-console-columns' },
        h('section', { className: 'provider-console-config', 'aria-label': text.title }, h(Stack, { gap: 3 },
          h(Select, { label: text.provider, placeholder: text.choose, options: catalog.providers.map(entry => ({ value: entry.provider_id, label: entry.display_name })), value: draft.providerId,
            disabled: draft.pending || catalogState.providers !== 'ready', onChange: value => update(documentKey, { providerId: value, providerRevision: catalog.providers.find(entry => entry.provider_id === value)?.config_revision ?? null, modelId: '' }) }),
          h(Select, { label: text.model, placeholder: text.choose, options: models.map(model => ({ value: model.model_id, label: model.display_name })), value: draft.modelId, disabled: draft.pending || catalogState.providers !== 'ready', onChange: value => update(documentKey, { modelId: value }) }),
          provider && h('p', { className: 'provider-console-note' }, `${text.revision}: ${draft.providerRevision}`),
          catalogState.providers === 'loading' && h(Loading, { label: text.loading }),
          catalogState.providers === 'error' && h('p', { role: 'alert' }, text.catalogFailed),
          catalogState.providers === 'ready' && !catalog.providers.length && h('p', null, text.noProviders),
          stale && h('p', { role: 'alert' }, text.stale),
          h(Button, { variant: 'secondary', disabled: draft.pending || Object.values(catalogState).includes('loading'), onClick: reset }, text.refresh),
          choices('skills'), choices('tools'),
          h(Input, { multiline: true, rows: 7, className: 'type-reading', label: text.prompt, value: draft.prompt, disabled: draft.pending, onChange: event => update(documentKey, { prompt: event.target.value }) }),
          h('div', { className: 'provider-console-actions' }, h(Button, { disabled: !ready, onClick: send }, text.send),
            draft.pending && h(Button, { variant: 'secondary', disabled: draft.cancelling, onClick: cancel }, text.cancel)))),
        h('section', { className: 'provider-console-result', 'aria-label': text.result, 'aria-busy': draft.pending },
          h('h2', null, text.result),
          draft.status === 'idle' ? h('p', null, text.empty) : h('p', { role: 'status', 'aria-atomic': true }, draft.pending ? (draft.cancelling ? text.cancelling : text[draft.phase === 'model' ? 'modelPhase' : draft.phase]) : text[draft.status]),
          draft.content && h('pre', { 'data-testid': 'provider-console-reply', className: 'type-reading' }, draft.content),
          draft.reasoning && h('details', null, h('summary', null, text.reasoning), h('pre', null, draft.reasoning)),
          draft.code != null && h('p', { role: 'alert' }, text.error + String(draft.code)),
          draft.unknown && h('p', { role: 'alert' }, text.unknown),
          draft.auditWarning && h('p', { role: 'alert' }, text.auditWarning),
          draft.runId && h(Button, { variant: 'secondary', disabled: checking !== null, onClick: () => checkRun(draft.runId) }, checking === draft.runId ? text.checking : text.check))),
      h('section', { className: 'provider-console-history', 'aria-label': text.history },
        h('h2', null, text.history), h('p', null, text.historyHelp),
        h(Button, { variant: 'secondary', disabled: historyState === 'loading', onClick: () => readHistory() }, text.refreshHistory),
        historyState === 'loading' && h(Loading, { label: text.checking }),
        historyState === 'error' && h('p', { role: 'alert' }, text.historyFailed),
        lookupError && h('p', { role: 'alert' }, text[lookupError]),
        historyState === 'ready' && !history.items.length && h('p', null, text.noRuns),
        h('ul', null, history.items.map(item => h('li', { key: item.run_id },
          h('span', null, `${item.model_id} · ${text[item.status]} · ${new Date(item.created_at * 1000).toLocaleString(locale)}`),
          h('code', null, item.run_id), item.result_unknown && h('p', null, text.unknown),
          h(Button, { variant: 'secondary', disabled: checking !== null, onClick: () => checkRun(item.run_id) }, checking === item.run_id ? text.checking : text.check)))),
        history.next_before && h(Button, { variant: 'secondary', disabled: historyState === 'loading', onClick: () => readHistory(true) }, text.more)));
  }

  return {
    Component,
    openView({ viewId, documentId }) {
      const key = documentId || viewId;
      ensure(key);
      return { Component: () => h(Component, { documentKey: key }),
        beforeClose: () => ({ dirty: !!documents.get(key)?.prompt || !!documents.get(key)?.content, blocked: !!documents.get(key)?.pending }),
        dispose: () => dispose(key) };
    },
    onUnload: () => { for (const key of documents.keys()) dispose(key); listeners.clear(); },
    onBeforeUpdate: () => Array.from(documents, ([key, value]) => [key, { ...value, pending: false, cancelling: false, unknown: value.unknown || value.pending, status: value.pending ? 'interrupted' : value.status }]),
    onAfterUpdate: state => {
      for (const [key, value] of state || []) {
        // 1.0.0 hands over only its draft fields. Its old pending request is never resumed.
        documents.set(key, initial({ ...value, pending: false, cancelling: false }));
      }
      for (const listener of listeners) listener();
    },
  };
}
