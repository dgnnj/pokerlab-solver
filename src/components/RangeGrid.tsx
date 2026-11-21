import { ActionOption, GridCell } from "../api";
import { actionClass, freq } from "../format";

/** 13x13 strategy grid. Each cell shows stacked bars per action; the dominant action is also
 *  written as text so the information does not depend on color alone. */
export function RangeGrid({
  grid,
  actions,
  selected,
  onSelect,
}: {
  grid: GridCell[];
  actions: ActionOption[];
  selected: string | null;
  onSelect: (hand: string) => void;
}) {
  const maxW = Math.max(...grid.map((c) => c.weight), 1e-9);
  return (
    <div className="range-grid" role="grid" aria-label="Estratégia por mão (13x13)">
      {grid.map((c) => {
        const empty = c.weight <= 0;
        let dom = -1;
        if (!empty) dom = c.strategy.reduce((bi, v, i, a) => (v > a[bi] ? i : bi), 0);
        const label = empty
          ? `${c.hand}: fora do range`
          : `${c.hand}: ${actions.map((a, i) => `${a.label} ${freq(c.strategy[i])}`).join(", ")}`;
        return (
          <button
            key={c.hand}
            role="gridcell"
            className={`rg-cell ${empty ? "rg-empty" : ""} ${selected === c.hand ? "rg-sel" : ""}`}
            title={label}
            aria-label={label}
            disabled={empty}
            onClick={() => onSelect(c.hand)}
            style={{ opacity: empty ? 1 : 0.45 + 0.55 * Math.sqrt(c.weight / maxW) }}
          >
            {!empty && (
              <span className="rg-bars" aria-hidden>
                {c.strategy.map((s, i) => (
                  <span key={i} className={`rg-bar ${actionClass(actions[i].kind, i)}`} style={{ width: `${s * 100}%` }} />
                ))}
              </span>
            )}
            <span className="rg-label">{c.hand}</span>
            {!empty && dom >= 0 && <span className="rg-dom">{shortAction(actions[dom])}</span>}
          </button>
        );
      })}
    </div>
  );
}

export function shortAction(a: ActionOption): string {
  switch (a.kind) {
    case "fold":
      return "F";
    case "check":
      return "X";
    case "call":
      return "C";
    case "allin":
      return "AI";
    default: {
      const m = a.label.match(/(\d+)% do pote/);
      return m ? `B${m[1]}` : a.kind === "raise" ? "R" : "B";
    }
  }
}

export function ActionLegend({ actions, freqs }: { actions: ActionOption[]; freqs?: number[] }) {
  return (
    <ul className="legend">
      {actions.map((a, i) => (
        <li key={i}>
          <span className={`swatch ${actionClass(a.kind, i)}`} aria-hidden /> <b>{shortAction(a)}</b> {a.label}
          {freqs && <span className="muted"> — {freq(freqs[i])}</span>}
        </li>
      ))}
    </ul>
  );
}
