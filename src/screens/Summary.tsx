import type { SessionSummary } from "../api";
import type { Nav, Screen } from "../App";
import { bb, GRADE_LABEL, MODE_LABEL, pct } from "../format";

export function Summary({ nav, summary, returnTo }: { nav: Nav; summary: SessionSummary; returnTo?: Screen }) {
  const s = summary;
  return (
    <section>
      <h1>Resumo da sessão</h1>
      <p className="muted">{MODE_LABEL[s.session.mode]}</p>
      <div className="stats-row">
        <div className="stat">
          <span className="stat-v">{s.decisions}</span>
          <span className="stat-l">decisões</span>
        </div>
        <div className="stat">
          <span className="stat-v">{bb(s.avg_loss_bb, 3)}</span>
          <span className="stat-l">perda média de EV</span>
        </div>
        <div className="stat">
          <span className="stat-v">{bb(s.total_loss_bb, 2)}</span>
          <span className="stat-l">perda total</span>
        </div>
      </div>
      <h2>Notas</h2>
      <ul className="grades">
        {(["best", "close", "inaccuracy", "mistake"] as const).map((g) => (
          <li key={g} className={`grade-${g}`}>
            {GRADE_LABEL[g]}: <b>{s.grades[g] ?? 0}</b>
          </li>
        ))}
      </ul>
      <h2>Assuntos que mais merecem revisão</h2>
      {s.worst_themes.length === 0 ? (
        <p>Nenhuma perda relevante nesta sessão.</p>
      ) : (
        <table>
          <thead>
            <tr>
              <th>Tema</th>
              <th>n</th>
              <th>Perda média</th>
              <th>Perda total</th>
              <th>Melhor/próxima</th>
            </tr>
          </thead>
          <tbody>
            {s.worst_themes.map((t) => (
              <tr key={t.key}>
                <td>{t.label}</td>
                <td>{t.n}</td>
                <td>{bb(t.avg_loss_bb, 3)}</td>
                <td>{bb(t.total_loss_bb, 2)}</td>
                <td>{pct(t.best_or_close_pct, 0)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <div className="row">
        {returnTo ? (
          <button className="primary" onClick={() => nav.go(returnTo)}>
            Continuar a sessão de estudo
          </button>
        ) : (
          <button className="primary" onClick={() => nav.go({ name: "setup", mode: "review" })}>
            Revisar erros
          </button>
        )}
        <button onClick={() => nav.go({ name: "home" })}>Início</button>
        <button onClick={() => nav.go({ name: "progress" })}>Ver progresso</button>
      </div>
    </section>
  );
}
