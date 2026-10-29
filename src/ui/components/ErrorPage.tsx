import { Component } from 'react';
import type { ErrorInfo, ReactNode } from 'react';
import { Button } from './Button';
import { StatusIcon } from './StatusIcon';

// Whole-page error state (FR-005 §8.1.4, B05-36, AC19). It knows nothing about the router: the caller passes what
// the buttons do. It never shows an error's message or stack, only fixed text.

export type ErrorPageKind = 'not-found' | 'error';

const COPY: Record<ErrorPageKind, { code?: string; title: string; message: string; icon: 'warning' | 'error' }> = {
  'not-found': {
    code: '404',
    title: '页面不存在',
    message: '您访问的页面不存在或已被删除。',
    icon: 'warning',
  },
  error: {
    title: '出了点问题',
    message: '页面遇到了意外错误，请重试或返回首页。',
    icon: 'error',
  },
};

export interface ErrorPageProps {
  kind?: ErrorPageKind;
  /** Shows the "返回首页" button. */
  onHome?: () => void;
  /** Shows the "重试" button. */
  onRetry?: () => void;
}

export function ErrorPage({ kind = 'not-found', onHome, onRetry }: ErrorPageProps) {
  const copy = COPY[kind];
  return (
    <section
      data-error-kind={kind}
      className="flex min-h-full flex-col items-center justify-center gap-4 p-10 text-center text-foreground"
    >
      <StatusIcon name={copy.icon} className={`size-16 ${copy.icon === 'error' ? 'text-error-text' : 'text-warning-text'}`} />
      <h1 className="text-2xl font-semibold">{copy.code ? `${copy.code} ${copy.title}` : copy.title}</h1>
      <p className="text-base text-muted-foreground">{copy.message}</p>
      <div className="flex gap-2">
        {onRetry ? (
          <Button variant="secondary" onClick={onRetry}>
            重试
          </Button>
        ) : null}
        {onHome ? <Button onClick={onHome}>返回首页</Button> : null}
      </div>
    </section>
  );
}

interface ErrorBoundaryProps {
  children: ReactNode;
  /** Defaults to a full navigation to `/`: the boundary sits outside the router, so it cannot use its hooks. */
  onHome?: () => void;
}

/** Catches render errors below it and shows `ErrorPage` in its `error` form (AC19). */
export class ErrorBoundary extends Component<ErrorBoundaryProps, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  componentDidCatch(_error: unknown, _info: ErrorInfo) {
    // The error object can hold request content or credentials, so only its location goes to the log.
    console.error('[dgos] error', { code: null, where: 'render' });
  }

  render() {
    if (!this.state.failed) return this.props.children;
    return (
      <ErrorPage
        kind="error"
        onRetry={() => this.setState({ failed: false })}
        onHome={this.props.onHome ?? (() => window.location.assign('/'))}
      />
    );
  }
}
