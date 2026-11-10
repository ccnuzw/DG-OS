import { invoke } from '@tauri-apps/api/core';
import { extractErrorCode } from '@dgos/ui';
import { ChatCommandError } from '../../stores/chatStore';
import type { AiResource } from '../../types/ai';

export interface ResourcePreview {
  base64: string;
  mime: string;
  start: number;
  bytes: number;
  total_bytes: number;
  truncated: boolean;
}

export async function readResourcePreview(sessionId: string, resource: AiResource): Promise<ResourcePreview> {
  let response: { success: boolean; data: ResourcePreview; error?: { code?: number; detail?: string } };
  try {
    response = await invoke('preview_chat_resource', { input: { session_id: sessionId, resource_id: resource.id, sha256: resource.sha256 } });
  } catch (error) { throw new ChatCommandError(extractErrorCode(error)); }
  if (!response?.success) throw new ChatCommandError(response?.error?.code ?? null, response?.error?.detail);
  const preview = response.data;
  if (preview.mime !== resource.mime || preview.total_bytes !== resource.bytes_len || preview.start !== 0 || preview.bytes < 0 || preview.bytes > preview.total_bytes || (!preview.truncated && preview.bytes !== preview.total_bytes)) throw new Error('resource_preview_changed');
  return preview;
}
export interface SelectableResourceText { text: string; offsets: number[] }

/** Map textarea offsets (UTF-16, normalized newlines) back to immutable UTF-8 bytes. */
export function selectableResourceText(source: string): SelectableResourceText {
  const offsets = [0];
  let bytes = 0;
  for (let index = 0; index < source.length;) {
    const code = source.codePointAt(index)!;
    if (code === 13) {
      const crlf = source.charCodeAt(index + 1) === 10;
      bytes += crlf ? 2 : 1;
      index += crlf ? 2 : 1;
      offsets.push(bytes);
      continue;
    }
    const units = code > 0xffff ? 2 : 1;
    if (units === 2) offsets.push(bytes);
    bytes += code <= 0x7f ? 1 : code <= 0x7ff ? 2 : code <= 0xffff ? 3 : 4;
    offsets.push(bytes); index += units;
  }
  return { text: source.replace(/\r\n?/g, '\n'), offsets };
}
export function selectedResourceRange(preview: SelectableResourceText, start: number, end: number) {
  if (start < 0 || end <= start || end > preview.text.length) return null;
  // Browser selections can split a surrogate pair; expand to complete Unicode scalars.
  if (start > 0 && /[\uDC00-\uDFFF]/.test(preview.text[start]) && /[\uD800-\uDBFF]/.test(preview.text[start - 1])) start--;
  if (end < preview.text.length && /[\uDC00-\uDFFF]/.test(preview.text[end]) && /[\uD800-\uDBFF]/.test(preview.text[end - 1])) end++;
  return { start: preview.offsets[start], max_bytes: preview.offsets[end] - preview.offsets[start] };
}
