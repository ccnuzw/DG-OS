import { useCallback, useEffect, useRef, useState } from 'react';

// Copy to the system clipboard (D077): navigator.clipboard.writeText first, document.execCommand('copy') as the
// fallback. The caller must show the outcome; a failure is never silent.

function execCommandCopy(text: string): boolean {
  if (typeof document.execCommand !== 'function') return false;
  const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const area = document.createElement('textarea');
  area.value = text;
  area.setAttribute('readonly', '');
  area.setAttribute('aria-hidden', 'true');
  area.className = 'sr-only';
  document.body.appendChild(area);
  try {
    area.focus();
    area.select();
    return document.execCommand('copy');
  } catch {
    return false;
  } finally {
    area.remove();
    previous?.focus();
  }
}

export async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard && typeof navigator.clipboard.writeText === 'function') {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // Fall through to the legacy path.
  }
  return execCommandCopy(text);
}

export type CopyState = 'idle' | 'copied' | 'failed';
export const COPY_FEEDBACK_MS = 2500;
export const COPY_TEXT: Readonly<Record<CopyState, string>> = {
  idle: '',
  copied: '已复制',
  failed: '复制失败，请手动选择文本复制',
};

/** Copy with visible, announced feedback that resets after a short time. */
export function useCopyFeedback(): [CopyState, (text: string) => void] {
  const [state, setState] = useState<CopyState>('idle');
  const timer = useRef<number | undefined>(undefined);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      clearTimeout(timer.current);
    };
  }, []);
  const copy = useCallback((text: string) => {
    void copyText(text).then(ok => {
      if (!mounted.current) return;
      setState(ok ? 'copied' : 'failed');
      clearTimeout(timer.current);
      timer.current = window.setTimeout(() => setState('idle'), COPY_FEEDBACK_MS);
    });
  }, []);
  return [state, copy];
}
