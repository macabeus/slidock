import { Banner, Button, DropZone, Icon } from '@slidock/ui';

import type { CompilerStatus } from '../compile/client';
import { ACCEPT } from '../compile/input';
import type { AppError } from '../compile/session';
import { ErrorDetails } from './ErrorDetails';

export interface LandingProps {
  compiler: CompilerStatus;
  /** Why the last input did not open. */
  error: { name: string | null; error: AppError } | null;
  onPick(file: File): void;
  onRetryCompiler(): void;
  onDismissError(): void;
}

export function Landing({ compiler, error, onPick, onRetryCompiler, onDismissError }: LandingProps) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-6 py-8">
      {error && error.error.kind !== 'compiler' && (
        <ErrorDetails name={error.name} error={error.error} onDismiss={onDismissError} className="w-full max-w-2xl" />
      )}
      {compiler.state === 'failed' && (
        <Banner
          tone="danger"
          title="The compiler couldn't load"
          className="w-full max-w-2xl"
          actions={
            <Button variant="wash" size="sm" onClick={onRetryCompiler}>
              <Icon name="refresh" className="size-4" />
              Try again
            </Button>
          }
        >
          <p>slidock converts presentations with WebAssembly in a background worker, and it did not start.</p>
          <p className="mt-1 break-words font-mono text-xs text-muted">{compiler.message}</p>
        </Banner>
      )}
      <DropZone
        accept={ACCEPT}
        onPick={(files) => {
          const [file] = files;
          if (file) {
            onPick(file);
          }
        }}
        title="Drop a .pptx, .key or .odp"
        hint="A Keynote package folder works too. PDF isn't supported."
        buttonLabel="Choose a presentation"
      />
      <div className="space-y-1 text-center text-sm text-muted">
        <p className="flex items-center justify-center gap-2">
          <Icon name="check-circle" className="size-4 shrink-0 text-success" />
          Converted in your browser. Your file never leaves this tab.
        </p>
        <p className="text-xs">
          Viewing and presenting work in current browsers; using the fonts installed on this computer needs Chrome or
          Edge.
        </p>
      </div>
    </div>
  );
}
