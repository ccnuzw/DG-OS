import type { AppearanceController } from './controller';

/** Semantic symbols first; physical minus/equal are a fallback for common non-US layouts. */
export function layoutScaleDirection(event:KeyboardEvent): -1|1|null {
  if(!event.metaKey||event.ctrlKey||event.altKey||event.isComposing||event.keyCode===229)return null;
  if(event.key==='-'||(!event.shiftKey&&event.code==='Minus'))return -1;
  if(event.key==='='||event.key==='+'||event.code==='Equal')return 1;
  return null;
}

export function installLayoutScaleShortcuts(target:Window,controller:AppearanceController,platform=navigator.platform):()=>void {
  if(!/Mac/i.test(platform))return()=>{};
  const keydown=(event:KeyboardEvent)=>{
    if(event.defaultPrevented)return;
    const direction=layoutScaleDirection(event);
    if(direction===null)return;
    // Claim repeats and boundaries too, so WebKit never applies native page zoom.
    event.preventDefault();
    if(!event.repeat)controller.adjustLayoutScale(direction);
  };
  // Bubble after React, editors and terminals; consumed or stopped events never reach this handler.
  target.addEventListener('keydown',keydown);
  return()=>target.removeEventListener('keydown',keydown);
}
