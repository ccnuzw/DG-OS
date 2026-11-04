import { useSyncExternalStore } from 'react';
import { createAppearanceController, type AppearanceController } from './controller';
import { handleError } from '../errors/handleError';
import { readAppearancePreferences, writeAppearancePreferences, recoverAppearancePreferences } from './host';
export type { Appearance, AppearanceSnapshot, AppearanceRead, AppearanceSave, Density, ResolvedTheme, Theme } from './model';
export { DEFAULTS, DENSITIES, THEMES } from './model';
export { createAppearanceController } from './controller';
export type { AppearanceController, AppearanceDeps, AppearanceState } from './controller';
let controller:AppearanceController|null=null;
function instance():AppearanceController {
  controller??=createAppearanceController({root:document.documentElement,read:readAppearancePreferences,write:writeAppearancePreferences,recover:recoverAppearancePreferences,
    matchMedia:typeof window.matchMedia==='function'?window.matchMedia.bind(window):undefined,onWriteError:error=>handleError(error,{where:'appearance'})});
  return controller;
}
export function initAppearance():Promise<void>{return instance().init();}
export function useAppearance(){const current=instance();return {state:useSyncExternalStore(current.subscribe,current.getSnapshot,current.getSnapshot),controller:current};}
export function disposeAppearance():void{controller?.dispose();controller=null;}
