import { useSyncExternalStore } from 'react';
import { catalogs } from './catalogs';
import { formatMessage, type MessageVariables } from './message';
import { getLocaleSnapshot, subscribeLocale, getLocale, type Locale } from './runtime';
export type { LanguagePreference, Locale } from './runtime';
export { LANGUAGE_PREFERENCES } from './runtime';
export { formatDate, formatDateTime, formatNumber, formatTime } from './format';
export { getLanguagePreference, getLocale, initLocale, resetLocaleForTesting, setLocaleForTesting, updateLanguage } from './runtime';
export function translate(locale:Locale,key:string,variables?:MessageVariables):string {return formatMessage(catalogs[locale],key,variables);}
export function t(key:string,variables?:MessageVariables):string{return translate(getLocale(),key,variables);}
export function useLocale(){
  const snapshot=useSyncExternalStore(subscribeLocale,getLocaleSnapshot,getLocaleSnapshot);
  return {locale:snapshot.locale,preference:snapshot.preference,initialized:snapshot.initialized,t:(key:string,variables?:MessageVariables)=>translate(snapshot.locale,key,variables)};
}
export type TranslationKey = string;
