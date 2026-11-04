import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button, Input } from '@dgos/ui';
import { useLocale } from '../../../i18n';

const categories = ['old_chats', 'session_assets', 'temp_files', 'cache', 'activity_records'] as const;
type Category = typeof categories[number];
const cutoffCategories: ReadonlyArray<Category> = ['old_chats', 'activity_records'];

interface CategoryStat { category: Category; status: 'ok' | 'empty' | 'not_applicable'; count: number; bytes: number | null }
interface Overview { categories: CategoryStat[] }
interface Preview { preview_id: string; category: Category; cutoff: number; expires_at: number; count: number; bytes: number | null }
interface CleanupResult { category: Category; status: 'done' | 'partial' | 'failed'; deleted: number; failed: number }
interface ExecuteResult { result: CleanupResult; overview: Overview }
interface VacuumPreview { preview_id: string; expires_at: number }
interface VacuumResult { bytes_before: number; bytes_after: number }

interface Response<T> { success: boolean; data?: T; error?: { code: number; message: string } }

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const response = await invoke<Response<T>>(command, args);
  if (!response.success || response.data === undefined) throw new Error(response.error?.message ?? 'storage command failed');
  return response.data;
}

export default function StorageSettings() {
  const { t } = useLocale();
  // `null` means "not read yet or read failed". It is rendered as an em dash, never as a zero count.
  const [overview, setOverview] = useState<Overview | null>(null);
  const [cutoffs, setCutoffs] = useState<Partial<Record<Category, string>>>({});
  const [preview, setPreview] = useState<Preview | null>(null);
  const [result, setResult] = useState<CleanupResult | null>(null);
  const [vacuumPreview, setVacuumPreview] = useState<VacuumPreview | null>(null);
  const [vacuumResult, setVacuumResult] = useState<VacuumResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(true);
  const ticket = useRef(0);

  // Each action takes a ticket. Only the newest live ticket may write state, so an old response never
  // overwrites a newer preview or result. Account switching reloads the page, which drops every ticket.
  const begin = () => { ticket.current += 1; setBusy(true); setError(null); return ticket.current; };
  const current = (id: number) => alive.current && id === ticket.current;

  const failureMessage = (failure: unknown) => {
    const message = failure instanceof Error ? failure.message : '';
    if (message === 'storage preview expired') return t('storage.preview.expired');
    if (message === 'storage preview not found or belongs to another account') return t('storage.preview.stale');
    if (message === 'storage maintenance is busy; retry later') return t('storage.busy');
    return t('storage.error');
  };

  const refreshAfterAction = async (id: number) => {
    if (current(id)) setOverview(null);
    try {
      const value = await call<Overview>('get_storage_overview');
      if (current(id)) setOverview(value);
    } catch (failure) {
      if (current(id)) setError(previous => previous ?? failureMessage(failure));
    }
  };

  const load = async () => {
    const id = begin();
    setPreview(null);
    try {
      const value = await call<Overview>('get_storage_overview');
      if (current(id)) setOverview(value);
    } catch (failure) {
      if (current(id)) { setOverview(null); setError(failureMessage(failure)); }
    } finally {
      if (current(id)) setBusy(false);
    }
  };

  useEffect(() => {
    alive.current = true;
    void load();
    return () => { alive.current = false; ticket.current += 1; };
  }, []);

  const requestPreview = async (category: Category) => {
    const id = begin();
    setPreview(null);
    setResult(null);
    const raw = cutoffs[category];
    const cutoffDays = raw && cutoffCategories.includes(category) ? Number(raw) : undefined;
    try {
      const value = await call<Preview>('preview_storage_cleanup', { input: { category, cutoffDays } });
      if (current(id)) setPreview(value);
    } catch (failure) {
      if (current(id)) {
        setError(failureMessage(failure));
        await refreshAfterAction(id);
      }
    } finally {
      if (current(id)) setBusy(false);
    }
  };

  const confirm = async () => {
    if (!preview) return;
    const id = begin();
    try {
      const value = await call<ExecuteResult>('execute_storage_cleanup', { input: { previewId: preview.preview_id } });
      if (!current(id)) return;
      setResult(value.result);
      setOverview(value.overview);
      setPreview(null);
    } catch (failure) {
      if (current(id)) {
        setError(failureMessage(failure));
        setPreview(null);
        await refreshAfterAction(id);
      }
    } finally {
      if (current(id)) setBusy(false);
    }
  };

  const requestVacuum = async () => {
    const id = begin();
    setVacuumResult(null);
    setVacuumPreview(null);
    try {
      const value = await call<VacuumPreview>('preview_vacuum');
      if (current(id)) setVacuumPreview(value);
    } catch (failure) {
      if (current(id)) {
        setError(failureMessage(failure));
        await refreshAfterAction(id);
      }
    } finally {
      if (current(id)) setBusy(false);
    }
  };

  const confirmVacuum = async () => {
    if (!vacuumPreview) return;
    const id = begin();
    setVacuumResult(null);
    try {
      const value = await call<VacuumResult>('execute_vacuum', { input: { previewId: vacuumPreview.preview_id, confirmed: true } });
      if (current(id)) setVacuumResult(value);
    } catch (failure) {
      if (current(id)) setError(failureMessage(failure));
    } finally {
      if (current(id)) {
        setVacuumPreview(null);
        await refreshAfterAction(id);
        if (current(id)) setBusy(false);
      }
    }
  };

  const formatCount = (category: Category) => {
    const stat = overview?.categories.find(entry => entry.category === category);
    return stat ? String(stat.count) : '—';
  };

  const formatBytes = (category: Category) => {
    const stat = overview?.categories.find(entry => entry.category === category);
    return stat?.bytes == null ? '—' : String(stat.bytes);
  };

  const statusText = (category: Category) => {
    const stat = overview?.categories.find(entry => entry.category === category);
    return stat ? t(`storage.status.${stat.status}`) : '—';
  };

  const resultText = (value: CleanupResult) => {
    if (value.status === 'done') return t('storage.result.done', { deleted: value.deleted });
    if (value.status === 'partial') return t('storage.result.partial', { deleted: value.deleted, failed: value.failed });
    return t('storage.result.failed', { failed: value.failed });
  };

  return (
    <div className="settings-content settings-sections storage-settings">
      <h1 className="m-0 text-lg font-semibold">{t('storage.title')}</h1>
      <p className="text-sm text-muted-foreground">{t('storage.help')}</p>
      {busy && <p className="text-sm text-muted-foreground">{t('storage.loading')}</p>}
      {error && <p role="alert" className="text-sm text-error-text">{error}</p>}
      <Button type="button" onClick={() => void load()} disabled={busy}>{t('storage.refresh')}</Button>
      <ul className="m-0 flex list-none flex-col gap-3 p-0">
        {categories.map(category => {
          const empty = formatCount(category) === '0' && !cutoffCategories.includes(category);
          return (
            <li key={category} className="storage-cleanup-row">
              <strong className="text-sm">{t(`storage.category.${category}`)}</strong>
              <span className="text-xs text-muted-foreground">{t(`storage.help.${category}`)}</span>
              <span className="text-sm">
                {t('storage.count')}: {formatCount(category)} · {t('storage.bytes')}: {formatBytes(category)} · {statusText(category)}
              </span>
              {cutoffCategories.includes(category) && (
                <Input
                  label={t('storage.cutoff')}
                  type="number"
                  min={1}
                  step={1}
                  disabled={busy}
                  value={cutoffs[category] ?? ''}
                  placeholder={String(category === 'old_chats' ? 30 : 90)}
                  onChange={event => setCutoffs(previous => ({ ...previous, [category]: event.target.value }))}
                />
              )}
              <Button type="button" onClick={() => void requestPreview(category)} disabled={busy || overview === null || empty || statusText(category) === t('storage.status.not_applicable')}>
                {t('storage.preview')}
              </Button>
            </li>
          );
        })}
      </ul>
      {preview && (
        <section className="flex flex-col gap-2">
          <p className="text-sm">
            {t('storage.preview.summary', { count: preview.count, bytes: preview.bytes ?? '—', expires: new Date(preview.expires_at * 1000).toLocaleString() })}
          </p>
          <div className="flex gap-2">
            <Button type="button" onClick={() => void confirm()} disabled={busy}>{t('storage.confirm')}</Button>
            <Button type="button" onClick={() => setPreview(null)} disabled={busy}>{t('storage.cancel')}</Button>
          </div>
        </section>
      )}
      {result && <p role="status" className="text-sm">{resultText(result)}</p>}
      <section className="flex flex-col gap-2">
        <strong className="text-sm">{t('storage.vacuum.title')}</strong>
        <span className="text-xs text-muted-foreground">{t('storage.vacuum.help')}</span>
        <Button type="button" onClick={() => void requestVacuum()} disabled={busy}>{t('storage.vacuum.preview')}</Button>
        {vacuumPreview && (
          <div className="flex flex-col gap-2">
            <p className="text-sm">{t('storage.vacuum.summary', { expires: new Date(vacuumPreview.expires_at * 1000).toLocaleString() })}</p>
            <div className="flex gap-2">
              <Button type="button" onClick={() => void confirmVacuum()} disabled={busy}>{t('storage.vacuum.confirm')}</Button>
              <Button type="button" onClick={() => setVacuumPreview(null)} disabled={busy}>{t('storage.cancel')}</Button>
            </div>
          </div>
        )}
        {vacuumResult && (
          <p role="status" className="text-sm">
            {t('storage.vacuum.result', { before: vacuumResult.bytes_before, after: vacuumResult.bytes_after })}
          </p>
        )}
      </section>
    </div>
  );
}
