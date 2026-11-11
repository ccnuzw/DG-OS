import { useId, useRef, useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { Check, CheckCheck, Clock3, FileText, Info, MessageCircle, Send, ShieldCheck, Terminal, X } from 'lucide-react';
import { useLocale } from '../../i18n';
import type { AgentEvent, Engine, Json, Pending } from '../../stores/agentRunnerStore';
import { ConfirmedExternalLink } from '../chat/SourcePreview';
import { safeSourceUrl } from '../chat/sources';
import { AgentDiff, LazyDetails, NativeValue } from './AgentExecutionViews';
import { NativeElicitation, elicitationDefaults, elicitationValid } from './NativeElicitation';
import { object, text } from './agentTranscript';
import './agent-native-request.css';

interface RequestProps {
  engine: Engine;
  pending: Pending;
  sessionId?: string;
  generation?: string;
  events?: readonly AgentEvent[];
  activeRun?: string | null;
  submit: (response: Json) => Promise<boolean>;
}

const commandApproval = 'item/commandExecution/requestApproval';
const fileApproval = 'item/fileChange/requestApproval';
const permissionApproval = 'item/permissions/requestApproval';
const userInput = 'item/tool/requestUserInput';
const mcpElicitation = 'mcpServer/elicitation/request';
const claudePermission = 'claude/toolPermission';
const decisionLabels: Record<string, string> = { accept: 'agent.allow', acceptForSession: 'agent.allowSession', decline: 'agent.deny', cancel: 'common.cancel' };

export function Request(props: RequestProps) {
  return <RequestBody key={JSON.stringify([props.engine, props.sessionId, props.generation, props.pending.id])} {...props} />;
}

function FileApprovalChanges({ data, events, activeRun }: { data: Record<string, Json>; events?: readonly AgentEvent[]; activeRun?: string | null }) {
  const { t } = useLocale();
  let native = data;
  // Item IDs are native-run scoped. Never borrow a matching name from an earlier run.
  if (!Array.isArray(data.changes) && typeof data.diff !== 'string' && activeRun && text(data.itemId) && events) {
    for (let index = events.length - 1; index >= 0; index--) {
      const event = events[index];
      const item = object(event.data);
      if (event.run !== activeRun || (event.kind !== 'fileChange' && item.type !== 'fileChange') || (item.id !== data.itemId && item.itemId !== data.itemId)
        || (text(item.turnId) && item.turnId !== data.turnId) || (text(item.threadId) && item.threadId !== data.threadId)) continue;
      native = item;
      break;
    }
  }
  const changes = Array.isArray(native.changes) && native.changes.length ? native.changes : text(native.diff) ? [native] : [];
  return <div className="agent-native-request-group agent-native-request-content-card">
    <strong className="agent-native-request-section-title"><FileText size={14} aria-hidden="true" />{t('agentRequest.fileChanges')}</strong>
    {changes.length ? changes.map((value, index) => {
      const change = object(value);
      const kind = object(change.kind);
      const changeType = text(kind.type) || text(change.kind);
      const path = text(change.path);
      const movePath = text(kind.movePath) || text(kind.move_path);
      const diff = text(change.diff);
      return <div className="agent-native-request-group" key={index}>
        {path || movePath || changeType ? <p><code>{path}</code>{movePath ? <> → <code>{movePath}</code></> : null}{changeType ? <> ({t(['add', 'delete', 'update'].includes(changeType) ? `agentRequest.change.${changeType}` : 'agentRequest.change.unknown')})</> : null}</p> : null}
        {diff ? <AgentDiff diff={diff} source="native" /> : <p className="agent-native-request-note">{t('agentRequest.diffUnavailable')}</p>}
      </div>;
    }) : <p className="agent-native-request-note">{t('agentRequest.diffUnavailable')}</p>}
  </div>;
}

function PermissionSummary({ value }: { value: Json | undefined }) {
  const { t } = useLocale();
  const permissions = object(value);
  const network = object(permissions.network).enabled;
  const files = object(permissions.fileSystem);
  const fileRules = (['read', 'write'] as const).flatMap(access => {
    const values = files[access];
    return Array.isArray(values) ? values.flatMap(path => typeof path === 'string' ? [{ access, path }] : []) : [];
  });
  const entries = Array.isArray(files.entries) ? files.entries : [];
  const hasDepth = typeof files.globScanMaxDepth === 'number';
  return <div className="agent-native-request-group agent-native-request-content-card">
    <strong className="agent-native-request-section-title"><ShieldCheck size={14} aria-hidden="true" />{t('agent.permissionExpansion')}</strong>
    {typeof network === 'boolean' ? <p>{t(network ? 'agentRequest.networkOn' : 'agentRequest.networkOff')}</p> : null}
    {fileRules.length || entries.length ? <ul className="agent-native-request-rules">
      {fileRules.map(({ access, path }, index) => <li key={index}>{t(`agentRequest.access.${access}`)}: <code>{path}</code></li>)}
      {entries.map((value, index) => {
        const entry = object(value);
        const path = object(entry.path);
        const special = object(path.value);
        const kind = text(special.kind);
        const specialName = ['root', 'minimal', 'tmpdir', 'slash_tmp', 'project_roots', 'current_working_directory'].includes(kind) ? t(`agentRequest.path.${kind}`) : kind;
        const access = text(entry.access);
        return <li key={index}>{['read', 'write', 'deny'].includes(access) ? t(`agentRequest.access.${access}`) : access}: {path.type === 'special' ? <>{specialName}{text(special.subpath) ? <> / <code>{text(special.subpath)}</code></> : null}</> : <code>{text(path.path) || text(path.pattern)}</code>}</li>;
      })}
    </ul> : null}
    {hasDepth ? <p>{t('agentRequest.globDepth', { depth: String(files.globScanMaxDepth) })}</p> : null}
    {typeof network !== 'boolean' && !fileRules.length && !entries.length && !hasDepth ? <p>{t('agentRequest.noPermissionDetails')}</p> : null}
  </div>;
}

function RequestBody({ engine, pending, sessionId, generation, events, activeRun, submit }: RequestProps) {
  const { t } = useLocale();
  const fieldId = useId();
  const [answers, setAnswers] = useState<Record<string, string[]>>({});
  const [busy, setBusy] = useState(false);
  const responding = useRef(false);
  const [failed, setFailed] = useState(false);
  const [confirmedRules, setConfirmedRules] = useState<Record<number, boolean>>({});
  const data = object(pending.data);
  const input = engine === 'claude' ? object(data.input) : data;
  const claudeQuestion = engine === 'claude' && pending.kind === claudePermission && (pending.title === 'AskUserQuestion' || data.tool === 'AskUserQuestion');
  const codexQuestion = engine === 'codex' && pending.kind === userInput;
  const questions = (claudeQuestion || codexQuestion) && Array.isArray(input.questions) ? input.questions.map(object) : [];
  const names = questions.map(question => claudeQuestion ? question.question : question.id);
  const questionsSupported = Array.isArray(input.questions) && (codexQuestion || names.length > 0)
    && names.every(name => typeof name === 'string') && new Set(names).size === names.length;
  const claudeComplete = questionsSupported && names.every(name => !!(Object.prototype.hasOwnProperty.call(answers, String(name)) && answers[String(name)].join(', ').trim()));
  const schema = object(data.requestedSchema);
  const elicitation = engine === 'codex' && pending.kind === mcpElicitation;
  const formMode = elicitation && (data.mode === undefined || data.mode === 'form');
  const urlMode = elicitation && data.mode === 'url';
  const externalUrl = urlMode ? safeSourceUrl(text(data.url)) : null;
  const [form, setForm] = useState<Record<string, Json>>(() => elicitationDefaults(schema));
  const canAccept = (!(claudeQuestion || codexQuestion) || questionsSupported && (!claudeQuestion || claudeComplete))
    && (!elicitation || urlMode || (formMode && elicitationValid(schema, form)));

  async function respond(value: Json) {
    if (responding.current) return;
    responding.current = true;
    setBusy(true);
    setFailed(false);
    try { if (await submit(value)) return; } catch { /* Keep input and allow only an explicit retry. */ }
    responding.current = false;
    setFailed(true);
    setBusy(false);
  }

  function accept() {
    if (!canAccept) return;
    if (claudeQuestion) return respond({ behavior: 'allow', answers: Object.fromEntries(names.map(name => [String(name), answers[String(name)].join(', ')])) });
    if (engine === 'claude') return respond({ behavior: 'allow' });
    if (codexQuestion) return respond({ answers: Object.fromEntries(Object.entries(answers).map(([key, value]) => [key, { answers: value }])) });
    return respond({ action: 'accept', content: urlMode ? null : form });
  }

  const approval = engine === 'codex' && [commandApproval, fileApproval].includes(pending.kind);
  const permissions = engine === 'codex' && pending.kind === permissionApproval;
  const decisions: Json[] = Array.isArray(data.availableDecisions) ? data.availableDecisions.filter(value => typeof value === 'string'
    ? Object.prototype.hasOwnProperty.call(decisionLabels, value)
    : pending.kind === commandApproval && value !== null && typeof value === 'object' && !Array.isArray(value)
      && Object.keys(value).length === 1 && (Object.prototype.hasOwnProperty.call(value, 'acceptWithExecpolicyAmendment') || Object.prototype.hasOwnProperty.call(value, 'applyNetworkPolicyAmendment'))) : ['accept', 'decline', 'cancel'];
  const supported = (engine === 'claude' && pending.kind === claudePermission) || codexQuestion || elicitation;
  const heading = claudeQuestion || codexQuestion ? t('agentRequest.questions') : elicitation ? t('agentRequest.elicitation') : engine === 'claude' ? pending.title : t('agent.approval');
  const command = text(input.command) || (Array.isArray(input.command) && input.command.every(argument => typeof argument === 'string') ? JSON.stringify(input.command) : '');
  const directory = text(input.cwd);
  const path = text(input.file_path) || text(input.path) || text(input.grantRoot);
  const network = object(data.networkApprovalContext);
  const RequestIcon = claudeQuestion || codexQuestion ? MessageCircle : elicitation ? Info : pending.kind === commandApproval ? Terminal : pending.kind === fileApproval ? FileText : ShieldCheck;
  const inputRequest = claudeQuestion || codexQuestion || elicitation;

  return <section data-testid="agent-native-request" data-request-id={pending.id} data-request-kind={pending.kind} data-request-tone={inputRequest ? 'input' : 'approval'} className="agent-native-request" aria-label={heading} aria-busy={busy}>
    <header className="agent-native-request-header">
      <span className="agent-native-request-heading-icon"><RequestIcon size={18} aria-hidden="true" /></span>
      <div className="agent-native-request-heading">
        <strong>{heading}</strong>
        <p className="agent-native-request-waiting"><Clock3 size={12} aria-hidden="true" />{t(busy ? 'agentExecution.processing' : 'agent.state.waiting')}</p>
      </div>
    </header>
    <div className="agent-native-request-body">
    {text(data.message) ? <p className="agent-native-request-message">{text(data.message)}</p> : null}
    {text(data.reason) && data.reason !== data.message ? <p className="agent-native-request-message">{text(data.reason)}</p> : null}
    {command ? <div className="agent-native-request-group agent-native-request-content-card"><strong className="agent-native-request-section-title"><Terminal size={14} aria-hidden="true" />{t('agent.requestCommand')}</strong><pre className="type-code agent-native-request-code">{command}</pre></div> : null}
    {directory || path ? <dl className="agent-native-request-metadata">
      {directory ? <div><dt>{t('agent.requestDirectory')}</dt><dd><code>{directory}</code></dd></div> : null}
      {path ? <div><dt>{t('agent.requestPath')}</dt><dd><code>{path}</code></dd></div> : null}
    </dl> : null}
    {text(network.host) ? <p>{t('agentRequest.networkTarget')}: <code>{text(network.protocol)}://{text(network.host)}</code></p> : null}
    {engine === 'claude' && !claudeQuestion ? <div className="agent-native-request-group agent-native-request-content-card"><strong className="agent-native-request-section-title"><Info size={14} aria-hidden="true" />{t('agent.requestInput')}</strong><NativeValue value={data.input} /></div> : null}
    {approval && pending.kind === fileApproval ? <FileApprovalChanges data={data} events={events} activeRun={activeRun} /> : null}
    {permissions ? <PermissionSummary value={data.permissions} /> : null}
    {permissions ? <p className="agent-native-request-note agent-native-request-scope"><ShieldCheck size={14} aria-hidden="true" /><span>{t('agent.permissionScope')}</span></p> : approval && decisions.includes('acceptForSession') ? <p className="agent-native-request-note agent-native-request-scope"><Info size={14} aria-hidden="true" /><span>{t('agentRequest.approvalScopes')}</span></p> : approval || (supported && !claudeQuestion && !codexQuestion && !elicitation) ? <p className="agent-native-request-note agent-native-request-scope"><ShieldCheck size={14} aria-hidden="true" /><span>{t('agentExecution.onceScope')}</span></p> : null}
    {urlMode ? <fieldset disabled={busy} className="agent-native-request-group agent-native-request-content-card">
      <legend>{t('agentRequest.externalPage')}</legend>
      <p className="agent-native-request-note">{t('agentRequest.externalHelp')}</p>
      {externalUrl ? <><p><code>{externalUrl}</code></p><div><ConfirmedExternalLink url={externalUrl}>{t('agentRequest.openExternal')}</ConfirmedExternalLink></div></> : <><p><code>{text(data.url)}</code></p><p className="agent-native-request-note">{t('agentRequest.invalidExternalUrl')}</p></>}
    </fieldset> : text(data.url) ? <p><code>{text(data.url)}</code></p> : null}
    {(claudeQuestion || codexQuestion) && !questionsSupported ? <p className="agent-native-request-error" role="alert">{t('agentRequest.invalidQuestions')}</p> : null}
    {questions.map((question, index) => {
      const key = typeof names[index] === 'string' ? String(names[index]) : '';
      const choices = Array.isArray(question.options) ? question.options.map(object) : [];
      const selected = Object.prototype.hasOwnProperty.call(answers, key) ? answers[key] : [];
      const prompt = text(question.question) || key;
      const header = text(question.header) || prompt;
      const error = claudeQuestion && !selected.join(', ').trim() ? t('agentRequest.answerRequired') : undefined;
      const customId = `${fieldId}-${index}`;
      const promptId = prompt !== header ? `${customId}-prompt` : undefined;
      const describedBy = [promptId, error && `${customId}-error`].filter(Boolean).join(' ') || undefined;
      const setAnswer = (value: string[]) => setAnswers(current => ({ ...current, [key]: value }));
      return <fieldset key={index} className="agent-native-request-group agent-native-request-field" disabled={busy || !questionsSupported} aria-invalid={error ? true : undefined} aria-describedby={describedBy}>
        <legend>{header}</legend>
        {promptId ? <p id={promptId}>{prompt}</p> : null}
        {choices.length ? <div className="agent-native-request-choices">{choices.map((option, index) => {
          const label = text(option.label);
          if (!label) return null;
          const chosen = selected.includes(label);
          return <Button key={index} variant="secondary" className="agent-native-request-choice" aria-pressed={chosen} aria-invalid={error ? true : undefined} aria-describedby={describedBy} onClick={() => setAnswer(claudeQuestion && question.multiSelect ? selected.includes(label) ? selected.filter(item => item !== label) : [...selected, label] : [label])}>
            <span className="agent-native-request-choice-indicator" aria-hidden="true">{chosen ? <Check size={12} /> : null}</span>
            <span className="agent-native-request-choice-copy"><span className="agent-native-request-choice-label">{label}</span>{text(option.description) ? <span className="agent-native-request-choice-description">{text(option.description)}</span> : null}</span>
          </Button>;
        })}</div> : null}
        {claudeQuestion || !choices.length || question.isOther === true || question.isSecret === true ? <Input id={customId} type={question.isSecret ? 'password' : 'text'} label={t('agent.customAnswer')} aria-describedby={promptId} aria-required={claudeQuestion && !choices.length || undefined} error={error} autoComplete="off" value={selected.join(', ')} onChange={e => setAnswer(e.target.value ? [e.target.value] : [])} /> : null}
      </fieldset>;
    })}
    {formMode ? <NativeElicitation schema={schema} values={form} disabled={busy} onChange={setForm} /> : elicitation && !urlMode ? <p role="alert">{t('agentRequest.unsupportedElicitation')}</p> : null}
    </div>
    <footer className="agent-native-request-footer">
    {failed ? <p role="alert" className="agent-native-request-error">{t('agentRequest.sendFailed')}</p> : null}
    {busy ? <p role="status" className="agent-native-request-submitting"><Clock3 size={14} aria-hidden="true" /><span>{t('agent.requestSending')}</span></p> : null}
    {permissions ? <div className="agent-native-request-actions">
      <Button data-decision="accept-turn" disabled={busy} onClick={() => void respond({ permissions: data.permissions, scope: 'turn' })}><Check size={14} aria-hidden="true" />{t('agent.allowTurn')}</Button>
      <Button data-decision="accept-session" variant="secondary" disabled={busy} onClick={() => void respond({ permissions: data.permissions, scope: 'session' })}><CheckCheck size={14} aria-hidden="true" />{t('agent.allowSession')}</Button>
      <Button data-decision="decline" variant="secondary" disabled={busy} onClick={() => void respond({ permissions: {}, scope: 'turn' })}><X size={14} aria-hidden="true" />{t('agent.deny')}</Button>
    </div> : approval ? <>
      {!decisions.length ? <p className="agent-native-request-note">{t('agentExecution.unsupported')}</p> : null}
      {decisions.some(decision => typeof decision === 'string') ? <div className="agent-native-request-actions">{decisions.map((decision, index) => typeof decision === 'string' ? <Button key={index} data-decision={decision} variant={decision === 'accept' ? 'primary' : 'secondary'} disabled={busy} onClick={() => void respond({ decision })}>{decision === 'accept' ? <Check size={14} aria-hidden="true" /> : decision === 'acceptForSession' ? <CheckCheck size={14} aria-hidden="true" /> : <X size={14} aria-hidden="true" />}{t(decisionLabels[decision])}</Button> : null)}</div> : null}
      {decisions.map((decision, index) => typeof decision !== 'string' ? <fieldset key={index} disabled={busy} className="agent-native-request-group agent-native-request-field">
        <legend>{t('agent.nativePolicy')}</legend>
        <NativeValue value={decision} />
        <label className="agent-native-request-option"><input type="checkbox" checked={Boolean(confirmedRules[index])} onChange={event => setConfirmedRules(current => ({ ...current, [index]: event.target.checked }))} /><span>{t('agent.confirmPolicy')}</span></label>
        <div><Button variant="secondary" data-decision="apply-policy" disabled={busy || !confirmedRules[index]} onClick={() => void respond({ decision })}>{t('agent.applyPolicy')}</Button></div>
      </fieldset> : null)}
    </> : supported ? <div className="agent-native-request-actions">
      <Button disabled={busy || !canAccept} onClick={() => void accept()}>{inputRequest ? <Send size={14} aria-hidden="true" /> : <Check size={14} aria-hidden="true" />}{t(claudeQuestion || codexQuestion ? 'agent.answer' : 'agent.allow')}</Button>
      <Button variant="secondary" disabled={busy} onClick={() => void respond(engine === 'claude' ? { behavior: 'deny' } : elicitation ? { action: 'decline', content: null } : { answers: {} })}><X size={14} aria-hidden="true" />{t('agent.deny')}</Button>
      {elicitation ? <Button variant="ghost" disabled={busy} onClick={() => void respond({ action: 'cancel', content: null })}>{t('common.cancel')}</Button> : null}
    </div> : <p className="agent-native-request-note">{t('agentExecution.unsupported')}</p>}
    </footer>
    <div className="agent-native-request-diagnostics">
    <details className="agent-native-request-details"><summary>{t('agent.permissionIdentity')}</summary>
      <dl className="agent-native-request-metadata">
        <div><dt>{t('agentExecution.operation')}</dt><dd>{pending.title} · {pending.kind}</dd></div>
        <div><dt>{t('agentExecution.requestId')}</dt><dd>{pending.id}</dd></div>
        {sessionId ? <div><dt>{t('agentExecution.sessionId')}</dt><dd>{sessionId}</dd></div> : null}
        {generation ? <div><dt>{t('agentExecution.generation')}</dt><dd>{generation}</dd></div> : null}
        {text(data.threadId) || text(data.turnId) || text(data.itemId) ? <div><dt>{t('agent.permissionIdentity')}</dt><dd>{text(data.threadId) || '—'} / {text(data.turnId) || '—'} / {text(data.itemId) || '—'}</dd></div> : null}
      </dl>
    </details>
    <LazyDetails summary={t('agent.details')}><pre className="type-code agent-native-request-code">{JSON.stringify(pending.data, null, 2)}</pre></LazyDetails>
    </div>
  </section>;
}
