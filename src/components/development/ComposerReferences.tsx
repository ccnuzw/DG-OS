import { useEffect, useRef, useState } from 'react';
import { Button, Input, Popover } from '@dgos/ui';
import { FilePlus, Folder, X } from 'lucide-react';
import { useLocale } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { addConversationReference, clearConversationReferences, useConversationReferences } from '../../stores/developmentReferences';
import { listDevelopmentFiles, readDevelopmentFile, type DevFileList } from '../../stores/developmentReview';
import { useSurfaceActive } from '../../workspace/SurfaceContext';
import { errorText } from './shared';
import './composer-references.css';

/** One draft owner drives both the chips above the prompt and the right-side picker. */
export function useComposerReferences({ projectId, taskId, ownerId, disabled = false, isCurrent }: { projectId: string | null; taskId: string | null; ownerId: string; disabled?: boolean; isCurrent: () => boolean }) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const projects = useDevelopment(state => state.projects);
  const { generation, pending } = useContextSelection();
  const active = useSurfaceActive();
  const references = useConversationReferences(projectId ?? '', ownerId);
  const owner = JSON.stringify([epoch, generation, projectId, taskId, ownerId]);
  const [opened, setOpened] = useState<string | null>(null);
  const [path, setPath] = useState('');
  const [query, setQuery] = useState('');
  const [mode, setMode] = useState<'file' | 'path'>('file');
  const [listing, setListing] = useState<DevFileList | null>(null);
  const [loading, setLoading] = useState(false);
  const [reading, setReading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sequence = useRef(0);
  const mounted = useRef(true);
  const mention = useRef(false);
  const list = useRef<HTMLUListElement>(null);
  const latest = useRef({ owner, eligible: false, isCurrent });
  latest.current = { owner, eligible: Boolean(projectId) && active && !pending && !disabled, isCurrent };
  const open = opened === owner && latest.current.eligible;
  function current(expectedOwner = owner) {
    return mounted.current && latest.current.owner === expectedOwner && latest.current.eligible && epoch === useDevelopment.getState().epoch && generation === contextGeneration() && latest.current.isCurrent();
  }
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++sequence.current; }; }, []);
  useEffect(() => { ++sequence.current; setOpened(null); setListing(null); setError(null); setReading(false); }, [owner, active, pending, disabled]);
  function close() { ++sequence.current; setOpened(null); setReading(false); }
  function show(fromMention: boolean) {
    if (!current()) return;
    mention.current = fromMention;
    ++sequence.current; setPath(''); setQuery(''); setListing(null); setError(null); setReading(false); setOpened(owner);
  }
  useEffect(() => {
    if (!open || !projectId) return;
    const request = ++sequence.current;
    setLoading(true); setListing(null); setError(null);
    void listDevelopmentFiles({ project_id: projectId, task_id: taskId, path, query, include_ignored: false }).then(result => {
      if (!current(owner) || request !== sequence.current) return;
      setLoading(false);
      if (result.ok) setListing(result.value);
      else setError(result.code);
    });
    return () => { ++sequence.current; };
  }, [open, owner, path, query]);
  async function choose(selectedPath: string, kind: 'file' | 'path') {
    if (!projectId || !current() || reading) return;
    const request = ++sequence.current;
    setError(null);
    if (kind === 'path') {
      addConversationReference(projectId, ownerId, { project_id: projectId, task_id: taskId, path: selectedPath, kind });
      close(); return;
    }
    setReading(true);
    const result = await readDevelopmentFile({ project_id: projectId, task_id: taskId, path: selectedPath });
    if (!current(owner) || request !== sequence.current) return;
    setReading(false);
    if (!result.ok) { setError(result.code); return; }
    if (result.value.text === null || !result.value.hash || result.value.reason) { setError(result.value.reason ?? 'file_access_denied'); return; }
    addConversationReference(projectId, ownerId, { project_id: projectId, task_id: taskId, path: result.value.path, kind: 'file', expected_hash: result.value.hash });
    close();
  }
  function promptChange(next: string, previous: string, caret: number | null, composing: boolean) {
    const end = caret ?? next.length;
    if (!composing && next.length === previous.length + 1 && next[end - 1] === '@' && (end === 1 || /\s/.test(next[end - 2])) && current()) {
      show(true);
      return next.slice(0, end - 1) + next.slice(end);
    }
    return next;
  }
  const chips = references.length ? <ul className="composer-reference-chips" aria-label={t('workbenchReferences.selected')}>
    {references.map(item => {
      const reference = item.reference;
      const range = reference.start_line !== undefined ? `:${reference.start_line}${reference.end_line !== undefined && reference.end_line !== reference.start_line ? `–${reference.end_line}` : ''}` : '';
      const label = `${projects.find(project => project.id === reference.project_id)?.name ?? reference.project_id} / ${reference.path}${range}`;
      return <li key={item.id} className="composer-reference-chip" data-reference-kind={reference.kind}>
        <span className="composer-reference-label" title={label}>{label}</span><span className="composer-reference-kind">{t(`workbenchReferences.kind.${reference.kind}`)}</span>
        <Button size="xs" variant="ghost" aria-label={t('workbenchReferences.remove', { path: label })} disabled={!latest.current.eligible} onClick={() => { if (projectId && current()) clearConversationReferences(projectId, ownerId, [item]); }}><X size={13} aria-hidden="true" /></Button>
      </li>;
    })}
  </ul> : null;
  const picker = <Popover label={t('workbenchReferences.add')} side="top" align="end" open={open} onOpenChange={next => next ? show(false) : close()} onCloseAutoFocus={event => {
    if (!current()) event.preventDefault();
    else if (mention.current) { event.preventDefault(); document.getElementById('agent-message')?.focus({ preventScroll: true }); mention.current = false; }
  }} trigger={<Button size="xs" variant="ghost" aria-label={t('workbenchReferences.add')} disabled={!latest.current.eligible}><FilePlus size={15} aria-hidden="true" /><span>{t('workbenchReferences.files')}</span></Button>} content={<div className="composer-reference-picker">
    <h3>{t('workbenchReferences.add')}</h3>
    <p>{t('workbenchReferences.boundary')}</p>
    <Input label={t('workbenchReferences.search')} type="search" value={query} disabled={reading} onChange={event => { ++sequence.current; setQuery(event.target.value); }} onKeyDown={event => { if (event.key === 'ArrowDown') { event.preventDefault(); list.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus(); } }} />
    <fieldset className="composer-reference-modes" disabled={reading}><legend>{t('workbenchReferences.include')}</legend>{(['file', 'path'] as const).map(kind => <label key={kind}><input type="radio" name={`reference-mode-${ownerId}`} checked={mode === kind} onChange={() => setMode(kind)} />{t(`workbenchReferences.kind.${kind}`)}</label>)}</fieldset>
    {path ? <div className="composer-reference-location"><Button size="xs" variant="ghost" disabled={reading} onClick={() => { ++sequence.current; setPath(path.split('/').slice(0, -1).join('/')); setQuery(''); }}>{t('workbenchReferences.parent')}</Button><span>{path}</span></div> : null}
    {loading || reading ? <p role="status">{t(reading ? 'workbenchReferences.reading' : 'workbenchReferences.loading')}</p> : null}
    {error ? <p role="alert" className="text-status-error">{errorText(error)}</p> : null}
    <ul ref={list} className="composer-reference-files" aria-label={t('workbenchReferences.files')} onKeyDown={event => {
      if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
      const buttons = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)'));
      const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
      if (!buttons.length || index < 0) return;
      event.preventDefault();
      buttons[event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (index + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length]?.focus();
    }}>{listing?.entries.map(entry => <li key={entry.path}><Button size="xs" variant="ghost" disabled={reading || entry.kind === 'blocked' || entry.kind === 'symlink'} onClick={() => {
      if (entry.kind === 'directory') { ++sequence.current; setPath(entry.path); setQuery(''); }
      else void choose(entry.path, mode);
    }}>{entry.kind === 'directory' ? <Folder size={14} aria-hidden="true" /> : null}<span>{query ? entry.path : entry.name}</span></Button>{entry.kind === 'directory' && mode === 'path' ? <Button size="xs" variant="ghost" disabled={reading} aria-label={t('workbenchReferences.addPath', { path: entry.path })} onClick={() => void choose(entry.path, 'path')}>{t('workbenchReferences.kind.path')}</Button> : null}</li>)}</ul>
    {listing && !listing.entries.length ? <p>{t('workbenchReferences.empty')}</p> : null}
    {listing?.truncated ? <p role="status">{t('workbenchReferences.truncated')}</p> : null}
    <Button size="xs" variant="secondary" onClick={close}>{t('common.cancel')}</Button>
  </div>} />;
  return { references, chips, picker, promptChange };
}
