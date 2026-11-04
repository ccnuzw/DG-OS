import { ContextMenu as Context, DropdownMenu as Dropdown } from 'radix-ui';
import { type ReactElement, type ReactNode } from 'react';
import { Icon } from './Icon';
import { FOCUS_RING_INSET } from '../lib/focus';
import { cn } from '../lib/cn';
export type MenuItem =
  | { kind:'item'; id:string; label:string; icon?:string; shortcut?:string; disabled?:boolean; danger?:boolean; onSelect?:()=>void }
  | { kind:'checkbox'; id:string; label:string; checked:boolean; onCheckedChange?:(checked:boolean)=>void; disabled?:boolean }
  | { kind:'separator' }
  | { kind:'label'; text:string }
  | { kind:'submenu'; id:string; label:string; icon?:string; items:readonly MenuItem[] };
/** Rebuild module data; never spread untrusted item props onto a DOM/Radix component. */
export function normalizeMenu(raw: unknown): MenuItem[] {
  let count=0;
  function collect(raw:unknown, nested=false):MenuItem[] {
    if(!Array.isArray(raw))return [];
    const result:MenuItem[]=[];
    for(const item of raw) {
      if(++count>100)break;
      if(!item||typeof item!=='object')continue;
      const optional=(name:string,type:string)=>item[name]===undefined||typeof item[name]===type;
      if(item.kind==='separator'){result.push({kind:'separator'});continue;}
      if(item.kind==='label'){if(typeof item.text==='string')result.push({kind:'label',text:item.text});continue;}
      if(typeof item.id!=='string'||typeof item.label!=='string'||!optional('disabled','boolean')||!optional('icon','string'))continue;
      if(item.kind==='item'&&optional('shortcut','string')&&optional('danger','boolean')&&optional('onSelect','function'))result.push({kind:'item',id:item.id,label:item.label,icon:item.icon,shortcut:item.shortcut,danger:item.danger,disabled:item.disabled||typeof item.onSelect!=='function',onSelect:item.onSelect});
      else if(item.kind==='item'&&typeof item.onSelect!=='function'&&optional('shortcut','string')&&optional('danger','boolean'))result.push({kind:'item',id:item.id,label:item.label,icon:item.icon,shortcut:item.shortcut,danger:item.danger,disabled:true});
      else if(item.kind==='checkbox'&&typeof item.checked==='boolean'&&optional('onCheckedChange','function'))result.push({kind:'checkbox',id:item.id,label:item.label,checked:item.checked,disabled:item.disabled||typeof item.onCheckedChange!=='function',onCheckedChange:item.onCheckedChange});
      else if(item.kind==='submenu'&&!nested&&Array.isArray(item.items))result.push({kind:'submenu',id:item.id,label:item.label,icon:item.icon,items:collect(item.items,true)});
    }
    return result;
  }
  return collect(raw);
}
const SURFACE='z-popover min-w-40 max-h-(--radix-dropdown-menu-content-available-height) overflow-auto rounded-md border border-solid border-border bg-popover p-1 font-sans text-sm text-popover-foreground shadow-md';
const ITEM=cn('flex min-h-7 cursor-default select-none items-center gap-2 rounded-sm px-2 py-1 data-[highlighted]:bg-accent data-[disabled]:pointer-events-none data-[disabled]:opacity-disabled',FOCUS_RING_INSET);
function entries(items:readonly MenuItem[], context:boolean):ReactNode {
  const P=context?Context:Dropdown;
  return items.map((item,index)=>{
    if(item.kind==='separator')return <P.Separator key={index} className="my-1 border-t border-border"/>;
    if(item.kind==='label')return <P.Label key={index} className="px-2 py-1 text-xs text-muted-foreground">{item.text}</P.Label>;
    if(item.kind==='submenu')return <P.Sub key={item.id}><P.SubTrigger className={ITEM}>{item.icon?<Icon name={item.icon}/>:null}{item.label}<span className="ml-auto"><Icon name="chevron-right"/></span></P.SubTrigger><P.Portal><P.SubContent className={SURFACE}>{entries(item.items,context)}</P.SubContent></P.Portal></P.Sub>;
    if(item.kind==='checkbox')return <P.CheckboxItem key={item.id} checked={item.checked} disabled={item.disabled} onCheckedChange={item.onCheckedChange} className={ITEM}><span className="size-4"><P.ItemIndicator><Icon name="check"/></P.ItemIndicator></span>{item.label}</P.CheckboxItem>;
    return <P.Item key={item.id} disabled={item.disabled} onSelect={item.onSelect} className={cn(ITEM,item.danger&&'text-error-text')}>{item.icon?<Icon name={item.icon}/>:null}{item.label}{item.shortcut?<span className="ml-auto pl-4 text-xs text-muted-foreground">{item.shortcut}</span>:null}</P.Item>;
  });
}
export interface ContextMenuProps {items:readonly MenuItem[];children:ReactNode;onOpenChange?:(open:boolean)=>void}
export function ContextMenu({items,children,onOpenChange}:ContextMenuProps){return <Context.Root onOpenChange={onOpenChange}><Context.Trigger asChild><div>{children}</div></Context.Trigger><Context.Portal><Context.Content className={SURFACE}>{entries(normalizeMenu(items),true)}</Context.Content></Context.Portal></Context.Root>;}
export interface DropdownMenuProps {trigger:ReactElement;items:readonly MenuItem[];label?:string}
export function DropdownMenu({trigger,items,label}:DropdownMenuProps){return <Dropdown.Root><Dropdown.Trigger asChild>{trigger}</Dropdown.Trigger><Dropdown.Portal><Dropdown.Content aria-label={label} sideOffset={4} className={SURFACE}>{entries(normalizeMenu(items),false)}</Dropdown.Content></Dropdown.Portal></Dropdown.Root>;}
