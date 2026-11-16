import { Banner, ProgressBar } from '@slidock/ui';
import type { ReactNode } from 'react';

import type { Resource } from '../data/resource';
import { ApiError } from '../data/source';

export function Loading({ label }: { label: string }) {
  return (
    <div className="mx-auto w-full max-w-sm py-16" aria-busy="true">
      <p className="mb-3 text-center text-sm text-muted">{label}</p>
      <ProgressBar label={label} />
    </div>
  );
}

export function LoadError({ what, error }: { what: string; error: Error }) {
  const detail = error instanceof ApiError ? `${error.path} answered ${error.status}.` : error.message;
  return (
    <Banner tone="danger" title={`Could not load ${what}`}>
      {detail}
    </Banner>
  );
}

/** Renders a resource's value, or its loading and error states. */
export function Await<T>({
  resource,
  what,
  children,
}: {
  resource: Resource<T>;
  what: string;
  children(value: T): ReactNode;
}) {
  if (resource.state === 'loading') {
    return <Loading label={`Loading ${what}…`} />;
  }
  if (resource.state === 'error') {
    return <LoadError what={what} error={resource.error} />;
  }
  return <>{children(resource.value)}</>;
}

export function Empty({ children }: { children: ReactNode }) {
  return (
    <p className="rounded-lg border border-dashed border-line px-4 py-8 text-center text-sm text-muted">{children}</p>
  );
}
