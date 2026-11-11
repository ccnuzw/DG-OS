import type { ChatRunStep, ChatWebActivity, ChatWebSource } from '../../stores/chatStore';

export function safeSourceUrl(value: string): string | null {
  try {
    const url = new URL(value);
    return url.protocol === 'https:' && !!url.hostname && !url.username && !url.password && (!url.port || url.port === '443') ? url.href : null;
  } catch { return null; }
}

export function webActivity(step: ChatRunStep): ChatWebActivity | null {
  if (step.kind !== 'web') return null;
  const value = step.receipt?.web_activity;
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const activity = value as Record<string, unknown>;
  if (typeof activity.id !== 'string' || !['search', 'fetch'].includes(String(activity.operation)) || !['running', 'completed', 'failed'].includes(String(activity.status)) || !Array.isArray(activity.sources)) return null;
  return { id: activity.id, operation: activity.operation as ChatWebActivity['operation'], status: activity.status as ChatWebActivity['status'], query: typeof activity.query === 'string' ? activity.query : undefined, url: typeof activity.url === 'string' ? activity.url : undefined, error: typeof activity.error === 'string' ? activity.error : undefined, truncated: activity.truncated === true, sources: activity.sources.filter((source): source is ChatWebSource => {
    if (!source || typeof source !== 'object') return false;
    const item = source as Record<string, unknown>;
    return typeof item.id === 'string' && !!item.id && typeof item.url === 'string' && !!safeSourceUrl(item.url) && typeof item.title === 'string' && typeof item.excerpt === 'string' && typeof item.retrieved_at === 'number' && Number.isFinite(item.retrieved_at) && ['search_result', 'web_page'].includes(String(item.kind)) && typeof item.truncated === 'boolean';
  }).slice(0, 16) };
}

/** Only host web receipts create evidence. MCP JSON and model-authored links never do. */
export function trustedSources(steps: ChatRunStep[]): ChatWebSource[] {
  const sources = new Map<string, ChatWebSource>();
  for (const step of steps) for (const source of webActivity(step)?.sources ?? []) sources.set(source.id, source);
  return [...sources.values()].slice(0, 48);
}

export function executionSteps(steps: ChatRunStep[]): ChatRunStep[] {
  const result: ChatRunStep[] = [];
  const webIds = new Map<string, number>();
  for (const step of steps) {
    if (!['tool', 'skill', 'context', 'web'].includes(step.kind)) continue;
    const activity = webActivity(step);
    const previous = activity ? webIds.get(activity.id) : undefined;
    if (previous !== undefined) result[previous] = step;
    else { if (activity) webIds.set(activity.id, result.length); result.push(step); }
  }
  return result;
}

export function operationFailed(step: ChatRunStep): boolean {
  const outcome = step.receipt?.outcome;
  return step.status === 'failed' || webActivity(step)?.status === 'failed' || (step.kind === 'tool' && typeof outcome === 'string' && ['error', 'failed', 'timeout', 'cancelled', 'denied', 'rejected'].includes(outcome));
}
