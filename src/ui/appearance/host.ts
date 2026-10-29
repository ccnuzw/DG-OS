import { invoke } from '@tauri-apps/api/core';
import type { Appearance } from './model';

// The envelope every host command returns (V1 interface contract).
interface CommandResponse<T> {
  success: boolean;
  data?: T;
  error?: { code?: number; message?: string };
}

function unwrap<T>(command: string, response: CommandResponse<T> | undefined): T | undefined {
  if (!response?.success) {
    const detail = response?.error?.code !== undefined ? ` (code ${response.error.code})` : '';
    throw new Error(`${command} failed${detail}`);
  }
  return response.data;
}

export async function readAppearancePreferences(): Promise<unknown> {
  const response = await invoke<CommandResponse<unknown>>('get_appearance_preferences');
  return unwrap('get_appearance_preferences', response);
}

export async function writeAppearancePreferences(patch: Partial<Appearance>): Promise<unknown> {
  const response = await invoke<CommandResponse<unknown>>('set_appearance_preferences', { patch });
  return unwrap('set_appearance_preferences', response);
}
