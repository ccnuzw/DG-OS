import { invoke } from '@tauri-apps/api/core';
import type { Appearance } from './model';
interface CommandResponse<T> { success:boolean;data?:T;error?:{code?:number;message?:string} }
function unwrap<T>(response:CommandResponse<T>):T {
  if(!response?.success||response.data===undefined)throw Object.assign(new Error('Appearance command failed'),{kind:'appearance_error',code:response?.error?.code??1000000});
  return response.data;
}
export async function readAppearancePreferences():Promise<unknown> { return unwrap(await invoke<CommandResponse<unknown>>('get_appearance_preferences')); }
export async function writeAppearancePreferences(snapshot:Appearance,expectedRevision:number):Promise<unknown> {return unwrap(await invoke<CommandResponse<unknown>>('set_appearance_preferences',{snapshot,expectedRevision}));}
export async function recoverAppearancePreferences(recoveryToken:string):Promise<unknown> {return unwrap(await invoke<CommandResponse<unknown>>('recover_appearance_preferences',{recoveryToken}));}
