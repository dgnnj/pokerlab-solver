import { useCallback, useEffect, useRef, useState } from "react";
import { api, BoardOption, EngineStatus, errorText, HandDecision, HandHistory, HandView } from "../api";
import type { Nav } from "../App";
import { CardRow } from "../components/Cards";
import { actionClass, bb, freq, GRADE_LABEL, pct, shortDate, signedBb } from "../format";

type FeedbackMode = "each" | "end";

// Survives navigation (Study/Progress and back): the hand itself lives in the backend.
const memo: { token: string | null; solutionId: string; hero: string; fbMode: FeedbackMode } = {
  token: null,
  solutionId: "",
  hero: "random",
  fbMode: "each",
};

function splitBoard(b: string): string[] {
  const out: string[] = [];
  for (let i = 0; i + 1 < b.length; i += 2) out.push(b.slice(i, i + 2));
  return out;
}

function CardBacks() {
  return (
    <span className="card-row" aria-label="Cartas do adversário ocultas">
      <span className="card card-lg card-back" aria-hidden />
      <span className="card card-lg card-back" aria-hidden />
    </span>
  );
}

function EngineCard({
  status,
  boards,
  onPrepare,
  onCancel,
}: {
  status: EngineStatus | null;
  boards: BoardOption[];
  onPrepare: () => void;
  onCancel: () => void;
}) {
  if (!status) return null;
  const sel = boards.find((b) => b.solution_id === memo.solutionId);
  if (status.state === "solving") {
    const p = status.max_iterations > 0 ? status.iteration / status.max_iterations : 0;
    return (
      <div className="card-panel" role="status" aria-live="polite">
        <h2>Resolvendo a árvore completa do board {status.board}…</h2>
        <progress max={1} value={Math.max(p, 0.01)} />
        <p>
          Iteração {status.iteration} de até {status.max_iterations} · explorabilidade{" "}
          {status.exploitability_pct == null ? "—" : pct(status.exploitability_pct, 3)} (meta {pct(status.target_pct, 1)}{" "}
          do pote) · {Math.round(status.elapsed_ms / 1000)} s
        </p>
        <p className="muted small">
          O cálculo para quando atinge a meta (em geral bem antes do limite de iterações). A interface continua livre.
        </p>
        <button onClick={onCancel}>Cancelar</button>
      </div>
    );
  }
  if (status.state === "failed") {
    return (
      <div className="alert alert-error">
        Falha ao resolver {status.board}: {status.error}{" "}
        <button onClick={onPrepare}>Tentar de novo</button>
      </div>
    );
  }
  if (status.state === "ready") {
    return (
      <div className="card-panel">
        <h2>Board {status.board} pronto</h2>
        <p className="small">
          Explorabilidade {pct(status.exploitability_pct, 3)} do pote (pacote: {pct(status.pack_exploitability_pct, 3)}) ·
          resolvido em {Math.round(status.solve_ms / 1000)} s com {status.threads} threads · ~{status.memory_mib} MiB
          {status.compressed ? " (armazenamento comprimido)" : ""} ·{" "}
          {status.same_config_as_pack
            ? "mesma configuração do pacote (ranges, tamanhos, rake, motor)"
            : "ATENÇÃO: configuração diferente da do pacote"}
        </p>
        {sel && sel.solution_id !== status.solution_id && (
          <p>
            Board selecionado ({sel.board}) ainda não resolvido. <button onClick={onPrepare}>Resolver {sel.board}</button>
          </p>
        )}
      </div>
    );
  }
  return (
    <div className="card-panel">
      <p>
        Nenhum board resolvido nesta sessão do app.{" "}
        <button className="primary" onClick={onPrepare} disabled={!sel}>
          Resolver {sel?.board ?? "board"}
        </button>
      </p>
      {sel && (
        <p className="muted small">
          No gerador do pacote este board levou {Math.round(sel.pack_solve_ms / 1000)} s e ~{sel.memory_mib} MiB de RAM
          (referência: Ryzen 5 7600X).
        </p>
      )}
    </div>
  );
}

