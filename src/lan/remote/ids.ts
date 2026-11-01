// UUID v4 from crypto.getRandomValues. crypto.randomUUID is only defined in secure contexts, and the remote SPA is
// served over plain http://192.168.x.x, so it is undefined there.

export function uuidV4(): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

/** Access codes are 26 Crockford Base32 characters; '-' and whitespace are ignored, case does not matter. */
export const ACCESS_CODE_LENGTH = 26;

export function normalizeAccessCode(raw: string): string {
  return raw.replace(/[-\s]/g, '').toUpperCase();
}
