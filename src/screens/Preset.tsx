import { useEffect, useState } from "react";
import { api, errorText, ThemeInfo } from "../api";
import type { Nav } from "../App";
import { shortDate } from "../format";
import { PackSelect, usePreferredPack } from "../components/PackSelect";

/** Study preset (~30 min): 10 min review, 15 min single theme, 5 min writing a practical rule. */
export function Preset({ nav, step }: { nav: Nav; step: number }) {
  const packs = nav.info?.packs ?? [];
  const [pack, setPack] = usePreferredPack(nav.info?.packs, false);
  const [themes, setThemes] = useState<ThemeInfo[]>([]);
  const [theme, setTheme] = useState("");
  const [note, setNote] = useState("");
  const [notes, setNotes] = useState<[string, string][]>([]);
  const [err, setErr] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    void api.listNotes().then(setNotes).catch(() => undefined);
  }, []);
  useEffect(() => {
    if (!pack) return;
    void api.listThemes(pack).then((t) => {
      setThemes(t);
      setTheme(t.length ? t[0].theme : "");
    });
  }, [pack]);

  const due = nav.info?.review_due ?? 0;

  const run = async (mode: "review" | "representative", count: number, th: string | null, nextStep: number) => {
    try {
      const s = await api.startSession(mode, mode === "review" ? null : pack, th, count);
      nav.go({ name: "train", sessionId: s.id, returnTo: { name: "preset", step: nextStep } });
    } catch (e) {
      setErr(errorText(e));
    }
  };

  return (
    <section>
      <h1>Sessão de estudo (~30 min)</h1>
      {err && <div className="alert alert-error">{err}</div>}
      <ol className="steps">
        <li className={step === 0 ? "current" : step > 0 ? "done" : ""}>
          <b>1. Revisão de erros (~10 min)</b>
          <p className="muted">{due > 0 ? `${due} item(ns) pendente(s).` : "Nada pendente — pule para o passo 2."}</p>
          {step === 0 && (
            <div className="row">
              <button className="primary" disabled={due === 0} onClick={() => run("review", Math.min(Math.max(due, 1), 12), null, 1)}>
                Começar revisão
              </button>
              <button onClick={() => nav.go({ name: "preset", step: 1 })}>Pular</button>
            </div>
          )}
        </li>
        <li className={step === 1 ? "current" : step > 1 ? "done" : ""}>
          <b>2. Treino de um único tema (~15 min, 20 decisões)</b>
          {step === 1 && packs.length > 1 && <PackSelect packs={packs} value={pack} onChange={setPack} />}
          {step === 1 && (
            <div className="row">
              <select value={theme} onChange={(e) => setTheme(e.target.value)} aria-label="Tema">
                {themes.map((t) => (
                  <option key={t.theme} value={t.theme}>
                    {t.label}
                  </option>
                ))}
              </select>
              <button className="primary" onClick={() => run("representative", 20, theme, 2)}>
                Treinar tema
              </button>
              <button onClick={() => nav.go({ name: "preset", step: 2 })}>Pular</button>
            </div>
          )}
        </li>
        <li className={step === 2 ? "current" : ""}>
          <b>3. Registrar uma regra prática (~5 min)</b>
          {step === 2 && (
            <div>
              <textarea
                rows={3}
                value={note}
                onChange={(e) => {
                  setNote(e.target.value);
                  setSaved(false);
                }}
                placeholder="Ex.: Em boards baixos conectados, o BB defende mais contra c-bet pequena."
                aria-label="Regra prática para testar nas próximas sessões"
              />
              <div className="row">
                <button
                  className="primary"
                  disabled={!note.trim()}
                  onClick={async () => {
                    try {
                      await api.addNote(null, note);
                      setSaved(true);
                      setNote("");
                      setNotes(await api.listNotes());
                    } catch (e) {
                      setErr(errorText(e));
                    }
                  }}
                >
                  Salvar regra
                </button>
                <button onClick={() => nav.go({ name: "home" })}>Concluir</button>
              </div>
              {saved && <p role="status">Regra salva.</p>}
            </div>
          )}
        </li>
      </ol>
      {notes.length > 0 && (
        <>
          <h2>Regras registradas</h2>
          <ul>
            {notes.map(([d, t], i) => (
              <li key={i}>
                <span className="muted small">{shortDate(d)}</span> — {t}
              </li>
            ))}
          </ul>
        </>
      )}
    </section>
  );
}
