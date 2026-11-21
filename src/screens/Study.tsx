import { useEffect, useMemo, useState } from "react";
import { api, errorText, StudyEntry, StudyView } from "../api";
import type { Nav, Screen } from "../App";
import { CardRow } from "../components/Cards";
import { ActionLegend, RangeGrid } from "../components/RangeGrid";
import { bb, formatLine, freq, pct, STREET_LABEL } from "../format";

export function Study({
  nav,
  exerciseId,
  highlight,
  back,
}: {
  nav: Nav;
  exerciseId?: string;
  highlight: string | null;
  back?: Screen;
}) {
  const [entries, setEntries] = useState<StudyEntry[]>([]);
  const [current, setCurrent] = useState<string | undefined>(exerciseId);
  const [view, setView] = useState<StudyView | null>(null);
  const [hand, setHand] = useState<string | null>(null);
  const [street, setStreet] = useState<string>("");
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    void api
      .listStudyEntries()
      .then((e) => {
        setEntries(e);
        if (!current && e.length) setCurrent(e[0].exercise_id);
      })
      .catch((e) => setErr(errorText(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!current) return;
    void api
      .studyNode(current, highlight)
      .then((v) => {
        setView(v);
        if (highlight) {
          const row = v.combos.find((c) => c.combo === highlight);
          setHand(row ? row.hand : null);
        } else setHand(null);
      })
      .catch((e) => setErr(errorText(e)));
  }, [current, highlight]);

  const boards = useMemo(() => Array.from(new Set(entries.map((e) => e.board))), [entries]);
  const currentEntry = entries.find((e) => e.exercise_id === current);
  const selectedBoard = currentEntry?.board;
  const selectedStreet = street || currentEntry?.street || "flop";
  const points = entries.filter((e) => e.board === selectedBoard && e.street === selectedStreet);
  const combos = view && hand ? view.combos.filter((c) => c.hand === hand) : [];

  return (
    <section className="study">
      <header className="train-head">
        <h1>Estudar</h1>
        {back && <button onClick={() => nav.go(back)}>Voltar ao treino</button>}
      </header>
      {err && <div className="alert alert-error">{err}</div>}
      <div className="row wrap">
        <label>
          Board
          <select
            value={selectedBoard ?? ""}
            onChange={(e) => {
              const first =
                entries.find((x) => x.board === e.target.value && x.street === selectedStreet) ??
                entries.find((x) => x.board === e.target.value);
              if (first) setCurrent(first.exercise_id);
            }}
          >
            {boards.map((b) => (
              <option key={b} value={b}>
                {b}
              </option>
            ))}
          </select>
        </label>
        <label>
          Rua
          <select
            value={selectedStreet}
            onChange={(e) => {
              setStreet(e.target.value);
              const first = entries.find((x) => x.board === selectedBoard && x.street === e.target.value);
              if (first) setCurrent(first.exercise_id);
            }}
          >
            {["flop", "turn", "river"]
              .filter((s) => entries.some((e) => e.board === selectedBoard && e.street === s))
              .map((s) => (
                <option key={s} value={s}>
                  {STREET_LABEL[s]}
                </option>
              ))}
          </select>
        </label>
        <label className="grow">
          Ponto de decisão ({points.length})
          <select value={current ?? ""} onChange={(e) => setCurrent(e.target.value)}>
            {points.map((e) => (
              <option key={e.exercise_id} value={e.exercise_id}>
                {e.position}: {e.line.length ? formatLine(e.line) : "primeira ação"}
              </option>
            ))}
          </select>
        </label>
      </div>
      {view && (
        <>
          <div className="study-head">
            <CardRow cards={view.board} size="md" />
            <div>
              <b>{view.theme_label}</b>
              <div className="muted">
                {view.preflop_line}
                {view.line.length > 0 && ` · ${formatLine(view.line)}`} · {view.position} decide · pote {bb(view.pot_bb)}
                {view.to_call_bb > 0 && ` · a pagar ${bb(view.to_call_bb)}`} · stack {bb(view.stack_bb)}
              </div>
            </div>
          </div>
          <div className="study-body">
            <div>
              <RangeGrid grid={view.grid} actions={view.actions} selected={hand} onSelect={setHand} />
            </div>
            <div className="study-side">
              <h2>Estratégia do range</h2>
              <ActionLegend actions={view.actions} freqs={view.aggregate} />
              {hand ? (
                <>
                  <h2>{hand}: combos</h2>
                  <table className="ev-table">
                    <thead>
                      <tr>
                        <th>Combo</th>
                        <th>Eq.</th>
                        {view.actions.map((a) => (
                          <th key={a.index} title={a.label}>
                            {a.label.split(" (")[0]}
                          </th>
                        ))}
                      </tr>
                    </thead>
                    <tbody>
                      {combos.map((c) => (
                        <tr key={c.combo} className={c.combo === view.highlight ? "chosen" : ""}>
                          <td>
                            {c.combo}
                            {c.combo === view.highlight && " (sua mão)"}
                          </td>
                          <td>{pct(c.equity * 100, 0)}</td>
                          {view.actions.map((a) => (
                            <td key={a.index}>
                              {freq(c.strategy[a.index])}
                              <br />
                              <span className="muted small">{bb(c.ev_bb[a.index], 2)}</span>
                            </td>
                          ))}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                  <p className="muted small">Em cada célula: frequência do solver e EV da ação (bb).</p>
                </>
              ) : (
                <p className="muted">Selecione uma mão na matriz para ver frequência e EV por combo e naipe.</p>
              )}
              <p className="muted small">{view.solve_info}</p>
              <p className="muted small">{view.ranges_note}</p>
            </div>
          </div>
        </>
      )}
    </section>
  );
}
