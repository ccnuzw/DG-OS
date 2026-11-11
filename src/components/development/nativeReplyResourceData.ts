import type { AgentEvent, Json } from '../../stores/agentRunnerStore';
import { safeSourceUrl } from '../chat/sources';
import { itemKey } from './agentTranscript';

export interface AgentReplySource {
  /** Native IDs only. Missing IDs are not invented for citation matching. */
  id?: string;
  url: string;
  title: string;
  excerpt: string;
  retrievedAt?: number;
  kind: 'search_result' | 'web_page';
  origin: 'webSearch';
  eventSeq: number;
  truncated: boolean;
}
const record = (value: Json | undefined): Record<string, Json> => value !== null && typeof value === 'object' && !Array.isArray(value) ? value : {};
const string = (value: Json | undefined, limit = 4096): string => typeof value === 'string' ? value.slice(0, limit) : '';
const privateKey = /^(?:thinking|thinking_delta|redacted_thinking|signature|signature_delta|encrypted_content|encryptedContent)$/;
export const NATIVE_RESOURCE_LIMITS = { items: 48, text: 32 * 1024, fields: 80, depth: 6, imageBytes: 2 * 1024 * 1024, imagePixels: 16 * 1024 * 1024, imageSide: 8192 } as const;

/** Only native search result fields qualify; never inspect assistant prose or MCP JSON. */
export function agentReplySources(events: readonly AgentEvent[]): AgentReplySource[] {
  const names = new Map<string, string>();
  const found = new Map<string, AgentReplySource>();
  for (const event of events) {
    const data = record(event.data);
    if (event.kind !== 'message' || data.type !== 'assistant' || !Array.isArray(data.content)) continue;
    for (const value of data.content) {
      const block = record(value);
      if (block.type === 'tool_use' && typeof block.id === 'string' && block.name === 'WebSearch') names.set(itemKey(event, block.id), block.name);
    }
  }
  const add = (value: Json, event: AgentEvent) => {
    const hit = record(value);
    const url = safeSourceUrl(string(hit.url));
    if (!url) return;
    const excerpt = string(hit.snippet ?? hit.excerpt, 6000);
    const rawExcerpt = hit.snippet ?? hit.excerpt;
    const timestamp = hit.retrieved_at;
    const source: AgentReplySource = {
      ...(typeof hit.id === 'string' && hit.id.length > 0 && hit.id.length <= 256 ? { id: hit.id } : {}),
      url, title: string(hit.title, 512) || new URL(url).hostname, excerpt,
      ...(typeof timestamp === 'number' && Number.isFinite(timestamp) && timestamp > 0 && timestamp <= 8640000000000 ? { retrievedAt: timestamp } : {}),
      kind: hit.kind === 'web_page' ? 'web_page' : 'search_result', origin: 'webSearch', eventSeq: event.seq,
      truncated: hit.truncated === true || typeof rawExcerpt === 'string' && rawExcerpt.length > excerpt.length,
    };
    const key = `${source.id ?? ''}:${url}`;
    if (found.size < NATIVE_RESOURCE_LIMITS.items || found.has(key)) found.set(key, source);
  };
  for (const event of events) {
    const data = record(event.data);
    if (event.kind === 'webSearch' || event.kind === 'item/started' && data.type === 'webSearch') {
      if (Array.isArray(data.results)) for (const hit of data.results.slice(0, 48)) add(hit, event);
    }
    if (event.kind !== 'message' || data.type !== 'user' || !Array.isArray(data.content)) continue;
    const webResult = data.content.some(value => { const block = record(value); return block.type === 'tool_result' && typeof block.tool_use_id === 'string' && names.has(itemKey(event, block.tool_use_id)); });
    if (!webResult) continue;
    const result = record(data.tool_use_result);
    if (!Array.isArray(result.results)) continue;
    // SDK WebSearch strings contain progress/errors, not structured result evidence.
    for (const group of result.results.slice(0, 48)) {
      const item = record(group);
      if (typeof item.tool_use_id !== 'string' || !item.tool_use_id || !Array.isArray(item.content)) continue;
      for (const hit of item.content.slice(0, 48)) add(hit, event);
    }
  }
  return [...found.values()];
}

