import type { ReactNode } from 'react';
import { Card, Stack } from '@dgos/ui';

// Full-screen container for the pages the gate shows instead of the app (welcome, sign-in, read failure).
export function AuthShell({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="flex min-h-screen w-full items-center justify-center bg-background p-6">
      <Card className="w-full max-w-md">
        <Card.Header>
          <h1 className="m-0 text-lg font-semibold text-foreground">{title}</h1>
        </Card.Header>
        <Card.Body>
          <Stack gap={4}>{children}</Stack>
        </Card.Body>
      </Card>
    </div>
  );
}

export function InlineError({ children }: { children: ReactNode }) {
  return (
    <p role="alert" className="m-0 text-sm text-error-text">
      {children}
    </p>
  );
}
