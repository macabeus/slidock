// One line per lane for the overview, from a run's summary: the lane's own checks summed over frontends.
import type { FrameSummary, Lane, Summary } from '../schema';
import { totals } from './overview';

export interface LaneFigure {
  label: string;
  /** Rows that pass the lane's check. */
  pass: number;
  rows: number;
}

function frames(byFrontend: { [frontend: string]: FrameSummary | undefined } | undefined): LaneFigure | null {
  if (!byFrontend) {
    return null;
  }
  const parts = Object.values(byFrontend).filter((part): part is FrameSummary => part !== undefined);
  return {
    label: 'identical',
    pass: parts.reduce((sum, part) => sum + part.identical, 0),
    rows: parts.reduce((sum, part) => sum + part.rows, 0),
  };
}

/** The figures a lane's summary reports, its headline first; empty when the summary lacks the lane's section. */
export function laneFigures(lane: Lane, summary: Summary): LaneFigure[] {
  const lanes = summary.lanes;
  switch (lane) {
    case 'static': {
      const total = totals(summary);
      return total ? [{ label: 'match', pass: total.match, rows: total.rows }] : [];
    }
    case 'host': {
      const figure = frames(lanes?.host);
      return figure ? [figure] : [];
    }
    case 'shell': {
      const figure = frames(lanes?.shell);
      return figure ? [figure] : [];
    }
    case 'fonts': {
      const parts = Object.values(lanes?.fonts ?? {}).filter((part) => part !== undefined);
      if (parts.length === 0) {
        return [];
      }
      const sum = (pick: (part: (typeof parts)[number]) => number) => parts.reduce((s, part) => s + pick(part), 0);
      return [
        { label: 'fonts complete', pass: sum((p) => p.complete), rows: sum((p) => p.rows) },
        { label: 'faces resolved', pass: sum((p) => p.faces), rows: sum((p) => p.faces + p.unresolved) },
      ];
    }
    case 'motion': {
      const parts = Object.values(lanes?.motion ?? {}).filter((part) => part !== undefined);
      if (parts.length === 0) {
        return [];
      }
      const sum = (pick: (part: (typeof parts)[number]) => number) => parts.reduce((s, part) => s + pick(part), 0);
      const effects = sum((p) => p.effects.native + p.effects.fallback + p.effects.unsupported);
      return [
        { label: 'end states identical', pass: sum((p) => p.endState.identical), rows: sum((p) => p.endState.rows) },
        {
          label: 'transition frames match',
          pass: sum((p) => p.transitions['motion-match']),
          rows: sum((p) => p.transitions.rows),
        },
        { label: 'effects native', pass: sum((p) => p.effects.native), rows: effects },
      ];
    }
  }
}