export interface NativeResource {
  kind: 'text' | 'image' | 'document' | 'resource' | 'resource_link' | 'structured' | 'unsupported';
  title: string;
  mime: string;
  uri?: string;
  path?: string;
  text?: string;
  fields?: Json;
  base64?: string;
  previewId?: string;
  byteLength?: number;
  omitted?: string;
  truncated: boolean;
  originalCharacters?: number;
  nativeType?: string;
}

export function nativeResourcePath(value: string): string | null {
  let path = value;
  if (value.startsWith('file:')) {
    try { const url = new URL(value); if (url.protocol !== 'file:' || url.hostname && url.hostname !== 'localhost' || url.search || url.hash) return null; path = decodeURIComponent(url.pathname); }
    catch { return null; }
  }
  if (!path || path.length > 4096 || /[\u0000-\u001f\\]/.test(path) || /^[a-z][a-z\d+.-]*:/i.test(path) || path.split('/').some(part => part === '..' || part === '.git')) return null;
  return path;
}

/** Bounded display projection. This never interprets arbitrary objects as sources or actions. */
export function nativeResources(value: Json): { resources: NativeResource[]; truncated: boolean } {
  const resources: NativeResource[] = [];
  let truncated = false;
  const push = (resource: NativeResource) => { if (resources.length < NATIVE_RESOURCE_LIMITS.items) resources.push(resource); else truncated = true; };
  const textResource = (text: string, title = '') => push({ kind: 'text', title, mime: 'text/plain', text: text.slice(0, NATIVE_RESOURCE_LIMITS.text), truncated: text.length > NATIVE_RESOURCE_LIMITS.text, originalCharacters: text.length });
  const walk = (input: Json, depth: number) => {
    if (resources.length >= NATIVE_RESOURCE_LIMITS.items) { truncated = true; return; }
    if (depth > NATIVE_RESOURCE_LIMITS.depth) { truncated = true; return; }
    if (typeof input === 'string') { textResource(input); return; }
    if (Array.isArray(input)) { for (const item of input.slice(0, NATIVE_RESOURCE_LIMITS.items)) walk(item, depth + 1); if (input.length > NATIVE_RESOURCE_LIMITS.items) truncated = true; return; }
    if (input === null || typeof input !== 'object') { push({ kind: 'structured', title: '', mime: 'application/json', fields: input, truncated: false }); return; }
    const nativeType = string(input.type, 120);
    const type = ['inputImage', 'input_image', 'localImage', 'image_url'].includes(nativeType) ? 'image' : nativeType;
    if (['thinking', 'redacted_thinking', 'signature', 'reasoning'].includes(type)) return;
    if (type === 'tool_result') { if (input.content !== undefined) walk(input.content, depth + 1); if (input.structuredContent !== undefined) push({ kind: 'structured', title: '', mime: 'application/json', fields: input.structuredContent, truncated: false }); return; }
    if (type === 'task_notification' && Array.isArray(input.resource_links)) {
      for (const link of input.resource_links.slice(0, NATIVE_RESOURCE_LIMITS.items)) walk({ ...record(link), type: 'resource_link' }, depth + 1);
      if (input.resource_links.length > NATIVE_RESOURCE_LIMITS.items) truncated = true;
      return;
    }
    if (type === 'mcpToolCall' && input.result !== undefined) { walk(input.result, depth + 1); return; }
    if (type === 'dynamicToolCall' && input.contentItems !== undefined) { walk(input.contentItems, depth + 1); return; }
    if (type === 'functionCallOutput' && input.output !== undefined) { walk(input.output, depth + 1); return; }
    const content = record(input.resource);
    const source = record(input.source);
    const generatedImage = type === 'imageGeneration' ? record(input.result) : undefined;
    const payload = type === 'resource' ? content : generatedImage?.type === 'image' ? generatedImage : Object.keys(source).length ? source : input;
    const mime = string(payload.mimeType ?? payload.media_type ?? input.mimeType ?? input.mime_type, 128).toLowerCase().split(';')[0].trim();
    const rawUri = payload.uri ?? input.uri ?? source.url ?? input.url;
    const rawPath = input.path ?? input.savedPath;
    const unavailableLocation = typeof rawUri === 'string' && (rawUri.length > 4096 || /^(?:data|blob):/i.test(rawUri)) || typeof rawPath === 'string' && rawPath.length > 4096;
    const uri = typeof rawUri === 'string' && rawUri.length <= 4096 && !/^(?:data|blob):/i.test(rawUri) ? rawUri : '';
    const path = typeof rawPath === 'string' && rawPath.length <= 4096 ? rawPath : '';
    const title = string(input.title ?? input.name, 512);
    const rawText = payload.text ?? input.text ?? (source.type === 'text' ? source.data : type === 'resource_link' ? input.description : undefined);
    const rawBase64 = payload.blob ?? (source.type !== 'text' ? payload.data ?? input.data : undefined);
    const byteLength = payload.bytes_len ?? payload.byteLength ?? payload.byte_length ?? input.bytes_len ?? input.byteLength ?? input.byte_length ?? input.size;
    const omitted = string(payload.unavailable_reason ?? input.unavailable_reason, 512) || (unavailableLocation ? 'unsupported_resource_location' : '');
    const previewId = string(payload.preview_resource_id ?? input.preview_resource_id, 128);
    if (type === 'text' && typeof rawText === 'string') { textResource(rawText, title); return; }
    if (['image', 'document', 'resource', 'resource_link', 'imageGeneration', 'imageView'].includes(type)) {
      const kind = type === 'imageGeneration' || type === 'imageView' ? 'image' : type as NativeResource['kind'];
      const nestedDocument = type === 'document' && source.type === 'content' && source.content !== undefined;
      push({ kind, title, mime, ...(uri ? { uri } : {}), ...(path ? { path } : {}), ...(nestedDocument ? { omitted: 'document_content_shown_below' } : {}),
        ...(typeof rawText === 'string' ? { text: rawText.slice(0, NATIVE_RESOURCE_LIMITS.text), originalCharacters: rawText.length } : {}),
        ...(typeof rawBase64 === 'string' ? { base64: rawBase64 } : {}),
        ...(previewId ? { previewId } : {}),
        ...(typeof byteLength === 'number' && Number.isSafeInteger(byteLength) && byteLength >= 0 ? { byteLength } : typeof rawBase64 === 'string' ? { byteLength: Math.floor(rawBase64.length * 3 / 4) - (rawBase64.endsWith('==') ? 2 : rawBase64.endsWith('=') ? 1 : 0) } : {}),
        ...(omitted ? { omitted } : {}), truncated: input.truncated === true || payload.truncated === true || typeof rawText === 'string' && rawText.length > NATIVE_RESOURCE_LIMITS.text });
      if (nestedDocument && source.content !== undefined) walk(source.content, depth + 1);
      return;
    }
    if (input.content !== undefined || input.structuredContent !== undefined) {
      if (input.content !== undefined) walk(input.content, depth + 1);
      if (input.structuredContent !== undefined) push({ kind: 'structured', title, mime: 'application/json', fields: input.structuredContent, truncated: input.truncated === true });
      return;
    }
    push({ kind: type ? 'unsupported' : 'structured', title, mime: 'application/json', fields: input, nativeType: type || undefined, truncated: false });
  };
  walk(value, 0);
  return { resources, truncated };
}

