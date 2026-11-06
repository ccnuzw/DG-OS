import type { FormEventHandler, KeyboardEvent, ReactNode } from 'react';
import { useLocale } from '../../i18n';
import type { ResourceViewState } from '../../resourceState';
import { cn } from '../lib/cn';
import { Button } from './Button';
import { Card } from './Card';
import { Input } from './Input';
import { Loading } from './Loading';
import { Spinner } from './Spinner';
import { StatusIcon, type StatusIconName } from './StatusIcon';
import './patterns.css';

export interface SectionHeaderProps {
  title: ReactNode;
  description?: ReactNode;
  eyebrow?: ReactNode;
  actions?: ReactNode;
  as?: 'h1' | 'h2' | 'h3' | 'h4';
  className?: string;
}

export function SectionHeader({ title, description, eyebrow, actions, as: Heading = 'h2', className }: SectionHeaderProps) {
  return (
    <header className={cn('flex min-w-0 flex-wrap items-start justify-between gap-3', className)}>
      <div className="min-w-0 max-w-prose">
        {eyebrow ? <p className="mb-1 break-words text-xs font-medium text-accent-default">{eyebrow}</p> : null}
        <Heading className="break-words text-lg font-semibold leading-tight text-ink-primary">{title}</Heading>
        {description ? <p className="mt-1 break-words text-sm leading-relaxed text-ink-secondary">{description}</p> : null}
      </div>
      {actions ? <div className="flex min-w-0 max-w-full flex-wrap items-center gap-2">{actions}</div> : null}
    </header>
  );
}

export type StatusBadgeStatus = 'success' | 'warning' | 'error' | 'info' | 'neutral' | 'loading';

const STATUS_ICONS: Partial<Record<Exclude<StatusBadgeStatus, 'neutral' | 'loading'>, StatusIconName>> = {
  success: 'success',
  warning: 'warning',
  error: 'error',
  info: 'info',
};

const STATUS_CLASSES: Record<StatusBadgeStatus, string> = {
  success: 'border-status-success-surface bg-status-success-surface text-status-success-text',
  warning: 'border-status-warning-surface bg-status-warning-surface text-status-warning-text',
  error: 'border-status-error-surface bg-status-error-surface text-status-error-text',
  info: 'border-status-info-surface bg-status-info-surface text-status-info-text',
  neutral: 'border-border-subtle bg-surface-inset text-ink-secondary',
  loading: 'border-border-subtle bg-surface-inset text-ink-secondary',
};

export interface StatusBadgeProps {
  status: StatusBadgeStatus;
  label: ReactNode;
  icon?: ReactNode;
  /** Keeps dense chrome compact while retaining the label as an accessible name. */
  compact?: boolean;
  className?: string;
}

export function StatusBadge({ status, label, icon, compact = false, className }: StatusBadgeProps) {
  const iconName = STATUS_ICONS[status as keyof typeof STATUS_ICONS];
  return (
    <span
      role={status === 'loading' ? 'status' : undefined}
      aria-label={compact ? String(label) : undefined}
      className={cn('inline-flex max-w-full min-w-0 items-center gap-1.5 rounded-full border px-2 py-0.5 text-xs leading-normal', STATUS_CLASSES[status], className)}
    >
      {icon ?? (status === 'loading' ? <Spinner className="size-3" /> : iconName ? <StatusIcon name={iconName} className="size-3.5" /> : null)}
      <span className={cn('min-w-0 break-words', compact && 'sr-only')}>{label}</span>
    </span>
  );
}

export type NoticeTone = 'success' | 'warning' | 'error' | 'info';

const NOTICE_CLASSES: Record<NoticeTone, string> = {
  success: 'border-status-success bg-status-success-surface text-status-success-text',
  warning: 'border-status-warning bg-status-warning-surface text-status-warning-text',
  error: 'border-status-error bg-status-error-surface text-status-error-text',
  info: 'border-status-info bg-status-info-surface text-status-info-text',
};

export interface NoticeProps {
  tone: NoticeTone;
  title?: ReactNode;
  children: ReactNode;
  action?: ReactNode;
  className?: string;
}

export function Notice({ tone, title, children, action, className }: NoticeProps) {
  return (
    <div role={tone === 'error' ? 'alert' : 'status'} className={cn('flex min-w-0 flex-wrap items-start gap-3 rounded-md border p-3 text-sm', NOTICE_CLASSES[tone], className)}>
      <StatusIcon name={tone} className="mt-0.5 size-4 shrink-0" />
      <div className="min-w-0 flex-1 break-words leading-relaxed">
        {title ? <p className="font-medium">{title}</p> : null}
        <div className={cn(title && 'mt-0.5')}>{children}</div>
      </div>
      {action ? <div className="ml-auto flex max-w-full shrink-0 flex-wrap items-center gap-2">{action}</div> : null}
    </div>
  );
}

