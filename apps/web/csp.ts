// The app's Content Security Policy, set as a <meta> in the built index.html. The app loads only its own scripts,
// compiles WebAssembly (`'wasm-unsafe-eval'`) in a module worker, shows the deck's media and fonts from `blob:` and
// `data:` URLs, and may fetch open fonts from Google Fonts; nothing else leaves the browser.
export const CSP = [
  "default-src 'self'",
  "script-src 'self' 'wasm-unsafe-eval'",
  "worker-src 'self' blob:",
  "style-src 'self' 'unsafe-inline' https://fonts.googleapis.com",
  "font-src 'self' data: blob: https://fonts.gstatic.com",
  "img-src 'self' data: blob:",
  "media-src 'self' blob:",
  "connect-src 'self' https://fonts.googleapis.com https://fonts.gstatic.com",
  "object-src 'none'",
  "base-uri 'self'",
  "form-action 'none'",
].join('; ');
