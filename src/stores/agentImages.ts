import { create } from 'zustand';
import type { ClipboardEvent } from 'react';
import type { AgentSession } from './agentRunnerStore';
import { useDevelopment } from './developmentStore';
import { developmentDraftKey, useDevelopmentDrafts } from './developmentDrafts';
import { fileBase64 } from '../components/chat/Attachments';

export type AgentImageMime = 'image/png' | 'image/jpeg' | 'image/webp';
export interface AgentImageInput { mime: AgentImageMime; base64: string }
export interface CapturedAgentImage { id: string; name: string; bytes: number; input: AgentImageInput }
export interface AgentImageDraft { images: CapturedAgentImage[]; busy: boolean; error: string | null }
interface AgentImagesState { drafts: Record<string, AgentImageDraft>; reset(): void }
export const AGENT_IMAGE_COUNT_LIMIT = 4;
export const AGENT_IMAGE_BYTES_LIMIT = 2 * 1024 * 1024;
export const AGENT_IMAGE_TOTAL_LIMIT = 2 * 1024 * 1024;
// Draft images are volatile, never written to localStorage, DB, or IPC until send.
// A bounded account-wide budget also prevents many parked drafts retaining bytes.
const ACCOUNT_IMAGE_BYTES_LIMIT = 64 * 1024 * 1024;
const IMAGE_DRAFT_LIFETIME_MS = 60 * 60 * 1000;
const EMPTY_IMAGES: CapturedAgentImage[] = [];
const revisions = new Map<string, number>();
const timers = new Map<string, number>();
let updatingDraftGuard = false;
export const useAgentImages = create<AgentImagesState>((set, get) => ({ drafts: {}, reset() {
  const owners = new Set(Object.keys(get().drafts));
  for (const timer of timers.values()) clearTimeout(timer);
  timers.clear(); revisions.clear(); set({ drafts: {} });
  useDevelopmentDrafts.setState(state => ({ entries: Object.fromEntries(Object.entries(state.entries).filter(([key]) => !owners.has(key))) }));
} }));

export const agentImagesKey = (projectId: string | null, sessionOrDraft: Pick<AgentSession, 'id' | 'generation'> | string) => typeof sessionOrDraft === 'string'
  ? developmentDraftKey('conversation', projectId, 'images', 'draft', sessionOrDraft)
  : developmentDraftKey('conversation', projectId, 'images', sessionOrDraft.id, sessionOrDraft.generation);