function DecisionDetail({ d, compact }: { d: HandDecision; compact?: boolean }) {
  const f = d.feedback;
  if (!f) {
    return (
      <div className="feedback" role="status">
        <h2>Sem avaliação</h2>
        <p>{d.not_evaluable}</p>
      </div>
    );
  }
  return (
    <div className={`feedback grade-${f.grade}`} role="status" aria-live="polite">
      <h2>
        {d.street}: {d.grade_label}
        {f.loss_bb > 0 && (
          <span className="loss">
            {" "}
            — perda {bb(f.loss_bb, 3)} ({pct(f.loss_pct_pot, 2)} do pote)
          </span>
        )}
      </h2>
      <ul>
        {d.explanation.map((t, i) => (
          <li key={i}>{t}</li>
        ))}
      </ul>
      {!compact && (
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
      )}
      {!compact && d.tolerance_bb != null && (
        <p className="muted small">
          Tolerância para "próxima": {bb(d.tolerance_bb, 3)}. EV pela convenção do solver (desistir = 0), no mesmo nó e
          para a mesma mão.
        </p>
      )}
    </div>
  );
}

function EventLog({ v }: { v: HandView }) {
  const groups: { head: string; items: string[] }[] = [{ head: `Flop ${v.board.slice(0, 3).join(" ")}`, items: [] }];
  for (const e of v.events) {
    if (e.kind === "card") groups.push({ head: e.label, items: [] });
    else groups[groups.length - 1].items.push(e.hero ? `${e.label} (você)` : e.label);
  }
  return (
    <div className="line hand-log" aria-label="Ação da mão">
      <div>
        <b>Pré-flop:</b> {v.preflop_line}
      </div>
      {groups.map((g, i) => (
        <div key={i}>
          <b>{g.head}:</b> {g.items.length ? g.items.join(", ") : "—"}
        </div>
      ))}
    </div>
  );
}

