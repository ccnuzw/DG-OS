// Provider Console 1.2: public host.ai foundation consumer; no automatic paid recovery.
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
      reasoning: 'Reasoning', history: 'This module’s runs', historyHelp: 'Status is durable. Only explicitly retained content can be retrieved for up to 24 hours. Reading and acknowledging never call the model.',
      refreshHistory: 'Refresh history', more: 'Load earlier runs', check: 'Check status', checking: 'Checking…', historyFailed: 'Could not read this module’s run status.', missing: 'Run not found in this module’s history.', noRuns: 'No recorded runs.', schema: 'Input schema', pin: 'Content hash', revision: 'Provider revision',
      resources: 'Context resources', import: 'Import context file', remove: 'Delete resource', start: 'Start byte', bytes: 'Maximum bytes', inline: 'Inline', onDemand: 'On demand', mode: 'Delivery mode', resourceHelp: 'Select immutable imported sources and byte ranges. Hashes bind versions, not permissions. Reads expire after 24 hours.',
      skillMode: 'Skill loading', manual: 'Manual', retain: 'Retain result for up to 24 hours', retainHelp: 'Off by default. At most 32 retained bodies; acknowledgement does not save a business document.', structured: 'Validate output against JSON Schema', outputSchema: 'Output JSON Schema', outputHelp: 'Host validation after generation, not a Provider guarantee. Invalid output is not repaired automatically.',
      inspect: 'Inspect context budget', budget: 'Host context budget', rejected: 'Context rejected', invalidSchema: 'Enter valid JSON for the output schema.', actionFailed: 'The host operation failed. Refresh or correct the selection before trying again.', source: 'Source run ID (explicit continuation)', operation: 'Operation', generate: 'Generate / continue', summarize: 'Summarize selected sources', confirmed: 'I confirm an additional model call for this summary', retrieve: 'Retrieve retained result', ack: 'Acknowledge receipt', acknowledged: 'Receipt acknowledged', expired: 'Expired', noContent: 'No retained content is available; this does not mean the run was never dispatched.', receipts: 'Host delivery and tool receipts', valid: 'Output validation', declared: 'Declared capabilities (not verified)',
      processing_result: 'Processing tool result…', final_answer: 'Preparing final response…',
    },
    'zh-CN': {
      title: '模型调试台', help: '使用已配置模型及所选技能、工具生成内容。每次工具调用均需宿主审批；请在模块设置中管理权限。',
      provider: '服务', model: '模型', prompt: '提示词', choose: '请选择', send: '发送', cancel: '取消运行', cancelling: '已请求取消，正在等待最终状态。',
      refresh: '刷新选择目录', loading: '正在读取目录…', catalogFailed: '目录不可用或权限被拒绝。请检查模块权限后刷新。', noProviders: '没有可用服务，请在设置中配置。',
      skills: '内容技能', tools: '工具', optional: '可选，不会自动选择。', emptyCatalog: '没有可用条目。', stale: '所选修订已不可用，请刷新目录并重新选择。',
      result: '调用结果', empty: '选择服务、模型并填写提示词后发送。', modelPhase: '正在生成…', awaiting_confirmation: '正在等待宿主审批，请在宿主弹框中核对本次工具调用。', tool: '正在执行已批准工具…',
      succeeded: '已完成', failed: '失败', cancelled: '已取消', interrupted: '已中断', running: '运行中', error: '宿主拒绝调用或未能完成请求，错误码：',
      unknown: '外部结果未知。请先核实状态，再决定是否重新发送。', auditWarning: '宿主未能确认审计写入，不能据此认定可以安全重试。',
      reasoning: '思考过程', history: '本模块的运行记录', historyHelp: '运行状态持久记录。仅显式保留的正文可在最长24小时内领取；读取和确认不会调用模型。',
      refreshHistory: '刷新记录', more: '加载更早记录', check: '核实状态', checking: '正在核实…', historyFailed: '无法读取本模块的运行状态。', missing: '本模块历史中未找到此运行。', noRuns: '暂无运行记录。', schema: '输入结构', pin: '内容摘要', revision: '服务修订',
      resources: '上下文资料', import: '导入上下文文件', remove: '删除资料', start: '起始字节', bytes: '最大字节数', inline: '直接提供', onDemand: '按需读取', mode: '提供方式', resourceHelp: '明确选择不可变资料及字节范围，摘要绑定版本而非权限；资料24小时后过期。',
      skillMode: '技能加载方式', manual: '手动', retain: '保留结果最长24小时', retainHelp: '默认关闭，最多保留32份正文；确认领取不代表业务文档已保存。', structured: '按 JSON Schema 校验输出', outputSchema: '输出 JSON Schema', outputHelp: '宿主在生成后校验，不是服务端结构保证；校验失败不会自动收费修复。',
      inspect: '检查上下文预算', budget: '宿主上下文预算', rejected: '上下文被拒绝', invalidSchema: '请填写合法的 JSON 输出结构。', actionFailed: '宿主操作失败，请刷新或修正选择后重试。', source: '来源运行 ID（显式继续）', operation: '操作', generate: '生成／继续', summarize: '摘要所选来源', confirmed: '我确认本次摘要会额外调用模型', retrieve: '领取保留结果', ack: '确认领取', acknowledged: '已确认领取', expired: '已过期', noContent: '没有可领取正文，不代表此运行从未派发。', receipts: '宿主提供片段与工具回执', valid: '输出校验', declared: '声明能力（未经实测）',
      processing_result: '正在处理工具结果…', final_answer: '正在生成最终回复…',
    },
  };
  const documents = new Map();
  const listeners = new Set();
  const controllers = new Map();
  const keyOfSkill = entry => JSON.stringify([entry.skill_id, entry.version, entry.content_hash]);
  const keyOfTool = entry => JSON.stringify([entry.server_id, entry.tool_name, entry.schema_hash]);
  const skillRef = ({ skill_id, version, content_hash }) => ({ skill_id, version, content_hash });
  const toolRef = ({ server_id, tool_name, schema_hash }) => ({ server_id, tool_name, schema_hash });
  const initial = value => ({ providerId: '', modelId: '', providerRevision: null, prompt: '', skills: [], tools: [], resources: [], contextRefs: [], skillMode: 'manual', retain: false, structured: false, outputSchema: '', sourceRunId: '', operation: 'generate', confirmed: false, retained: null, content: '', reasoning: '', status: 'idle', pending: false, runId: null, unknown: false, auditWarning: false, code: null, phase: 'model', ...value });
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
    const [actionBusy, setActionBusy] = React.useState(false);
    const [actionError, setActionError] = React.useState(null);
    const [inspection, setInspection] = React.useState(null);
    const actionLock = React.useRef(false);
    const input = () => ({ provider_id: draft.providerId, model_id: draft.modelId, provider_revision: draft.providerRevision,
      prompt: draft.prompt, skill_refs: draft.skills.map(skillRef), tools: draft.tools.map(toolRef), generation: {},
      foundation: { skill_mode: draft.skillMode, context_refs: draft.contextRefs, output: draft.structured ? { kind: 'json_schema', schema: JSON.parse(draft.outputSchema) } : { kind: 'text' }, retain_result: draft.retain, source_run_id: draft.sourceRunId.trim() || null, operation: draft.operation, confirmed: draft.confirmed } });
    const signature = JSON.stringify([draft.providerId, draft.modelId, draft.providerRevision, draft.prompt, draft.skills, draft.tools, draft.contextRefs, draft.skillMode, draft.retain, draft.structured, draft.outputSchema, draft.sourceRunId, draft.operation, draft.confirmed]);
    const currentBudget = inspection?.signature === signature ? inspection.budget : null;
    const perform = async action => {
      if (actionLock.current || documents.get(documentKey)?.pending) return;
      actionLock.current = true; setActionBusy(true); setActionError(null);
      try { await action(); } catch (error) { if (alive.current) setActionError(error instanceof SyntaxError ? text.invalidSchema : text.actionFailed); }
      finally { actionLock.current = false; if (alive.current) setActionBusy(false); }
    };
    const inspect = () => perform(async () => { const budget = await host.ai.inspect(input()); if (alive.current) setInspection({ signature, budget }); });
    const importFile = file => perform(async () => {
      if (!file) return;
      const base64 = await new Promise((resolve, reject) => { const reader = new FileReader(); reader.onerror = reject; reader.onload = () => resolve(String(reader.result).split(',')[1] || ''); reader.readAsDataURL(file); });
      const mime = /\.md$/i.test(file.name) ? 'text/markdown' : /\.txt$/i.test(file.name) ? 'text/plain' : file.type;
      const resource = await host.ai.importResource({ name: file.name, mime, base64 });
      const current = documents.get(documentKey);
      if (!current) { await host.ai.deleteResource(resource.id); return; }
      update(documentKey, { resources: [...current.resources, resource] });
    });
    const resourceRef = resource => draft.contextRefs.find(ref => ref.resource_id === resource.id);
    const editRef = (resource, patch) => update(documentKey, { contextRefs: draft.contextRefs.map(ref => ref.resource_id === resource.id ? { ...ref, ...patch } : ref) });
    const retrieve = runId => perform(async () => {
      const retained = await host.ai.getResult(runId);
      if (alive.current) update(documentKey, { retained });
      if (!retained && alive.current) setActionError(text.noContent);
    });
    const send = async () => {
      if (!ready || actionBusy || documents.get(documentKey)?.pending || (draft.operation === 'summarize' && !draft.confirmed)) return;
      let request;
      try { request = input(); } catch { setActionError(text.invalidSchema); return; }
      setActionError(null);
      const abort = new AbortController();
      controllers.set(documentKey, abort);
      update(documentKey, { pending: true, status: 'running', phase: 'model', content: '', reasoning: '', runId: null, unknown: false, auditWarning: false, code: null, cancelling: false });
      const active = () => controllers.get(documentKey) === abort && documents.has(documentKey);
      let terminalSeen = false;
      let modelRequested = false;
      const settle = terminal => {
        if (!active() || terminalSeen) return;
        terminalSeen = true;
        update(documentKey, { status: terminal.status, content: terminal.content ?? documents.get(documentKey).content, unknown: terminal.result_unknown, auditWarning: terminal.audit_warning, code: terminal.error?.code ?? null });
      };
      try {
        const budget = await host.ai.inspect(request, { signal: abort.signal });
        if (!active()) return;
        if (alive.current) setInspection({ signature, budget });
        if (!budget.can_send) { update(documentKey, { status: 'failed' }); setActionError(budget.rejection || text.rejected); return; }
        modelRequested = true;
        const terminal = await host.ai.run(request, event => {
          if (!active() || terminalSeen) return;
          if (event.kind === 'started') update(documentKey, { runId: event.run_id });
          if (event.kind === 'delta') update(documentKey, { content: documents.get(documentKey).content + event.text });
          if (event.kind === 'reasoning_delta') update(documentKey, { reasoning: documents.get(documentKey).reasoning + event.text });
          if (event.kind === 'phase') update(documentKey, { phase: event.phase });
          if (event.kind === 'terminal') settle(event.terminal);
        }, { signal: abort.signal });
        settle(terminal);
      } catch (error) {
        if (active() && !terminalSeen) update(documentKey, { status: 'failed', code: error?.code ?? null, unknown: modelRequested && (!!documents.get(documentKey).runId || error?.code == null || error.code === 1000000) });
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
          h(Select, { label: text.skillMode, value: draft.skillMode, disabled: draft.pending, options: [{ value: 'manual', label: text.manual }, { value: 'on_demand', label: text.onDemand }], onChange: value => update(documentKey, { skillMode: value }) }),
          h('section', { className: 'provider-console-choices', 'aria-label': text.resources }, h('h2', null, text.resources), h('p', null, text.resourceHelp),
            h('label', null, text.import, h('input', { type: 'file', accept: '.txt,.md,text/plain,text/markdown,image/png,image/jpeg,image/webp', disabled: draft.pending || actionBusy, onChange: event => { const file = event.target.files?.[0]; event.target.value = ''; void importFile(file); } })),
            draft.resources.map(resource => h('div', { key: resource.id, className: 'provider-console-choice' },
              h(Checkbox, { label: resource.name, disabled: draft.pending || actionBusy, checked: !!resourceRef(resource), onChange: checked => update(documentKey, { contextRefs: checked ? [...draft.contextRefs, { resource_id: resource.id, sha256: resource.sha256, start: 0, max_bytes: resource.mime.startsWith('image/') ? resource.bytes_len : Math.min(resource.bytes_len, 8192), mode: 'inline' }] : draft.contextRefs.filter(ref => ref.resource_id !== resource.id) }) }),
              h('p', null, `${resource.bytes_len} bytes · ${new Date(resource.expires_at * 1000).toLocaleString(locale)}`), h('code', null, resource.sha256),
              resourceRef(resource) && h(Stack, { gap: 2 },
                h(Input, { label: `${text.start}: ${resource.name}`, type: 'number', min: 0, step: 1, disabled: draft.pending || resource.mime.startsWith('image/'), value: String(resourceRef(resource).start), onChange: event => editRef(resource, { start: Number(event.target.value) }) }),
                h(Input, { label: `${text.bytes}: ${resource.name}`, type: 'number', min: 1, step: 1, disabled: draft.pending || resource.mime.startsWith('image/'), value: String(resourceRef(resource).max_bytes), onChange: event => editRef(resource, { max_bytes: Number(event.target.value) }) }),
                h(Select, { label: `${text.mode}: ${resource.name}`, disabled: draft.pending, value: resourceRef(resource).mode, options: [{ value: 'inline', label: text.inline }, { value: 'on_demand', label: text.onDemand }], onChange: value => editRef(resource, { mode: value }) })),
              h(Button, { variant: 'secondary', disabled: draft.pending || actionBusy, onClick: () => perform(async () => { await host.ai.deleteResource(resource.id); const current = documents.get(documentKey); if (current) update(documentKey, { resources: current.resources.filter(item => item.id !== resource.id), contextRefs: current.contextRefs.filter(ref => ref.resource_id !== resource.id) }); }) }, text.remove)))),
          h(Checkbox, { label: text.structured, checked: draft.structured, disabled: draft.pending, onChange: value => update(documentKey, { structured: value }) }),
          draft.structured && h(Input, { label: text.outputSchema, multiline: true, rows: 5, value: draft.outputSchema, disabled: draft.pending, onChange: event => update(documentKey, { outputSchema: event.target.value }) }), h('p', null, text.outputHelp),
          h(Checkbox, { label: text.retain, checked: draft.retain, disabled: draft.pending, onChange: value => update(documentKey, { retain: value }) }), h('p', null, text.retainHelp),
          h(Input, { label: text.source, value: draft.sourceRunId, disabled: draft.pending, onChange: event => update(documentKey, { sourceRunId: event.target.value }) }),
          h(Select, { label: text.operation, value: draft.operation, disabled: draft.pending, options: [{ value: 'generate', label: text.generate }, { value: 'summarize', label: text.summarize }], onChange: value => update(documentKey, { operation: value, confirmed: false }) }),
          draft.operation === 'summarize' && h(Checkbox, { label: text.confirmed, checked: draft.confirmed, disabled: draft.pending, onChange: value => update(documentKey, { confirmed: value }) }),
          h(Button, { variant: 'secondary', disabled: !ready || actionBusy, onClick: inspect }, text.inspect),
          currentBudget && h('section', { 'aria-label': text.budget }, h('h2', null, text.budget), h('pre', null, JSON.stringify(currentBudget, null, 2))),
          models.find(model => model.model_id === draft.modelId) && h('details', null, h('summary', null, text.declared), h('pre', null, JSON.stringify({ capabilities: models.find(model => model.model_id === draft.modelId).capabilities, context_window: models.find(model => model.model_id === draft.modelId).context_window }, null, 2))),
          actionError && h('p', { role: 'alert' }, actionError),
          h(Input, { multiline: true, rows: 7, className: 'type-reading', label: text.prompt, value: draft.prompt, disabled: draft.pending, onChange: event => update(documentKey, { prompt: event.target.value }) }),
          h('div', { className: 'provider-console-actions' }, h(Button, { disabled: !ready || actionBusy || (draft.operation === 'summarize' && !draft.confirmed), onClick: send }, text.send),
            draft.pending && h(Button, { variant: 'secondary', disabled: draft.cancelling, onClick: cancel }, text.cancel)))),
        h('section', { className: 'provider-console-result', 'aria-label': text.result, 'aria-busy': draft.pending },
          h('h2', null, text.result),
          draft.status === 'idle' ? h('p', null, text.empty) : h('p', { role: 'status', 'aria-atomic': true }, draft.pending ? (draft.cancelling ? text.cancelling : text[draft.phase === 'model' ? 'modelPhase' : draft.phase]) : text[draft.status]),
          draft.content && h('pre', { 'data-testid': 'provider-console-reply', className: 'type-reading' }, draft.content),
          draft.reasoning && h('details', null, h('summary', null, text.reasoning), h('pre', null, draft.reasoning)),
          draft.retained && h('section', null, h('p', null, `${draft.retained.run_id}: ${text[draft.retained.status] || draft.retained.status}`),
            h('p', null, `${text.valid}: ${String(draft.retained.output_valid)}`),
            draft.retained.content === null ? h('p', null, text.noContent) : h('pre', { className: 'type-reading' }, draft.retained.content),
            h('details', null, h('summary', null, text.receipts), h('pre', null, JSON.stringify({ context: draft.retained.context_receipts, tools: draft.retained.tool_receipts, skills: draft.retained.skill_receipts, source_run_id: draft.retained.source_run_id, expires_at: draft.retained.expires_at }, null, 2))),
            draft.retained.acknowledged_at ? h('p', null, text.acknowledged) : h(Button, { variant: 'secondary', disabled: actionBusy || draft.pending, onClick: () => perform(async () => { const runId = draft.retained.run_id; await host.ai.ackResult(runId); const retained = await host.ai.getResult(runId); if (alive.current) update(documentKey, { retained }); }) }, text.ack)),
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
          h(Button, { variant: 'secondary', disabled: actionBusy || draft.pending, onClick: () => retrieve(item.run_id) }, text.retrieve),
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