export interface BoundedFields { value: Json; truncated: boolean }
export function boundedResourceFields(value: Json): BoundedFields {
  let remaining = NATIVE_RESOURCE_LIMITS.fields as number;
  let characters = NATIVE_RESOURCE_LIMITS.text as number;
  let truncated = false;
  const visit = (input: Json, depth: number, key = ''): Json => {
    if (--remaining < 0 || depth > NATIVE_RESOURCE_LIMITS.depth || characters <= 0) { truncated = true; return null; }
    if (typeof input === 'string') {
      // Binary data stays in explicit resource previews, never in the field inspector.
      if (/^(?:data|blob|base64|bytes|image_data)$/i.test(key) || /^data:[^,]{0,128};base64,/.test(input) || input.length > 256 && /^[A-Za-z0-9+/=\r\n]+$/.test(input.slice(0, 512))) { truncated = true; return '[binary content omitted]'; }
      const text = input.slice(0, Math.min(4096, characters)); characters -= text.length; if (text.length < input.length) truncated = true; return text;
    }
    if (Array.isArray(input)) { const result: Json[] = []; for (const item of input) { if (remaining <= 0 || characters <= 0) { truncated = true; break; } result.push(visit(item, depth + 1)); } return result; }
    if (input !== null && typeof input === 'object') {
      if (typeof input.type === 'string' && ['thinking', 'redacted_thinking', 'signature', 'reasoning'].includes(input.type)) { truncated = true; return null; }
      const result: Record<string, Json> = Object.create(null);
      for (const name in input) {
        if (!Object.prototype.hasOwnProperty.call(input, name) || privateKey.test(name)) continue;
        if (remaining <= 0 || characters <= 0) { truncated = true; break; }
        result[name.slice(0, 160)] = visit(input[name], depth + 1, name);
      }
      return result;
    }
    return input;
  };
  return { value: visit(value, 0), truncated };
}

