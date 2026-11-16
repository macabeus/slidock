/** The bench's mark: a slide and its oracle, offset, over a score bar in the brand gradient. */
export function Logo({ className = 'size-full' }: { className?: string }) {
  return (
    <svg viewBox="0 0 64 64" className={className} aria-hidden>
      <defs>
        <linearGradient id="sd-bench-logo" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#f472b6" />
          <stop offset="0.5" stopColor="#fb7185" />
          <stop offset="1" stopColor="#fb923c" />
        </linearGradient>
      </defs>
      <rect x="16" y="8" width="40" height="28" rx="5" fill="url(#sd-bench-logo)" opacity="0.45" />
      <rect x="8" y="16" width="40" height="28" rx="5" fill="url(#sd-bench-logo)" />
      <path d="M16 36l8-9 6 6 5-4 7 7z" fill="#fff" opacity="0.92" />
      <rect x="8" y="50" width="14" height="6" rx="2" fill="#34d399" />
      <rect x="24" y="50" width="12" height="6" rx="2" fill="#38bdf8" />
      <rect x="38" y="50" width="8" height="6" rx="2" fill="#fbbf24" />
    </svg>
  );
}
