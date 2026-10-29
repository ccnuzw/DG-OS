import { cn } from '../lib/cn';
import { omitStyle, type DomProps } from '../lib/props';

// The container has no interactive role (FR-005 §7.1.7). Slots are plain `div`s: the heading level of a title
// belongs to the page that uses the card, so none is imposed here.

function CardRoot(props: DomProps<'div'>) {
  const { className, ...rest } = omitStyle(props);
  return <div {...rest} className={cn('rounded-lg border border-border bg-card text-card-foreground shadow-sm', className)} />;
}

function CardHeader(props: DomProps<'div'>) {
  const { className, ...rest } = omitStyle(props);
  return <div {...rest} className={cn('border-b border-border px-4 py-3 text-base font-semibold', className)} />;
}

function CardBody(props: DomProps<'div'>) {
  const { className, ...rest } = omitStyle(props);
  return <div {...rest} className={cn('px-4 py-4 text-sm', className)} />;
}

function CardFooter(props: DomProps<'div'>) {
  const { className, ...rest } = omitStyle(props);
  return <div {...rest} className={cn('flex items-center justify-end gap-2 border-t border-border px-4 py-3', className)} />;
}

export const Card = Object.assign(CardRoot, { Header: CardHeader, Body: CardBody, Footer: CardFooter });
