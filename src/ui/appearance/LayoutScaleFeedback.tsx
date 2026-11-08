import { createPortal } from 'react-dom';
import { useAppearance } from './index';
import { useLocale } from '../../i18n';

export function LayoutScaleFeedback(){
  const {state}=useAppearance();
  const {t}=useLocale();
  const error=state.error==='revision_conflict'?'appearance.error.conflict':state.phase==='unconfirmed'?'appearance.error.unconfirmed':state.error==='save_failed'?'appearance.error.failed':null;
  const slider=document.querySelector<HTMLElement>('.appearance-layout-scale [role="slider"]');
  const box=slider?.getBoundingClientRect();
  const sliderVisible=box&&box.width>0&&box.height>0&&box.top>=0&&box.bottom<=window.innerHeight;
  if((state.layoutFeedback===undefined||sliderVisible)&&!error)return null;
  return createPortal(<div role="status" aria-live="polite" className="pointer-events-none fixed inset-x-5 bottom-5 z-toast flex justify-center"><p className="m-0 max-w-full rounded-md border border-solid border-edge-control bg-surface-panel px-4 py-2 text-sm text-ink-primary shadow-sm">{error?t(error):t('appearance.layoutFeedback',{value:state.layoutFeedback!})}</p></div>,document.body);
}
