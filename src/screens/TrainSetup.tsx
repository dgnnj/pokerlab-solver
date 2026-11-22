import { useEffect, useState } from "react";
import { api, errorText, Mode, ThemeInfo } from "../api";
import type { Nav } from "../App";
import { MODE_LABEL } from "../format";
import { PackSelect, usePreferredPack } from "../components/PackSelect";

export function TrainSetup({ nav, mode: initialMode }: { nav: Nav; mode: Mode }) {
  const packs = nav.info?.packs ?? [];
  const [pack, setPack] = usePreferredPack(nav.info?.packs, true);
  const [themes, setThemes] = useState<ThemeInfo[]>([]);
  const [theme, setTheme] = useState<string>("");
  const [count, setCount] = useState(20);
  const [mode, setMode] = useState<Mode>(initialMode);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => setMode(initialMode), [initialMode]);
  useEffect(() => {
    void api.getPref("preferred_count").then((v) => v && setCount(Number(v)));
  }, []);
  // Theme keys only mean something inside one pack: with every pack, only the street filters.
  useEffect(() => {
    setTheme((t) => (t.startsWith("street:") ? t : ""));
    if (!pack) {
      setThemes([]);
      return;
    }
    void api
      .listThemes(pack)
      .then(setThemes)
      .catch((e) => setErr(errorText(e)));
  }, [pack]);

  const start = async () => {
    try {
      const s = await api.startSession(mode, pack || null, mode === "review" ? null : theme || null, count);
      nav.go({ name: "train", sessionId: s.id });
    } catch (e) {
      setErr(errorText(e));
    }
  };

  const due = nav.info?.review_due ?? 0;

  return (
    <section>
      <h1>{mode === "review" ? "Revisar erros" : "Treinar"}</h1>
      {err && <div className="alert alert-error">{err}</div>}
      <div className="form">
        <fieldset>
          <legend>Tipo de amostra</legend>
          {(["representative", "hard", "review"] as Mode[]).map((m) => (
            <label key={m} className="radio">
              <input type="radio" name="mode" checked={mode === m} onChange={() => setMode(m)} /> {MODE_LABEL[m]}
            </label>
          ))}
          <p className="muted small">
            A amostra representativa sorteia mãos pelo peso real no ponto (use para medir evolução). "Decisões difíceis" e
            "Revisão" são para praticar e ficam separadas nas estatísticas.
          </p>
        </fieldset>
        {mode !== "review" && packs.length > 1 && (
          <PackSelect packs={packs} value={pack} onChange={setPack} allLabel="Todos os spots" />
        )}
        {mode !== "review" ? (
          <label>
            Tema
            <select value={theme} onChange={(e) => setTheme(e.target.value)}>
              <option value="">Todos os temas</option>
              <option value="street:flop">Todos do flop</option>
              <option value="street:turn">Todos do turn</option>
              <option value="street:river">Todos do river</option>
              {themes.map((t) => (
                <option key={t.theme} value={t.theme}>
                  {t.label} ({t.exercises} pontos)
                </option>
              ))}
            </select>
          </label>
        ) : (
          <p>
            {due > 0
              ? `${due} situação(ões) aguardando revisão. Repete a mesma mão quando possível; senão, outra mão do mesmo tema.`
              : "Nenhum erro pendente de revisão agora. Erros e imprecisões entram na fila automaticamente."}
          </p>
        )}
        <label>
          Número de decisões
          <input type="number" min={1} max={200} value={count} onChange={(e) => setCount(Number(e.target.value))} />
        </label>
        <button className="primary big" onClick={start} disabled={mode === "review" && due === 0}>
          Começar
        </button>
      </div>
    </section>
  );
}
