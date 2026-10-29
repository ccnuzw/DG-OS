// Unified error handling (FR-005 §8.3, B05-27, B05-28, B05-29).
//
// The host's own message text, any request content and stack traces never reach the screen or the log: the
// screen shows text from ERROR_MESSAGES or the generic fallback, the log carries the code and where it happened.
import { modalApi } from '../feedback/modalStore';
import { toast } from '../feedback/toastStore';
import { lookupError } from './messages';

export interface HandleErrorOptions {
  /** Where the error happened, for the log line (a screen or command name, never user input). */
  where?: string;
  /** Opens a route. Without it no action button is shown. */
  navigate?: (route: string) => void;
  /** Whether a route exists in the application's route table. Without it no action button is shown. */
  routeExists?: (route: string) => boolean;
}

const DURATION: Record<'minor' | 'moderate', number> = { minor: 3000, moderate: 5000 };

/** The numeric `code` of a host error, or `null` when the value has none (a string, `undefined`, a thrown `Error`). */
export function extractErrorCode(error: unknown): number | null {
  if (typeof error !== 'object' || error === null || !('code' in error)) return null;
  const { code } = error as { code: unknown };
  return typeof code === 'number' && Number.isSafeInteger(code) && code >= 0 ? code : null;
}

function logError(code: number | null, where: string | undefined): void {
  console.error('[dgos] error', { code, where });
}

export function handleError(error: unknown, options: HandleErrorOptions = {}): void {
  const code = extractErrorCode(error);
  const info = lookupError(code);
  logError(code, options.where);

  // B05-28: an unregistered code (or none at all) gets the generic text and moderate treatment.
  if (!info) {
    toast.error(code === null ? '操作失败' : `操作失败（错误码 ${code}）`, { duration: DURATION.moderate });
    return;
  }

  // B05-29: a severe error blocks until the user acknowledges it.
  if (info.severity === 'severe') {
    modalApi.error({ title: info.message, content: info.description });
    return;
  }

  const { action } = info;
  const { navigate, routeExists } = options;
  const canNavigate = action && navigate && routeExists?.(action.route);
  toast.error(info.message, {
    description: info.severity === 'moderate' ? info.description : undefined,
    duration: DURATION[info.severity],
    action: canNavigate ? { label: action.label, onClick: () => navigate(action.route) } : undefined,
  });
}
