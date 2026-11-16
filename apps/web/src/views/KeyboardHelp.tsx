import { Dialog } from '@slidock/ui';

const VIEW_KEYS: [string[], string][] = [
  [['→', 'PageDown', 'Space'], 'Next slide'],
  [['←', 'PageUp'], 'Previous slide'],
  [['Home', 'End'], 'First and last slide'],
  [['↑', '↓'], 'Move through the thumbnails'],
  [['P'], 'Present'],
  [['?'], 'This help'],
];

const PRESENT_KEYS: [string[], string][] = [
  [['→', 'Space', 'PageDown', 'Enter'], 'Next build step or slide'],
  [['←', 'PageUp', 'Backspace'], 'Previous step or slide'],
  [['Home', 'End'], 'First and last slide'],
  [['3', 'Enter'], 'Go to slide 3'],
  [['B', '.'], 'Black screen'],
  [['W', ','], 'White screen'],
  [['Esc'], 'Stop presenting'],
];

function KeyTable({ caption, rows }: { caption: string; rows: [string[], string][] }) {
  return (
    <table className="w-full text-sm">
      <caption className="mb-2 text-left text-[11px] font-semibold uppercase tracking-wider text-muted">
        {caption}
      </caption>
      <tbody>
        {rows.map(([keys, action]) => (
          <tr key={action} className="border-t border-line-subtle">
            <td className="py-1.5 pr-4">
              <span className="flex flex-wrap gap-1">
                {keys.map((key) => (
                  <kbd
                    key={key}
                    className="rounded border border-line bg-elevated/60 px-1.5 font-mono text-xs text-fg-secondary"
                  >
                    {key}
                  </kbd>
                ))}
              </span>
            </td>
            <td className="py-1.5 text-fg-secondary">{action}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function KeyboardHelp({ open, onClose }: { open: boolean; onClose(): void }) {
  return (
    <Dialog open={open} onClose={onClose} title="Keyboard shortcuts">
      <KeyTable caption="Viewing" rows={VIEW_KEYS} />
      <KeyTable caption="Presenting" rows={PRESENT_KEYS} />
      <p className="text-xs text-muted">While presenting, a click on the left third of the slide goes back.</p>
    </Dialog>
  );
}
