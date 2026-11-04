import { useEffect } from 'react';
import { Button } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useDevelopmentEditor, hasDevelopmentDraft } from '../../stores/developmentEditor';
import { errorText } from './shared';

export function FileEditor({ projectId, taskId, path }: { projectId: string; taskId: string | null; path: string }) {
  const { t } = useLocale();
  const editor = useDevelopmentEditor();
  useEffect(() => { void useDevelopmentEditor.getState().open(projectId, taskId, path); }, [projectId, taskId, path]);
  const dirty = hasDevelopmentDraft();
  const file = editor.file;
  return <article className="dev-review-stack" data-testid="dev-file-source">
    <div className="dev-review-toolbar"><strong className="break-all text-xs">{path}</strong><span role="status" className="text-xs">{t(editor.saving ? 'direct.saving' : dirty ? 'direct.unsaved' : 'direct.saved')}</span></div>
    {editor.busy ? <p role="status">{t('review.loading')}</p> : null}
    {editor.error ? <p role="alert" className="text-xs text-destructive">{errorText(editor.error)}</p> : null}
    {file ? <><p className="m-0 break-all text-xs text-muted-foreground">{t('review.bytes', { size: file.size })} · {file.hash}</p>
      {file.text !== null ? <><label className="dev-review-stack text-xs">{t('direct.source')}<textarea className="dev-editor" data-testid="dev-file-editor" aria-label={t('direct.source')} spellCheck={false} value={editor.draft} readOnly={!taskId} onChange={event => editor.edit(event.target.value)} onKeyDown={event => { if ((event.metaKey || event.ctrlKey) && event.key === 's') { event.preventDefault(); void editor.save(); } }} /></label>
      {!taskId ? <p className="dev-evidence-warning">{t('direct.taskRequired')}</p> : null}
      <div className="flex flex-wrap gap-2"><Button size="xs" data-testid="dev-file-save" disabled={!taskId || !dirty || editor.saving || editor.error === 'file_changed'} onClick={() => void editor.save()}>{t('direct.save')}</Button><Button size="xs" variant="secondary" disabled={editor.saving} onClick={() => void editor.reloadDisk()}>{t('direct.reload')}</Button></div></> : <p className="text-xs">{t(`review.reason.${file.reason ?? 'unavailable'}`)}</p>}
    </> : null}
    {editor.error === 'file_changed' ? <section className="dev-review-stack" aria-label={t('direct.conflict')} data-testid="dev-file-conflict"><p role="alert" className="dev-evidence-warning">{t('direct.conflict')}</p><div className="dev-comparison"><div><h3 className="text-xs">{t('direct.localDraft')}</h3><pre className="dev-diff" tabIndex={0}>{editor.draft}</pre></div><div><h3 className="text-xs">{t('direct.diskContent')}</h3><pre className="dev-diff" tabIndex={0}>{editor.disk?.text ?? t('direct.diskUnavailable')}</pre></div></div><Button size="xs" variant="secondary" data-testid="dev-file-reload" disabled={editor.saving} onClick={() => void editor.reloadDisk()}>{t('direct.reloadDiscard')}</Button></section> : null}
  </article>;
}
