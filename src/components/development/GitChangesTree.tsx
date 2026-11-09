import { Button } from '@dgos/ui';
import { useLocale } from '../../i18n';
import type { GitFile, GitReview } from '../../stores/developmentDelivery';
import type { ProjectGitOverview } from '../../stores/developmentExplorer';

export function GitChangesTree({ review, selected, onSelect, onOpenDiff }: { review: GitReview | ProjectGitOverview; selected: string[]; onSelect: (path: string, checked: boolean) => void; onOpenDiff?: (path: string, source?: 'index' | 'worktree') => void }) {
  const { t } = useLocale();
  const conflicts = new Set(review.conflicts.map(entry => entry.path));
  const groups: { label: string; source: 'index' | 'worktree'; files: GitFile[] }[] = [
    { label: 'delivery.conflicts', source: 'worktree', files: review.files.filter(file => conflicts.has(file.path)) },
    { label: 'workbenchGit.unstaged', source: 'worktree', files: review.files.filter(file => !conflicts.has(file.path) && file.worktree.trim() !== '') },
    { label: 'workbenchGit.staged', source: 'index', files: review.files.filter(file => !conflicts.has(file.path) && file.index.trim() !== '' && file.index !== '?') },
  ];
  return <div className="dev-git-change-tree">{!review.files.length ? <p className="m-0 text-xs text-ink-secondary">{t('workbenchGit.noChanges')}</p> : null}{groups.map(group => <details key={group.label} open className="dev-git-change-group"><summary>{t(group.label)} <span>{group.files.length}</span></summary><GitDirectory files={group.files} prefix="" source={group.source} selected={selected} onSelect={onSelect} onOpenDiff={onOpenDiff} /></details>)}</div>;
}
function GitDirectory({ files, prefix, source, selected, onSelect, onOpenDiff }: { files: GitFile[]; prefix: string; source: 'index' | 'worktree'; selected: string[]; onSelect: (path: string, checked: boolean) => void; onOpenDiff?: (path: string, source?: 'index' | 'worktree') => void }) {
  const { t } = useLocale();
  const directories = new Set<string>();
  const leaves: GitFile[] = [];
  for (const file of files) {
    const rest = file.path.slice(prefix.length);
    if (rest.includes('/')) directories.add(rest.split('/')[0]); else leaves.push(file);
  }
  return <div className="dev-git-tree-directory">{[...directories].sort().map(name => <details key={name} open><summary>{name}</summary><GitDirectory files={files.filter(file => file.path.startsWith(`${prefix}${name}/`))} prefix={`${prefix}${name}/`} source={source} selected={selected} onSelect={onSelect} onOpenDiff={onOpenDiff} /></details>)}{leaves.sort((a, b) => a.path.localeCompare(b.path)).map(file => <div key={file.path} className="dev-git-tree-file"><label className="dev-checkbox"><input type="checkbox" aria-label={source === 'index' && file.worktree.trim() !== '' ? `${file.path} (${t('workbenchGit.staged')})` : file.path} checked={selected.includes(file.path)} onChange={event => onSelect(file.path, event.target.checked)} />{onOpenDiff ? null : file.path.slice(prefix.length)}</label>{onOpenDiff ? <Button size="xs" variant="ghost" title={t('workbenchGit.viewChanges')} onClick={() => onOpenDiff(file.path, source)}>{file.path.slice(prefix.length)}</Button> : null}<span className="dev-git-status" title={t(source === 'index' ? 'delivery.indexStatus' : 'delivery.worktreeStatus')}>{source === 'index' ? file.index : file.worktree}</span>{file.original_path ? <span className="text-xs text-ink-secondary">{file.original_path}</span> : null}</div>)}</div>;
}
