import {
  createAppearanceController,
  type AppearanceController,
} from './controller';
import { handleError } from '../errors/handleError';
import { readAppearancePreferences, writeAppearancePreferences } from './host';
import type { Appearance, Density, ResolvedTheme, Theme } from './model';

export type { Appearance, Density, ResolvedTheme, Theme } from './model';
export { DEFAULTS, DENSITIES, THEMES } from './model';
export { createAppearanceController } from './controller';
export type { AppearanceController, AppearanceDeps } from './controller';

// The application's controller. Created on first use so importing this module never touches `document`.
let controller: AppearanceController | null = null;

function instance(): AppearanceController {
  controller ??= createAppearanceController({
    root: document.documentElement,
    read: readAppearancePreferences,
    write: writeAppearancePreferences,
    matchMedia: typeof window.matchMedia === 'function' ? window.matchMedia.bind(window) : undefined,
    // The controller has already rolled the interface back (AC21); this tells the user the save failed.
    onWriteError: (error) => handleError(error, { where: 'appearance' }),
  });
  return controller;
}

export const initAppearance = (): Promise<void> => instance().init();
export const updateAppearance = (patch: Partial<Appearance>): Promise<void> => instance().update(patch);
export const getCurrentTheme = (): Theme => instance().getTheme();
export const getResolvedTheme = (): ResolvedTheme => instance().getResolvedTheme();
export const getCurrentDensity = (): Density => instance().getDensity();

export function disposeAppearance(): void {
  controller?.dispose();
  controller = null;
}
