/** Unknown native requests retain their original generation through renderer restoration. */
export function pendingAgentDraftKeys(entries: Record<string, unknown>, epoch: number, projectId: string | null, sessionId: string, kind: 'pending-send' | 'pending-steer'): string[] {
  return Object.keys(entries).filter(key => {
    try {
      const parts: unknown = JSON.parse(key);
      return Array.isArray(parts) && parts.length === 6 && parts[0] === epoch && parts[1] === 'conversation' && parts[2] === projectId && parts[3] === sessionId && typeof parts[4] === 'string' && parts[5] === kind;
    } catch { return false; }
  });
}