export interface EmptyStateProps {
  title: ReactNode;
  description?: ReactNode;
  action?: ReactNode;
  icon?: ReactNode;
  className?: string;
}

export function EmptyState({ title, description, action, icon, className }: EmptyStateProps) {
  return (
    <div className={cn('flex min-w-0 flex-col items-center justify-center gap-2 rounded-lg border border-dashed border-border-subtle bg-surface-inset p-6 text-center', className)}>
      {icon ? <div className="text-ink-tertiary">{icon}</div> : null}
      <p className="max-w-prose break-words font-medium text-ink-primary">{title}</p>
      {description ? <p className="max-w-prose break-words text-sm leading-relaxed text-ink-secondary">{description}</p> : null}
      {action ? <div className="mt-2 flex max-w-full flex-wrap justify-center gap-2">{action}</div> : null}
    </div>
  );
}

export interface SearchFieldProps {
  value: string;
  onChange: (value: string) => void;
  label: ReactNode;
  placeholder?: string;
  disabled?: boolean;
  readOnly?: boolean;
  onClear?: () => void;
  clearLabel?: string;
  className?: string;
}

export function SearchField({ value, onChange, label, placeholder, disabled, readOnly, onClear, clearLabel, className }: SearchFieldProps) {
  const { t } = useLocale();
  const resolvedClearLabel = clearLabel ?? t('patterns.clearSearch');
  return (
    <div className={cn('flex min-w-0 items-end gap-2', className)}>
      <div className="min-w-0 flex-1">
        <Input
          type="search"
          label={label}
          value={value}
          placeholder={placeholder}
          disabled={disabled}
          readOnly={readOnly}
          onChange={(event) => onChange(event.currentTarget.value)}
        />
      </div>
      {onClear ? (
        <Button type="button" variant="ghost" size="xs" onClick={onClear} disabled={disabled || readOnly || value.length === 0} aria-label={resolvedClearLabel}>
          {resolvedClearLabel}
        </Button>
      ) : null}
    </div>
  );
}

export interface FilterBarProps {
  children: ReactNode;
  label?: string;
  actions?: ReactNode;
  onSubmit?: FormEventHandler<HTMLFormElement>;
  className?: string;
}

export function FilterBar({ children, label, actions, onSubmit, className }: FilterBarProps) {
  return (
    <form onSubmit={onSubmit} className={cn('flex min-w-0 flex-wrap items-end gap-3 py-2', className)}>
      <div role="group" aria-label={label} className="flex min-w-0 flex-1 flex-wrap items-end gap-3">
        {children}
      </div>
      {actions ? <div className="flex max-w-full shrink-0 flex-wrap items-center gap-2">{actions}</div> : null}
    </form>
  );
}

export interface ResourceLayoutProps {
  list: ReactNode;
  detail?: ReactNode;
  listLabel?: string;
  detailLabel?: string;
  className?: string;
}

export function ResourceLayout({ list, detail, listLabel, detailLabel, className }: ResourceLayoutProps) {
  return (
    <div className={cn('resource-layout-grid min-w-0 max-w-full gap-4 overflow-hidden', className)}>
      <section aria-label={listLabel} className="min-w-0 overflow-hidden">{list}</section>
      {detail ? <section aria-label={detailLabel} className="min-w-0 overflow-hidden">{detail}</section> : null}
    </div>
  );
}

type ResourceStateProps = {
  state: ResourceViewState;
  loadingLabel?: ReactNode;
  emptyContent?: ReactNode;
  errorContent?: ReactNode;
  retry?: ReactNode;
};

