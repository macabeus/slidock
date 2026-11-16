import type { DeckDiagnostic, ErrorKind } from '@slidock/types';
import { Banner, Button, Icon } from '@slidock/ui';

import type { AppError } from '../compile/session';

const COMPILE_TITLES: Record<ErrorKind, string> = {
  refused: "slidock couldn't read this presentation",
  failed: 'This presentation failed to convert',
  request: 'The compiler refused the request',
  bundle: "The zip couldn't be written",
  cancelled: 'The conversion was cancelled',
  internal: 'The compiler crashed',
};

const COMPILE_HINTS: Record<ErrorKind, string> = {
  refused: 'It may be password-protected, too large, damaged, or not a presentation.',
  failed: 'Nothing could be printed. The codes below say what went wrong.',
  request: 'This is a bug in slidock. Please report it with the details below.',
  bundle: 'This is a bug in slidock. Please report it with the details below.',
  cancelled: 'Open the file again to retry.',
  internal: 'This is a bug in slidock. Please report it with the details below.',
};

function DiagnosticList({ diagnostics }: { diagnostics: readonly DeckDiagnostic[] }) {
  if (diagnostics.length === 0) {
    return null;
  }
  return (
    <ul className="mt-2 space-y-1">
      {diagnostics.map((diagnostic, index) => (
        <li key={`${diagnostic.code}-${index}`} className="flex gap-2 text-xs">
          <code className="shrink-0 font-mono font-semibold">{diagnostic.code}</code>
          <span className="text-fg-secondary">{diagnostic.message}</span>
        </li>
      ))}
    </ul>
  );
}

/** Report text for a bug report: the error as JSON. */
function report(name: string | null, error: AppError): string {
  return JSON.stringify({ file: name, ...error }, null, 2);
}

export interface ErrorDetailsProps {
  name: string | null;
  error: AppError;
  onDismiss?(): void;
  className?: string;
}

export function ErrorDetails({ name, error, onDismiss, className }: ErrorDetailsProps) {
  const copy = () => {
    void navigator.clipboard?.writeText(report(name, error)).catch(() => {});
  };
  const dismiss = onDismiss && (
    <Button variant="ghost" size="sm" onClick={onDismiss}>
      Dismiss
    </Button>
  );
  if (error.kind === 'unsupported') {
    return (
      <Banner
        tone="warning"
        title={name ? `Can't open “${name}”` : "Can't open this drop"}
        className={className}
        actions={dismiss}
      >
        <p>{error.message}</p>
      </Banner>
    );
  }
  if (error.kind === 'compiler') {
    return (
      <Banner tone="danger" title="The compiler stopped" className={className} actions={dismiss}>
        <p className="break-words font-mono text-xs">{error.message}</p>
      </Banner>
    );
  }
  const { kind, message, diagnostics } = error.error;
  return (
    <Banner
      tone="danger"
      title={COMPILE_TITLES[kind]}
      className={className}
      actions={
        <>
          <Button variant="ghost" size="sm" onClick={copy}>
            <Icon name="clipboard" className="size-4" />
            Copy details
          </Button>
          {dismiss}
        </>
      }
    >
      {name && <p className="font-medium text-fg">{name}</p>}
      <p>{COMPILE_HINTS[kind]}</p>
      <DiagnosticList diagnostics={diagnostics} />
      <p className="mt-2 break-words font-mono text-xs text-muted">
        {kind}: {message}
      </p>
    </Banner>
  );
}