function currentOwner(owner: string) {
  try { const parts = JSON.parse(owner); return parts[0] === useDevelopment.getState().epoch && parts[1] === 'conversation' && parts[3] === 'images'; }
  catch { return false; }
}
function draft(owner: string): AgentImageDraft { return useAgentImages.getState().drafts[owner] ?? { images: EMPTY_IMAGES, busy: false, error: null }; }
function update(owner: string, value: AgentImageDraft) {
  useAgentImages.setState(state => ({ drafts: { ...state.drafts, [owner]: value } }));
}
function mark(owner: string, images: CapturedAgentImage[], busy = false) {
  // Only opaque identities enter the generic unsaved-draft guard. Bytes/names do
  // not leak into draft inspection, persistence or JSON comparison paths.
  updatingDraftGuard = true;
  try {
    useDevelopmentDrafts.setState(state => {
      const entries = { ...state.entries };
      if (images.length || busy) entries[owner] = { kind: 'conversation', baseline: [], value: [...images.map(image => image.id), ...(busy ? ['importing'] : [])] };
      else delete entries[owner];
      return { entries };
    });
  } finally { updatingDraftGuard = false; }
}
function invalidate(owner: string) { revisions.set(owner, (revisions.get(owner) ?? 0) + 1); }
function stopTimer(owner: string) { clearTimeout(timers.get(owner)); timers.delete(owner); }
function scheduleExpiry(owner: string) {
  stopTimer(owner);
  const timer = window.setTimeout(() => {
    clearAgentImages(owner);
    if (currentOwner(owner)) update(owner, { images: [], busy: false, error: 'expired' });
  }, IMAGE_DRAFT_LIFETIME_MS);
  timers.set(owner, timer);
}
export function captureAgentImages(owner: string): CapturedAgentImage[] { return currentOwner(owner) ? draft(owner).images.slice() : []; }
export function agentImageInputs(images: CapturedAgentImage[]): AgentImageInput[] { return images.map(image => image.input); }
export function acknowledgeAgentImages(owner: string, submitted: ReadonlyArray<Pick<CapturedAgentImage, 'id'>>) {
  if (!currentOwner(owner)) return;
  const value = draft(owner);
  const remaining = value.images.filter(image => !submitted.some(sent => sent.id === image.id));
  if (remaining.length === value.images.length) return;
  if (!remaining.length && !value.busy) clearAgentImages(owner);
  else { update(owner, { ...value, images: remaining }); mark(owner, remaining, value.busy); }
}
export function clearAgentImages(owner: string) {
  invalidate(owner); stopTimer(owner);
  useAgentImages.setState(state => { const drafts = { ...state.drafts }; delete drafts[owner]; return { drafts }; });
  mark(owner, []);
}
export function transferAgentImages(from: string, to: string) {
  if (from === to || !currentOwner(from) || !currentOwner(to)) return;
  const source = draft(from);
  if (source.busy) throw new Error('agent_images_busy');
  const target = draft(to);
  const images = [...target.images, ...source.images.filter(image => !target.images.some(existing => existing.id === image.id))];
  if (target.busy || images.length > AGENT_IMAGE_COUNT_LIMIT || images.reduce((sum, image) => sum + image.bytes, 0) > AGENT_IMAGE_TOTAL_LIMIT) throw new Error('agent_images_transfer_conflict');
  invalidate(from); invalidate(to);
  stopTimer(from);
  useAgentImages.setState(state => { const drafts = { ...state.drafts, [to]: { images, busy: false, error: null } }; delete drafts[from]; return { drafts }; });
  mark(to, images); mark(from, []);
  if (images.length) scheduleExpiry(to);
}
export function removeAgentImage(owner: string, id: string) {
  if (!currentOwner(owner) || draft(owner).busy) return;
  acknowledgeAgentImages(owner, draft(owner).images.filter(image => image.id === id));
}
export function setAgentImageError(owner: string, error: string) { if (currentOwner(owner)) update(owner, { ...draft(owner), error }); }
function mimeFor(file: File): AgentImageMime | null {
  return ['image/png', 'image/jpeg', 'image/webp'].includes(file.type) ? file.type as AgentImageMime : null;
}
export async function addAgentImages(owner: string, files: File[]) {
  if (!files.length || !currentOwner(owner) || draft(owner).busy) return;
  const existing = draft(owner);
  const bytes = files.reduce((sum, file) => sum + file.size, 0);
  const accountBytes = Object.values(useAgentImages.getState().drafts).reduce((sum, value) => sum + value.images.reduce((size, image) => size + image.bytes, 0), 0);
  if (existing.images.length + files.length > AGENT_IMAGE_COUNT_LIMIT || existing.images.reduce((sum, image) => sum + image.bytes, 0) + bytes > AGENT_IMAGE_TOTAL_LIMIT || accountBytes + bytes > ACCOUNT_IMAGE_BYTES_LIMIT || files.some(file => !file.size || file.size > AGENT_IMAGE_BYTES_LIMIT)) {
    setAgentImageError(owner, 'limits'); return;
  }
  if (files.some(file => !mimeFor(file))) { setAgentImageError(owner, 'type'); return; }
  const revision = (revisions.get(owner) ?? 0) + 1;
  revisions.set(owner, revision);
  update(owner, { ...existing, busy: true, error: null });
  mark(owner, existing.images, true);
  try {
    const added: CapturedAgentImage[] = [];
    for (const file of files) {
      const base64 = await fileBase64(file);
      if (!currentOwner(owner) || revisions.get(owner) !== revision) return;
      added.push({ id: crypto.randomUUID(), name: file.name.slice(0, 128) || 'image', bytes: file.size, input: { mime: mimeFor(file)!, base64 } });
    }
    // Other owners may have imported while this FileReader was pending.
    const retainedBytes = Object.values(useAgentImages.getState().drafts).reduce((sum, value) => sum + value.images.reduce((size, image) => size + image.bytes, 0), 0);
    if (retainedBytes + bytes > ACCOUNT_IMAGE_BYTES_LIMIT) { const current = draft(owner); update(owner, { ...current, busy: false, error: 'limits' }); mark(owner, current.images); return; }
    const images = [...draft(owner).images, ...added];
    update(owner, { images, busy: false, error: null }); mark(owner, images); scheduleExpiry(owner);
  } catch {
    if (currentOwner(owner) && revisions.get(owner) === revision) { const current = draft(owner); update(owner, { ...current, busy: false, error: 'read' }); mark(owner, current.images); }
  }
}

export function agentImagePaste(owner: string, disabled = false) {
  return (event: ClipboardEvent<HTMLElement>) => {
    const files = Array.from(event.clipboardData.files ?? []);
    const images = files.filter(file => file.type.startsWith('image/'));
    if (!images.length) {
      for (const item of Array.from(event.clipboardData.items ?? [])) {
        if (item.kind === 'file' && item.type.startsWith('image/')) { const file = item.getAsFile(); if (file) images.push(file); }
      }
    }
    if (!images.length) return; // Text and remote URLs remain normal composer text.
    event.preventDefault();
    if (!disabled) void addAgentImages(owner, images);
  };
}

useDevelopment.subscribe((state, previous) => { if (state.epoch !== previous.epoch) useAgentImages.getState().reset(); });
useDevelopmentDrafts.subscribe((state, previous) => {
  if (updatingDraftGuard) return;
  for (const owner of Object.keys(useAgentImages.getState().drafts)) {
    if (previous.entries[owner] && !state.entries[owner]) {
      invalidate(owner); stopTimer(owner);
      useAgentImages.setState(current => { const drafts = { ...current.drafts }; delete drafts[owner]; return { drafts }; });
    }
  }
});