export function Hand({ nav }: { nav: Nav }) {
  const [boards, setBoards] = useState<BoardOption[]>([]);
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [hand, setHand] = useState<HandView | null>(null);
  const [hist, setHist] = useState<HandHistory | null>(null);
  const [solutionId, setSolutionId] = useState(memo.solutionId);
  const [hero, setHero] = useState(memo.hero);
  const [fbMode, setFbMode] = useState<FeedbackMode>(memo.fbMode);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [exported, setExported] = useState<string | null>(null);
  const shownAt = useRef(performance.now());

  memo.solutionId = solutionId;
  memo.hero = hero;
  memo.fbMode = fbMode;

  const loadHistory = useCallback(() => {
    void api.handHistory(15).then(setHist).catch((e) => setErr(errorText(e)));
  }, []);

  useEffect(() => {
    void (async () => {
      try {
        const bs = await api.handBoards();
        setBoards(bs);
        const st = await api.handStatus();
        setStatus(st);
        if (!memo.solutionId) {
          const fromEngine = st.state !== "idle" ? st.solution_id : null;
          setSolutionId(fromEngine ?? bs[0]?.solution_id ?? "");
        }
        if (memo.token) {
          try {
            setHand(await api.handView(memo.token));
          } catch {
            memo.token = null;
          }
        }
      } catch (e) {
        setErr(errorText(e));
      }
    })();
    loadHistory();
  }, [loadHistory]);

  // Poll while solving.
  useEffect(() => {
    if (status?.state !== "solving") return;
    const id = window.setInterval(() => {
      void api
        .handStatus()
        .then(setStatus)
        .catch(() => undefined);
    }, 500);
    return () => window.clearInterval(id);
  }, [status?.state]);

  const prepare = useCallback(async () => {
    setErr(null);
    try {
      setStatus(await api.handPrepare(memo.solutionId));
    } catch (e) {
      setErr(errorText(e));
    }
  }, []);

  const cancel = useCallback(async () => {
    setStatus(await api.handCancel());
  }, []);

  const newHand = useCallback(async () => {
    setErr(null);
    setBusy(true);
    try {
      const v = await api.handStart(memo.hero);
      memo.token = v.token;
      setHand(v);
      shownAt.current = performance.now();
      if (v.finished) loadHistory();
    } catch (e) {
      setErr(errorText(e));
    } finally {
      setBusy(false);
    }
  }, [loadHistory]);

  const act = useCallback(
    async (index: number) => {
      if (!hand || hand.finished || busy) return;
      setBusy(true);
      try {
        const ms = Math.round(performance.now() - shownAt.current);
        const v = await api.handAct(hand.token, index, ms);
        setHand(v);
        shownAt.current = performance.now();
        if (v.finished) loadHistory();
      } catch (e) {
        setErr(errorText(e));
      } finally {
        setBusy(false);
      }
    },
    [hand, busy, loadHistory],
  );

  const leave = useCallback(() => {
    memo.token = null;
    setHand(null);
    loadHistory();
  }, [loadHistory]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement) return;
      if (hand && !hand.finished) {
        const n = Number(e.key);
        if (n >= 1 && n <= hand.actions.length) void act(n - 1);
      } else if (hand?.finished && (e.key === "Enter" || e.key === " ")) {
        e.preventDefault();
        void newHand();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [hand, act, newHand]);

  const ready = status?.state === "ready";
  const readyBoard = ready ? status.board : null;

  // ---------- Active hand ----------
  if (hand) {
    const last = hand.decisions[hand.decisions.length - 1];
    const o = hand.outcome;
    const showFeedback = fbMode === "each" || hand.finished;
    return (
      <section className="train">
        <header className="train-head">
          <div>
            <h1>Mão completa</h1>
            <span className="muted">
              Você é {hand.hero_position} contra {hand.villain_position} · board inicial {hand.board.slice(0, 3).join(" ")} ·{" "}
              {hand.summary.decisions} decisão(ões)
            </span>
          </div>
          <div className="row">
            {hand.finished ? (
              <button className="primary" onClick={() => void newHand()} disabled={busy || !ready} autoFocus>
                Nova mão <span className="kbd">Enter</span>
              </button>
            ) : null}
            <button onClick={leave}>{hand.finished ? "Voltar" : "Abandonar mão"}</button>
          </div>
        </header>
        {err && <div className="alert alert-error">{err}</div>}

        <div className="table-felt" aria-label="Mesa">
          <div className="villain hero">
            <span className="seat">{hand.villain_position}</span>
            {hand.villain_cards ? <CardRow cards={hand.villain_cards} size="lg" /> : <CardBacks />}
            <span className="stack">Stack: {bb(hand.villain_stack_bb)}</span>
          </div>
          <div className="board" aria-label={`Board no ${hand.street}`}>
            <CardRow cards={hand.board} size="lg" />
            <div className="pot">
              Pote: <b>{bb(hand.pot_bb)}</b>
              {hand.to_call_bb > 0 && !hand.finished && (
                <>
                  {" "}
                  · a pagar <b>{bb(hand.to_call_bb)}</b> (equidade necessária{" "}
                  {pct((100 * hand.to_call_bb) / (hand.pot_bb + hand.to_call_bb))})
                </>
              )}
            </div>
          </div>
          <div className="hero">
            <span className="seat hero-seat">Você: {hand.hero_position}</span>
            <CardRow cards={hand.hero_cards} size="lg" />
            <span className="stack">Stack: {bb(hand.hero_stack_bb)}</span>
          </div>
        </div>

        <EventLog v={hand} />

        {!hand.finished && (
          <div className="actions" role="group" aria-label="Suas opções">
            {hand.actions.map((a) => (
              <button
                key={a.index}
                className={`action ${actionClass(a.kind, a.index)}`}
                onClick={() => void act(a.index)}
                disabled={busy}
                aria-keyshortcuts={String(a.index + 1)}
              >
                <span className="kbd">{a.index + 1}</span> {a.label}
              </button>
            ))}
          </div>
        )}

        {!hand.finished && showFeedback && last && (
          <>
            <p className="muted small">Sua decisão anterior ({last.chosen_label}):</p>
            <DecisionDetail d={last} />
          </>
        )}
        {!hand.finished && fbMode === "end" && hand.decisions.length > 0 && (
          <p className="muted small">Notas ocultas até o fim da mão.</p>
        )}

        {hand.finished && o && (
          <>
            <div className={`card-panel highlight result-${o.winner}`} role="status">
              <h2>{o.summary}</h2>
              <div className="stats-row">
                <div className="stat">
                  <span className="stat-v">{signedBb(o.hero_net_bb)}</span>
                  <span className="stat-l">Resultado no pós-flop (pote de {bb(o.pot_bb)})</span>
                </div>
                {o.hero_total_bb != null && (
                  <div className="stat">
                    <span className="stat-v">{signedBb(o.hero_total_bb)}</span>
                    <span className="stat-l">Na mão, contando o pré-flop</span>
                  </div>
                )}
                <div className="stat">
                  <span className="stat-v">{bb(hand.summary.total_loss_bb, 3)}</span>
                  <span className="stat-l">
                    Perda de EV nas suas decisões ({hand.summary.graded} de {hand.summary.decisions} avaliadas)
                  </span>
                </div>
              </div>
              {o.showdown && (
                <p>
                  Você: {o.hero_hand} · Adversário ({hand.villain_cards?.join("")}): {o.villain_hand}
                </p>
              )}
              {o.rake_bb > 0 && <p className="small">Rake: {bb(o.rake_bb)}</p>}
              <p className="muted small">
                O resultado é a sorte da mão; a perda de EV mede a qualidade das decisões contra a estratégia do motor.
                Uma mão ganha com erros continua com erros.
              </p>
            </div>
            {hand.cut && <div className="alert">{hand.cut}</div>}
            <h2>Suas decisões</h2>
            {hand.decisions.length === 0 && <p className="muted">A mão terminou sem decisões suas.</p>}
            {hand.decisions.map((d) => (
              <details key={d.seq} className="decision-review" open={d.feedback?.grade === "mistake" || d.feedback?.grade === "inaccuracy"}>
                <summary>
                  <b>{d.street}</b> {d.board.join(" ")} · pote {bb(d.pot_bb)} · você: {d.chosen_label} ·{" "}
                  <span className={d.feedback ? `grade-${d.feedback.grade}` : ""}>
                    {d.feedback ? GRADE_LABEL[d.feedback.grade] : "sem avaliação"}
                    {d.feedback && d.feedback.loss_bb > 0 ? ` (−${bb(d.feedback.loss_bb, 3)})` : ""}
                  </span>
                </summary>
                <DecisionDetail d={d} />
              </details>
            ))}
          </>
        )}
      </section>
    );
  }

  // ---------- Setup ----------
  const s = hist?.stats;
  return (
    <section>
      <h1>Mão completa</h1>
      <p className="muted">
        Jogue uma mão do flop ao river contra a estratégia do motor: as duas mãos são sorteadas pelos ranges, o adversário age
        pelas frequências da mão dele em cada ponto, turn e river saem do baralho restante. Cada decisão sua é avaliada pela
        perda de EV.
      </p>
      {err && <div className="alert alert-error">{err}</div>}
      <div className="form">
        <label>
          Board (flop)
          <select value={solutionId} onChange={(e) => setSolutionId(e.target.value)}>
            {boards.map((b) => (
              <option key={b.solution_id} value={b.solution_id}>
                {b.board}
                {readyBoard === b.board ? " — pronto" : ""}
              </option>
            ))}
          </select>
        </label>
        <div className="row">
          {solutionId && (
            <CardRow cards={splitBoard(boards.find((b) => b.solution_id === solutionId)?.board ?? "")} size="md" />
          )}
          <button
            onClick={() => {
              const others = boards.filter((b) => b.solution_id !== solutionId);
              if (others.length) setSolutionId(others[Math.floor(Math.random() * others.length)].solution_id);
            }}
          >
            Sortear outro board
          </button>
        </div>
        <fieldset>
          <legend>Sua posição</legend>
          {[
            ["random", "Aleatória"],
            ["BB", "BB (fora de posição)"],
            ["BTN", "BTN (em posição)"],
          ].map(([v, l]) => (
            <label key={v} className="radio">
              <input type="radio" name="hero" checked={hero === v} onChange={() => setHero(v)} /> {l}
            </label>
          ))}
        </fieldset>
        <fieldset>
          <legend>Notas</legend>
          <label className="radio">
            <input type="radio" name="fb" checked={fbMode === "each"} onChange={() => setFbMode("each")} /> Depois de cada
            decisão
          </label>
          <label className="radio">
            <input type="radio" name="fb" checked={fbMode === "end"} onChange={() => setFbMode("end")} /> Só no fim da mão
          </label>
        </fieldset>
      </div>

      <EngineCard status={status} boards={boards} onPrepare={() => void prepare()} onCancel={() => void cancel()} />

      {ready && (
        <button className="primary big" onClick={() => void newHand()} disabled={busy}>
          Jogar mão no board {readyBoard}
        </button>
      )}
      <p className="muted small">
        Uma mão inteira precisa da árvore completa do flop (todas as cartas de turn e river), que não cabe no instalador. O
        app a resolve aqui, com a mesma configuração do pacote, e a mantém na memória enquanto estiver aberto: as mãos
        seguintes no mesmo board são instantâneas. Trocar de board descarta a árvore anterior.
      </p>

      <h2>Histórico de mãos</h2>
      {s && s.hands > 0 ? (
        <>
          <div className="stats-row">
            <div className="stat">
              <span className="stat-v">{s.finished}</span>
              <span className="stat-l">Mãos concluídas{s.hands > s.finished ? ` (${s.hands - s.finished} interrompidas)` : ""}</span>
            </div>
            <div className="stat">
              <span className="stat-v">{bb(s.avg_loss_per_decision_bb, 3)}</span>
              <span className="stat-l">Perda média por decisão ({s.graded} avaliadas)</span>
            </div>
            <div className="stat">
              <span className="stat-v">{bb(s.avg_loss_per_hand_bb, 3)}</span>
              <span className="stat-l">Perda média de EV por mão</span>
            </div>
            <div className="stat">
              <span className="stat-v">{signedBb(s.net_bb)}</span>
              <span className="stat-l">Resultado somado no pós-flop (sorte incluída)</span>
            </div>
          </div>
          {hist!.by_position.length > 1 && (
            <p className="small">
              {hist!.by_position
                .map(([p, st]) => `${p}: ${st.finished} mãos, perda média ${bb(st.avg_loss_per_decision_bb, 3)} por decisão`)
                .join(" · ")}
            </p>
          )}
          <table>
            <thead>
              <tr>
                <th>Quando</th>
                <th>Posição</th>
                <th>Mãos</th>
                <th>Board</th>
                <th>Resultado</th>
                <th>Perda de EV</th>
              </tr>
            </thead>
            <tbody>
              {hist!.recent.map((r) => (
                <tr key={r.id} title={r.line ?? ""}>
                  <td>{shortDate(r.started_at)}</td>
                  <td>{r.hero_position}</td>
                  <td>
                    {r.hero_combo} x {r.villain_combo}
                  </td>
                  <td>{r.final_board ?? r.flop}</td>
                  <td>{r.ended_at ? signedBb(r.hero_net_bb ?? 0) : "interrompida"}</td>
                  <td>
                    {bb(r.total_loss_bb, 3)} ({r.graded}/{r.decisions})
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="muted small">
            Resultado em bb no pós-flop (o pote inicial de 5,5 bb conta para quem leva a mão). Perda de EV somada nas decisões
            avaliadas, com o número de decisões avaliadas/total.
          </p>
          <div className="row">
            <button
              onClick={() =>
                void api
                  .exportHandsCsv()
                  .then(setExported)
                  .catch((e) => setErr(errorText(e)))
              }
            >
              Exportar mãos completas (CSV)
            </button>
          </div>
          {exported && (
            <p className="small">{exported}</p>
          )}
        </>
      ) : (
        <p className="muted">Nenhuma mão jogada ainda.</p>
      )}
      <p className="muted small">
        <button onClick={() => nav.go({ name: "progress" })}>Ver progresso dos exercícios</button>
      </p>
    </section>
  );
}
