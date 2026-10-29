import { Tabs as TabsPrimitive } from 'radix-ui';
import type { ComponentProps } from 'react';
import { cn } from '../lib/cn';
import { FOCUS_RING } from '../lib/focus';
import { omitStyle } from '../lib/props';

// Radix Tabs (D066). `role="tablist"`, `tab`, `tabpanel`, `aria-selected`, roving tabindex, arrow keys that move
// focus and (automatic activation) selection. Only the active panel is mounted.

type StyleFree<T> = Omit<T, 'style'>;

/** Controlled only: `value` and `onValueChange` are required, there is no `defaultValue` (FR-005 §7.1.7). */
export type TabsProps = StyleFree<Omit<ComponentProps<typeof TabsPrimitive.Root>, 'defaultValue' | 'value' | 'onValueChange'>> & {
  value: string;
  onValueChange: (value: string) => void;
};

function TabsRoot(props: TabsProps) {
  const { className, ...rest } = omitStyle(props);
  return <TabsPrimitive.Root {...rest} className={cn('flex flex-col gap-4', className)} />;
}

function TabsList(props: StyleFree<ComponentProps<typeof TabsPrimitive.List>>) {
  const { className, ...rest } = omitStyle(props);
  return <TabsPrimitive.List {...rest} className={cn('inline-flex items-center gap-1 border-b border-border', className)} />;
}

function TabsTrigger(props: StyleFree<ComponentProps<typeof TabsPrimitive.Trigger>>) {
  const { className, ...rest } = omitStyle(props);
  return (
    <TabsPrimitive.Trigger
      {...rest}
      className={cn(
        '-mb-px inline-flex h-9 items-center border-b-2 border-transparent px-3 text-sm font-medium text-muted-foreground transition-colors',
        'enabled:hover:text-foreground',
        'data-[state=active]:border-primary data-[state=active]:text-foreground',
        'disabled:cursor-not-allowed disabled:opacity-disabled',
        FOCUS_RING,
        className,
      )}
    />
  );
}

function TabsContent(props: StyleFree<ComponentProps<typeof TabsPrimitive.Content>>) {
  const { className, ...rest } = omitStyle(props);
  return <TabsPrimitive.Content {...rest} className={cn(FOCUS_RING, className)} />;
}

export const Tabs = Object.assign(TabsRoot, { List: TabsList, Trigger: TabsTrigger, Content: TabsContent });
