/** The slidock mark: two stacked slides in the brand gradient. */
export function Logo({ className = 'size-full' }: { className?: string }) {
  return (
    <svg viewBox="0 0 64 64" className={className} aria-hidden>
      <defs>
        <linearGradient id="sd-logo" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#f472b6" />
          <stop offset="0.5" stopColor="#fb7185" />
          <stop offset="1" stopColor="#fb923c" />
        </linearGradient>
      </defs>
      <rect x="14" y="8" width="42" height="30" rx="5" fill="url(#sd-logo)" opacity="0.45" />
      <rect x="8" y="18" width="42" height="30" rx="5" fill="url(#sd-logo)" />
      <path d="M24 26v14l12-7z" fill="#fff" opacity="0.92" />
      <rect x="20" y="52" width="18" height="4" rx="2" fill="url(#sd-logo)" />
    </svg>
  );
}
