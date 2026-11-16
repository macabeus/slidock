// The presenter window's rules: the slidock app's look (`@slidock/ui`'s tokens, dark by default and light by
// `data-theme` or the system preference; the compact header card with the logo, the gradient title, the subtitle,
// the theme toggle and the brand strip; the status banner) as plain CSS, so a deck page draws it without the app's
// stylesheet, and the presenter view's layout, which keeps the player's own dark palette in either theme. The window
// sets its own box sizing and margins, so it draws the same under the web app's reset as on a bare deck page.

const DARK =
  '--sd-root:#0f172a;--sd-root-gradient:linear-gradient(135deg,#0f172a 0%,#1e293b 50%,#0f172a 100%);' +
  '--sd-surface:#1e293b;--sd-elevated:#334155;--sd-text:#f1f5f9;--sd-secondary:#cbd5e1;--sd-muted:#94a3b8;' +
  '--sd-accent:#fb923c;--sd-brand:linear-gradient(to right,#f472b6,#fb7185,#fb923c);--sd-ring:#f472b6;' +
  '--sd-warning:#fcd34d;color-scheme:dark';

const LIGHT =
  '--sd-root:#f8fafc;--sd-root-gradient:none;--sd-surface:#ffffff;--sd-elevated:#f1f5f9;--sd-text:#0f172a;' +
  '--sd-secondary:#334155;--sd-muted:#526075;--sd-accent:#c2410c;' +
  '--sd-brand:linear-gradient(to right,#be185d,#be123c,#c2410c);--sd-ring:#be185d;--sd-warning:#92400e;' +
  'color-scheme:light';

const MIX = (token: string, percent: number) => `color-mix(in oklab,var(${token}) ${percent}%,transparent)`;

export const PRESENTER_CSS = `
.sd-pw{${DARK};display:flex;flex-direction:column;gap:16px;box-sizing:border-box;height:100%;padding:16px;background-color:var(--sd-root);background-image:var(--sd-root-gradient);background-attachment:fixed;color:var(--sd-text);font:16px/1.5 ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif}
:root[data-theme=light] .sd-pw{${LIGHT}}
@media (prefers-color-scheme:light){:root:not([data-theme]) .sd-pw{${LIGHT}}}
.sd-pw *,.sd-pw ::before,.sd-pw ::after{box-sizing:border-box}
.sd-pw p{margin:0}
.sd-pw svg{display:block}
.sd-pw-header{flex:none;overflow:hidden;border-radius:12px;background-image:linear-gradient(to right,var(--sd-surface) 0%,${MIX('--sd-elevated', 40)} 50%,var(--sd-surface) 100%);box-shadow:0 20px 25px -5px rgb(0 0 0/.1),0 8px 10px -6px rgb(0 0 0/.1)}
.sd-pw-bar{display:flex;align-items:center;gap:16px;height:56px;padding:0 16px}
.sd-pw-brand{display:flex;flex:1 1 0%;align-items:center;gap:16px;min-width:0}
.sd-pw-logo{position:relative;flex-shrink:0;width:32px;height:32px}
.sd-pw-glow{position:absolute;inset:0;border-radius:9999px;background-color:${MIX('--sd-accent', 25)};filter:blur(24px)}
.sd-pw-mark,.sd-pw-mark svg{position:relative;width:100%;height:100%}
.sd-pw-titles{min-width:0}
.sd-pw-title{width:fit-content;font-size:18px;line-height:1.25;font-weight:700;letter-spacing:-.025em;background-image:var(--sd-brand);-webkit-background-clip:text;background-clip:text;color:transparent}
.sd-pw-subtitle{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:12px;line-height:16px;font-weight:500;color:var(--sd-muted)}
.sd-pw-actions{display:flex;flex-shrink:0;flex-wrap:wrap;align-items:center;gap:8px;margin-left:auto}
.sd-pw-theme{display:inline-flex;align-items:center;justify-content:center;width:36px;height:36px;margin:0;padding:0;border:0;border-radius:6px;background:transparent;color:var(--sd-muted);font:inherit;cursor:pointer;transition:color .15s cubic-bezier(.4,0,.2,1),background-color .15s cubic-bezier(.4,0,.2,1)}
.sd-pw-theme:hover{background-color:var(--sd-elevated);color:var(--sd-text)}
.sd-pw-theme:focus-visible{outline:2px solid var(--sd-ring);outline-offset:2px}
.sd-pw-theme svg{width:20px;height:20px}
.sd-pw-strip{height:4px;background-image:var(--sd-brand)}
.sd-pw-main{flex:1 1 0%;min-height:0}
.sd-pw-banner{display:flex;gap:12px;padding:12px 16px;border:1px solid ${MIX('--sd-warning', 30)};border-radius:6px;background-color:${MIX('--sd-warning', 5)};color:var(--sd-warning);font-size:14px;line-height:20px}
.sd-pw-banner svg{flex-shrink:0;width:20px;height:20px;margin-top:2px}
.sd-pw-banner>div{flex:1 1 0%;min-width:0}
.sd-pw-banner .sd-pw-banner-title{font-weight:600}
.sd-pw-banner .sd-pw-banner-text{margin-top:4px;color:var(--sd-secondary)}
.sd-pw-waiting{font-size:14px;line-height:20px;color:var(--sd-muted)}
.sd-presenter{position:relative;display:grid;grid-template-columns:minmax(0,3fr) minmax(0,2fr);grid-template-rows:auto minmax(0,1fr) minmax(0,1fr);gap:12px;box-sizing:border-box;width:100%;height:100%;padding:12px;border-radius:12px;background:#020617;color:#e2e8f0;font:14px/1.4 system-ui,sans-serif;color-scheme:dark}
.sd-presenter header{grid-column:1/-1;display:flex;flex-wrap:wrap;align-items:center;gap:8px 16px}
.sd-presenter .sd-current{grid-row:2/4;min-height:0}
.sd-presenter .sd-next{min-height:0;display:flex;flex-direction:column;gap:6px}
.sd-presenter .sd-next>div{flex:1;min-height:0;background:#000}
.sd-presenter .sd-notes{min-height:0;overflow:auto;padding:10px 12px;border-radius:10px;background:#0f172a;font-size:18px;white-space:pre-wrap}
.sd-presenter .sd-notes p{margin:0 0 .6em}
.sd-presenter .sd-label{color:#94a3b8;font-size:12px;text-transform:uppercase;letter-spacing:.06em}
.sd-presenter .sd-time{font:600 20px ui-monospace,SFMono-Regular,Menlo,monospace}
.sd-presenter button,.sd-presenter input{margin:0;font:inherit;color:inherit;background:#1e293b;border:1px solid #334155;border-radius:8px;padding:4px 10px}
.sd-presenter input{width:4em}
.sd-presenter input::placeholder{color:#94a3b8;opacity:1}
.sd-presenter button[aria-pressed=true]{background:#f97316;color:#020617}
@media (max-width:700px){.sd-presenter{grid-template-columns:1fr;grid-template-rows:auto minmax(0,2fr) minmax(0,1fr) minmax(0,1fr)}.sd-presenter .sd-current{grid-row:auto}}
`;

const injected = new WeakSet<Document>();

/** Adds the presenter window's rules to `document`, once. */
export function ensurePresenterStyle(document: Document): void {
  if (injected.has(document)) {
    return;
  }
  injected.add(document);
  const style = document.createElement('style');
  style.setAttribute('data-sd-presenter', '');
  style.textContent = PRESENTER_CSS;
  (document.head ?? document.documentElement).append(style);
}
