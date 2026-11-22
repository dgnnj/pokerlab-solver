import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorText, FeedbackView, Presented } from "../api";
import type { Nav, Screen } from "../App";
import { CardRow } from "../components/Cards";
import { actionClass, bb, formatLine, freq, MODE_LABEL, pct } from "../format";

// Keeps the current exercise/feedback per session so that switching to Study and back
// restores the same decision instead of drawing a new one.
const cache = new Map<number, { ex: Presented; fb: FeedbackView | null; marked: boolean }>();

export function Train({ nav, sessionId, returnTo }: { nav: Nav; sessionId: number; returnTo?: Screen }) {
  const [ex, setEx] = useState<Presented | null>(null);
  const [fb, setFb] = useState<FeedbackView | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [marked, setMarked] = useState(false);
  const shownAt = useRef(performance.now());

  const finish = useCallback(async () => {
    cache.delete(sessionId);
    const summary = await api.endSession(sessionId);
    await nav.refresh();
    nav.go({ name: "summary", summary, returnTo });
  }, [sessionId, nav, returnTo]);

  const loadNext = useCallback(async () => {
    setErr(null);
    setFb(null);
    setMarked(false);
    try {
      const next = await api.nextExercise(sessionId);
      if (!next) {
        await finish();
        return;
      }
      setEx(next);
      shownAt.current = performance.now();
    } catch (e) {
      setErr(errorText(e));
    }
  }, [sessionId, finish]);

  useEffect(() => {
    const c = cache.get(sessionId);
    if (c) {
      setEx(c.ex);
      setFb(c.fb);
      setMarked(c.marked);
      shownAt.current = performance.now();
    } else {
      void loadNext();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  useEffect(() => {
    if (ex) cache.set(sessionId, { ex, fb, marked });
  }, [ex, fb, marked, sessionId]);

  const choose = useCallback(
    async (index: number) => {
      if (!ex || fb || busy) return;
      setBusy(true);
      try {
        const ms = Math.round(performance.now() - shownAt.current);
        setFb(await api.submitAnswer(ex.token, index, ms));
      } catch (e) {
        setErr(errorText(e));
      } finally {
        setBusy(false);
      }
    },
    [ex, fb, busy],
  );

  const markReview = useCallback(async () => {
    if (!fb) return;
    await api.markReview(fb.attempt_id, "manual");
    setMarked(true);
  }, [fb]);

  const openStudy = useCallback(() => {
    if (!ex) return;
    nav.go({
      name: "study",
      exerciseId: ex.exercise_id,
      highlight: fb?.feedback.combo ?? null,
      back: { name: "train", sessionId, returnTo },
    });
  }, [ex, fb, nav, sessionId, returnTo]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      if (!fb && ex) {
        const n = Number(e.key);
        if (n >= 1 && n <= ex.actions.length) void choose(n - 1);
      } else if (fb) {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          void loadNext();
        } else if (e.key.toLowerCase() === "r") void markReview();
        else if (e.key.toLowerCase() === "e") openStudy();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [ex, fb, choose, loadNext, markReview, openStudy]);

  if (!ex) {
    return (
      <section>
        {err ? <div className="alert alert-error">{err}</div> : <p>Carregando exercício…</p>}
        {err && <button onClick={() => nav.go({ name: "home" })}>Voltar ao início</button>}
      </section>
    );
  }

  const s = fb?.session ?? ex.session;
  const f = fb?.feedback;

  return (
    <section className="train">
      <header className="train-head">
        <div>
          <h1>{ex.theme_label}</h1>
          <span className="muted">
            {ex.pack_name} · {MODE_LABEL[ex.mode]} · decisão {Math.min(s.done + (fb ? 0 : 1), s.target_count)} de {s.target_count}
            {ex.review_note && ` · ${ex.review_note}`}
          </span>
        </div>
        <button onClick={finish}>Encerrar sessão</button>
      </header>
      {err && <div className="alert alert-error">{err}</div>}

      <div className="table-felt" aria-label="Mesa">
        <div className="villain">
          <span className="seat">{ex.villain_position}</span>
        </div>
        <div className="board" aria-label={`Board no ${ex.street}`}>
          <CardRow cards={ex.board} size="lg" />
          <div className="pot">
            Pote: <b>{bb(ex.pot_bb)}</b>
            {ex.to_call_bb > 0 && (
              <>
                {" "}
                · a pagar <b>{bb(ex.to_call_bb)}</b> (equidade necessária {pct((100 * ex.to_call_bb) / (ex.pot_bb + ex.to_call_bb))})
              </>
            )}
          </div>
        </div>
        <div className="hero">
          <span className="seat hero-seat">Você: {ex.hero_position}</span>
          <CardRow cards={ex.hero_cards} size="lg" />
          <span className="stack">Stack restante: {bb(ex.stack_bb)}</span>
        </div>
      </div>

      <div className="line">
        <b>Ação até aqui:</b> {ex.preflop_line}
        {ex.line.length > 0 && <> · {formatLine(ex.line)}</>}
        {ex.line.length === 0 && <> · Primeira ação no {ex.street}</>}
      </div>

      <div className="actions" role="group" aria-label="Suas opções">
        {ex.actions.map((a) => {
          const res = f?.actions[a.index];
          const isBest = f && f.best_index === a.index;
          const isChosen = f && f.chosen_index === a.index;
          return (
            <button
              key={a.index}
              className={`action ${actionClass(a.kind, a.index)} ${isBest ? "is-best" : ""} ${isChosen ? "is-chosen" : ""}`}
              onClick={() => choose(a.index)}
              disabled={!!fb || busy}
              aria-keyshortcuts={String(a.index + 1)}
            >
              <span className="kbd">{a.index + 1}</span> {a.label}
              {res && (
                <span className="action-res">
                  EV {bb(res.ev_bb, 3)} · freq {freq(res.frequency)}
                  {res.loss_bb > 0 ? ` · perde ${bb(res.loss_bb, 3)}` : " · melhor EV"}
                </span>
              )}
            </button>
          );
        })}
      </div>

      {fb && f && (
        <div className={`feedback grade-${f.grade}`} role="status" aria-live="polite">
          <h2>
            {fb.grade_label}
            {f.loss_bb > 0 && (
              <span className="loss">
                {" "}
                — perda {bb(f.loss_bb, 3)} ({pct(f.loss_pct_pot, 2)} do pote)
              </span>
            )}
          </h2>
          <ul>
            {fb.explanation.map((t, i) => (
              <li key={i}>{t}</li>
            ))}
          </ul>
          <table className="ev-table">
            <thead>
              <tr>
                <th>Ação</th>
                <th>Frequência do solver ({f.combo})</th>
                <th>EV</th>
                <th>Custo vs. melhor</th>
              </tr>
            </thead>
            <tbody>
              {f.actions.map((a) => (
                <tr key={a.index} className={a.index === f.chosen_index ? "chosen" : ""}>
                  <td>
                    {a.label}
                    {a.index === f.chosen_index && " (sua escolha)"}
                    {a.index === f.best_index && " ★"}
                  </td>
                  <td>{freq(a.frequency)}</td>
                  <td>{bb(a.ev_bb, 3)}</td>
                  <td>{a.loss_bb > 0 ? bb(a.loss_bb, 3) : "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="muted small">
            Tolerância para "próxima": {bb(fb.tolerance_bb, 3)}. EV pela convenção do solver (desistir = 0), em bb, no mesmo
            nó e para a mesma mão.
          </p>
          <div className="row">
            <button className="primary" onClick={() => void loadNext()} autoFocus>
              Próxima <span className="kbd">Enter</span>
            </button>
            <button onClick={markReview} disabled={marked}>
              {marked ? "Marcado para revisão" : fb.in_review ? "Já está na revisão (marcar tag)" : "Marcar para revisão"}{" "}
              <span className="kbd">R</span>
            </button>
            <button onClick={openStudy}>
              Abrir em Estudar <span className="kbd">E</span>
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
