import { useEffect, useState } from "react";
import { api, errorText, ProgressFilter, ProgressReport, ThemeInfo } from "../api";
import { bb, GRADE_LABEL, MODE_LABEL, pct, shortDate } from "../format";

const PERIODS: { label: string; days: number | null }[] = [
  { label: "Todo o período", days: null },
  { label: "Últimos 7 dias", days: 7 },
  { label: "Últimos 30 dias", days: 30 },
];

export function Progress() {
  const [filter, setFilter] = useState<ProgressFilter>({ group_by: "theme" });
  const [period, setPeriod] = useState<number | null>(null);
  const [report, setReport] = useState<ProgressReport | null>(null);
  const [themes, setThemes] = useState<ThemeInfo[]>([]);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    void api.listThemes().then(setThemes).catch(() => undefined);
  }, []);

  useEffect(() => {
    const since = period ? new Date(Date.now() - period * 86400000).toISOString().slice(0, 19) + "Z" : undefined;
    void api
      .progress({ ...filter, since })
      .then(setReport)
      .catch((e) => setErr(errorText(e)));
  }, [filter, period]);

  const set = (k: keyof ProgressFilter) => (e: React.ChangeEvent<HTMLSelectElement | HTMLInputElement>) =>
    setFilter({ ...filter, [k]: e.target.value || undefined });

  return (
    <section>
      <h1>Progresso</h1>
      {err && <div className="alert alert-error">{err}</div>}
      <div className="filters">
        <label>
          Agrupar por
          <select value={filter.group_by} onChange={set("group_by")}>
            <option value="theme">Tema/exercício</option>
            <option value="street">Rua</option>
            <option value="position">Posição</option>
            <option value="board">Board</option>
            <option value="sampling_mode">Tipo de amostra</option>
            <option value="pack_version">Versão do pacote</option>
          </select>
        </label>
        <label>
          Amostra
          <select value={filter.mode ?? ""} onChange={set("mode")}>
            <option value="">Todas</option>
            {Object.entries(MODE_LABEL).map(([k, v]) => (
              <option key={k} value={k}>
                {v}
              </option>
            ))}
          </select>
        </label>
        <label>
          Posição
          <select value={filter.position ?? ""} onChange={set("position")}>
            <option value="">Todas</option>
            <option value="BTN">BTN</option>
            <option value="BB">BB</option>
          </select>
        </label>
        <label>
          Rua
          <select value={filter.street ?? ""} onChange={set("street")}>
            <option value="">Todas</option>
            <option value="flop">Flop</option>
            <option value="turn">Turn</option>
            <option value="river">River</option>
          </select>
        </label>
        <label>
          Tipo de pote
          <select value={filter.pot_type ?? ""} onChange={set("pot_type")}>
            <option value="">Todos</option>
            <option value="SRP">Pote simples (SRP)</option>
          </select>
        </label>
        <label>
          Textura
          <select value={filter.texture ?? ""} onChange={set("texture")}>
            <option value="">Todas</option>
            {["rainbow", "two-tone", "monotone", "paired", "connected", "A-high", "broadway-high", "middle", "low"].map((t) => (
              <option key={t} value={t}>
                {t}
              </option>
            ))}
          </select>
        </label>
        <label>
          Exercício
          <select value={filter.theme ?? ""} onChange={set("theme")}>
            <option value="">Todos</option>
            {themes.map((t) => (
              <option key={t.theme} value={t.theme}>
                {t.label}
              </option>
            ))}
          </select>
        </label>
        <label>
          Período
          <select value={period ?? ""} onChange={(e) => setPeriod(e.target.value ? Number(e.target.value) : null)}>
            {PERIODS.map((p) => (
              <option key={p.label} value={p.days ?? ""}>
                {p.label}
              </option>
            ))}
          </select>
        </label>
        <label>
          Versão do pacote
          <input placeholder="ex.: 1.0.0" value={filter.pack_version ?? ""} onChange={set("pack_version")} />
        </label>
      </div>
      {report && (
        <>
          <div className="stats-row">
            <div className="stat">
              <span className="stat-v">{report.total.n}</span>
              <span className="stat-l">decisões (n)</span>
            </div>
            <div className="stat">
              <span className="stat-v">{bb(report.total.avg_loss_bb, 3)}</span>
              <span className="stat-l">perda média de EV</span>
            </div>
            <div className="stat">
              <span className="stat-v">{pct(report.total.best_or_close_pct, 0)}</span>
              <span className="stat-l">melhor ou próxima</span>
            </div>
          </div>
          {filter.mode === undefined && (
            <p className="muted small">
              Dica: filtre por "Amostra representativa" para medir evolução; revisão e decisões difíceis são amostras de prática.
            </p>
          )}
          <table>
            <thead>
              <tr>
                <th>Grupo</th>
                <th>n</th>
                <th>Perda média</th>
                <th>Perda total</th>
                <th>Melhor/próxima</th>
              </tr>
            </thead>
            <tbody>
              {report.groups.map((g) => (
                <tr key={g.key}>
                  <td>{MODE_LABEL[g.label] ?? g.label}</td>
                  <td>{g.n}</td>
                  <td>{bb(g.avg_loss_bb, 3)}</td>
                  <td>{bb(g.total_loss_bb, 2)}</td>
                  <td>{pct(g.best_or_close_pct, 0)}</td>
                </tr>
              ))}
              {report.groups.length === 0 && (
                <tr>
                  <td colSpan={5}>Sem decisões para este filtro.</td>
                </tr>
              )}
            </tbody>
          </table>
          <h2>Histórico recente</h2>
          <table className="history">
            <thead>
              <tr>
                <th>Quando</th>
                <th>Board</th>
                <th>Mão</th>
                <th>Situação</th>
                <th>Escolha</th>
                <th>Melhor</th>
                <th>Perda</th>
                <th>Nota</th>
              </tr>
            </thead>
            <tbody>
              {report.recent.map((a) => (
                <tr key={a.id}>
                  <td>{shortDate(a.created_at)}</td>
                  <td>{a.board}</td>
                  <td>{a.combo}</td>
                  <td>{a.theme_label}</td>
                  <td>{a.chosen_label}</td>
                  <td>{a.best_label}</td>
                  <td>{bb(a.loss_bb, 3)}</td>
                  <td className={`grade-${a.grade}`}>
                    {GRADE_LABEL[a.grade]}
                    {a.in_review && " · revisão"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </section>
  );
}