export type NativeImageFailure = 'unsupportedMime' | 'imageTooLarge' | 'invalidBase64' | 'mimeMismatch' | 'invalidDimensions';
export class NativeImageError extends Error { constructor(public readonly reason: NativeImageFailure) { super(reason); } }
export function decodeNativeImage(base64: string, mime: string): { bytes: Uint8Array<ArrayBuffer>; width: number; height: number } {
  if (!['image/png', 'image/jpeg', 'image/webp'].includes(mime)) throw new NativeImageError('unsupportedMime');
  if (base64.length > Math.ceil(NATIVE_RESOURCE_LIMITS.imageBytes / 3) * 4) throw new NativeImageError('imageTooLarge');
  if (!base64.length || base64.length % 4 !== 0 || /[^A-Za-z0-9+/=]/.test(base64)) throw new NativeImageError('invalidBase64');
  let binary: string;
  try { binary = atob(base64); } catch { throw new NativeImageError('invalidBase64'); }
  if (binary.length > NATIVE_RESOURCE_LIMITS.imageBytes) throw new NativeImageError('imageTooLarge');
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  const view = new DataView(bytes.buffer);
  const ascii = (at: number, length: number) => String.fromCharCode(...bytes.subarray(at, at + length));
  let width = 0; let height = 0;
  if (mime === 'image/png') {
    if (bytes.length < 33 || ![137, 80, 78, 71, 13, 10, 26, 10].every((byte, i) => bytes[i] === byte) || view.getUint32(8) !== 13 || ascii(12, 4) !== 'IHDR') throw new NativeImageError('mimeMismatch');
    width = view.getUint32(16); height = view.getUint32(20);
  } else if (mime === 'image/jpeg') {
    if (bytes.length < 4 || bytes[0] !== 0xff || bytes[1] !== 0xd8) throw new NativeImageError('mimeMismatch');
    let offset = 2;
    while (offset + 3 < bytes.length) {
      if (bytes[offset++] !== 0xff) break;
      while (bytes[offset] === 0xff) offset++;
      const marker = bytes[offset++];
      if (marker === 0xda || marker === 0xd9) break;
      if (marker === 0x01 || marker >= 0xd0 && marker <= 0xd7) continue;
      if (offset + 2 > bytes.length) break;
      const length = view.getUint16(offset);
      if (length < 2 || offset + length > bytes.length) break;
      if ([0xc0, 0xc1, 0xc2, 0xc3, 0xc5, 0xc6, 0xc7, 0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf].includes(marker) && length >= 7) { height = view.getUint16(offset + 3); width = view.getUint16(offset + 5); break; }
      offset += length;
    }
  } else {
    if (bytes.length < 30 || ascii(0, 4) !== 'RIFF' || ascii(8, 4) !== 'WEBP' || view.getUint32(4, true) + 8 !== bytes.length) throw new NativeImageError('mimeMismatch');
    const chunk = ascii(12, 4);
    if (chunk === 'VP8X') { width = 1 + bytes[24] + (bytes[25] << 8) + (bytes[26] << 16); height = 1 + bytes[27] + (bytes[28] << 8) + (bytes[29] << 16); }
    else if (chunk === 'VP8L' && bytes[20] === 0x2f) { width = 1 + bytes[21] + ((bytes[22] & 0x3f) << 8); height = 1 + (bytes[22] >> 6) + (bytes[23] << 2) + ((bytes[24] & 0x0f) << 10); }
    else if (chunk === 'VP8 ' && bytes[23] === 0x9d && bytes[24] === 0x01 && bytes[25] === 0x2a) { width = view.getUint16(26, true) & 0x3fff; height = view.getUint16(28, true) & 0x3fff; }
  }
  if (!width || !height || width > NATIVE_RESOURCE_LIMITS.imageSide || height > NATIVE_RESOURCE_LIMITS.imageSide || width * height > NATIVE_RESOURCE_LIMITS.imagePixels) throw new NativeImageError('invalidDimensions');
  return { bytes, width, height };
}
