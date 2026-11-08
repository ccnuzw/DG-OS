import { Home, Settings, Search, Plus, X, Check, ChevronDown, ChevronRight, MoreHorizontal, Copy, Trash2, Play, Square, ArrowLeft, Folder, FileText, Code, Palette, Globe, Menu, FolderOpen, type LucideIcon } from 'lucide-react';
import { cn } from '../lib/cn';
const ICONS: Record<string,LucideIcon>={home:Home,settings:Settings,search:Search,plus:Plus,close:X,check:Check,'chevron-down':ChevronDown,'chevron-right':ChevronRight,more:MoreHorizontal,copy:Copy,delete:Trash2,play:Play,stop:Square,'arrow-left':ArrowLeft,folder:Folder,'folder-open':FolderOpen,file:FileText,code:Code,palette:Palette,globe:Globe,menu:Menu};
const warned=new Set<string>();
export interface IconProps{name:string;size?:'sm'|'md'|'lg';label?:string}
export function Icon({name,size='md',label}:IconProps){
  const Glyph=ICONS[name];
  if(!Glyph){if(!warned.has(name)){warned.add(name);console.warn('Unknown UI icon');}return null;}
  return <Glyph className={cn(size==='sm'?'size-3.5':size==='lg'?'size-5':'size-4')} aria-hidden={label?undefined:true} role={label?'img':undefined} aria-label={label} />;
}