function ResourceState({ state, loadingLabel, emptyContent, errorContent, retry, children }: ResourceStateProps & { children?: ReactNode }) {
  const { t } = useLocale();
  const refresh = state.status === 'ready' ? state.refresh : undefined;
  const error = state.status === 'error' ? state.error
    : refresh?.status === 'error' || refresh?.status === 'refreshing' ? refresh.error : undefined;
  return (
    <>
      <>
        {state.status === 'idle' ? <Notice tone="info" action={retry}>{t('patterns.notChecked')}</Notice> : null}
        {state.status === 'loading' ? <Loading label={loadingLabel ?? t('patterns.loading')} className="p-4" /> : null}
        {refresh?.status === 'refreshing' ? <Notice tone="info">{t('patterns.refreshing')} {t('patterns.showingPrevious')}</Notice> : null}
        {refresh?.status === 'stale' ? <Notice tone="warning" action={retry}>{t('patterns.showingPrevious')} {t('patterns.notChecked')}</Notice> : null}
        {error !== undefined ? (
          <Notice tone="error" title={state.status === 'ready' ? t('patterns.refreshFailed') : t('patterns.loadFailed')} action={retry}>
            {errorContent ?? error}
            {state.status === 'ready' ? <p>{t('patterns.showingPrevious')}</p> : null}
          </Notice>
        ) : null}
      </>
      {state.status === 'ready' ? state.empty ? emptyContent ?? <EmptyState title={t('patterns.empty')} /> : children : null}
    </>
  );
}

export interface ResourceListProps extends ResourceStateProps {
  children?: ReactNode;
  label?: string;
  className?: string;
}

export function ResourceList({ children, label, className, ...state }: ResourceListProps) {
  return (
    <div aria-label={label} className={cn('flex min-w-0 flex-col gap-2 overflow-hidden', className)}>
      <ResourceState {...state}>{children}</ResourceState>
    </div>
  );
}

export interface ResourceDetailProps extends ResourceStateProps {
  children?: ReactNode;
  label?: string;
  readOnly?: boolean;
  className?: string;
}

export function ResourceDetail({ children, label, readOnly = false, className, ...state }: ResourceDetailProps) {
  return (
    <div aria-label={label} data-read-only={readOnly || undefined} className={cn('min-w-0 overflow-hidden rounded-md bg-surface-panel p-3', className)}>
      <ResourceState {...state}>{children}</ResourceState>
    </div>
  );
}

export interface ResourceCardProps {
  children?: ReactNode;
  title?: ReactNode;
  description?: ReactNode;
  status?: ReactNode;
  actions?: ReactNode;
  onSelect?: () => void;
  disabled?: boolean;
  readOnly?: boolean;
  className?: string;
}

export function ResourceCard({ children, title, description, status, actions, onSelect, disabled = false, readOnly = false, className }: ResourceCardProps) {
  const interactive = Boolean(onSelect) && !disabled && !readOnly;
  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!interactive || !onSelect) return;
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      onSelect();
    }
  };
  return (
    <Card
      role={onSelect ? 'button' : undefined}
      tabIndex={interactive ? 0 : undefined}
      aria-disabled={onSelect && (disabled || readOnly) ? true : undefined}
      data-read-only={readOnly || undefined}
      onClick={interactive ? onSelect : undefined}
      onKeyDown={handleKeyDown}
      className={cn('min-w-0 overflow-hidden', interactive && 'cursor-pointer transition-colors hover:border-accent-default focus-visible:border-accent-default focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-border-focus', disabled && 'opacity-disabled', className)}
    >
      {title || status || actions ? (
        <Card.Header className="resource-card-header grid min-w-0 items-start gap-2 border-0 px-3 py-2">
          {title ? <p className="min-w-0 break-words text-sm font-medium">{title}</p> : null}
          {status ? <div className="max-w-full">{status}</div> : null}
          {description ? <p className="col-span-2 break-words text-sm font-normal leading-relaxed text-ink-secondary">{description}</p> : null}
          {actions ? <div className="col-span-2 flex max-w-full flex-wrap gap-2" onClick={(event) => event.stopPropagation()} onKeyDown={(event) => event.stopPropagation()}>{actions}</div> : null}
        </Card.Header>
      ) : null}
      {children ? <Card.Body className="min-w-0 break-words px-3 py-2">{children}</Card.Body> : null}
    </Card>
  );
}

export interface FixedActionBarProps {
  children: ReactNode;
  status?: ReactNode;
  label?: string;
  className?: string;
}

export function FixedActionBar({ children, status, label, className }: FixedActionBarProps) {
  return (
    <div aria-label={label} className={cn('sticky bottom-0 z-sticky flex min-w-0 flex-wrap items-center justify-between gap-3 border-t border-border-subtle bg-surface-panel px-4 py-3', className)}>
      <div className="min-w-0 flex-1 break-words text-sm text-ink-secondary">{status}</div>
      <div className="flex max-w-full shrink-0 flex-wrap items-center justify-end gap-2">{children}</div>
    </div>
  );
}
