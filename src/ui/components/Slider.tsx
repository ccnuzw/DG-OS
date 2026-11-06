import { Slider as Primitive } from 'radix-ui';
import { useId } from 'react';
import { FOCUS_RING } from '../lib/focus';
import { cn } from '../lib/cn';
export interface SliderProps {label:string;hideLabel?:boolean;value:number;min:number;max:number;step:number;onValueChange:(value:number)=>void;onValueCommit?:(value:number)=>void;disabled?:boolean}
let warned=false;
export function Slider({label,hideLabel,value,min,max,step,onValueChange,onValueCommit,disabled}:SliderProps){
  const id=useId();
  if(![min,max,step,value].every(Number.isFinite)||min>=max||step<=0||typeof label!=='string'||!label.trim()){
    if(!warned){warned=true;console.warn('Invalid slider configuration');}return null;
  }
  return <div className="flex flex-col gap-1"><label id={id} className={hideLabel?'sr-only':'text-sm font-medium'}>{label}</label><Primitive.Root aria-labelledby={id} value={[value]} min={min} max={max} step={step} disabled={disabled} onValueChange={values=>onValueChange(values[0])} onValueCommit={values=>onValueCommit?.(values[0])} className="relative flex h-7 touch-none select-none items-center disabled:opacity-disabled"><Primitive.Track className="relative h-1 flex-1 rounded-full bg-surface-inset"><Primitive.Range className="absolute h-full rounded-full bg-accent-default"/></Primitive.Track><Primitive.Thumb aria-labelledby={id} className={cn('block size-4 rounded-full border border-solid border-edge-control bg-surface-field',FOCUS_RING)}/></Primitive.Root></div>;
}
