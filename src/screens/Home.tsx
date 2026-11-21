import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import type { Nav } from "../App";
import { MODE_LABEL } from "../format";

export function Home({ nav }: { nav: Nav }) {
  const info = nav.info;
  const [err, setErr] = useState<string | null>(null);
  const [profile, setProfile] = useState<string | null>(null);

  useEffect(() => {
    void api.getPref("preferred_count").then(setProfile).catch(() => setProfile("20"));
  }, []);

  const quick = async (count: number) => {
    try {
      const s = await api.startSession("representative", null, count);
      nav.go({ name: "train", sessionId: s.id });
    } catch (e) {
      setErr(errorText(e));
    }
  };

  const chooseProfile = async (count: string) => {
    await api.setPref("preferred_count", count);
    setProfile(count);
  };

  return (
    <section>
      <h1>Início</h1>
      {err && <div className="alert alert-error">{err}</div>}
      {info?.health.updated && (
        <div className="alert alert-ok" role="status">
          Atualizado de {info.health.previous_version} para {info.health.version}. Verificação de saúde:{" "}
          {info.health.checks.join("; ")}.
        </div>
      )}
      {info?.health.first_run && !profile && (
        <div className="card-panel highlight">
          <h2>Bem-vindo!</h2>
          <p>
            O pacote inicial <b>{info.pack_title}</b> foi instalado e validado ({info.pack_exercises} exercícios, offline).
            Escolha seu perfil de treino:
          </p>
          <div className="row">
            <button onClick={() => chooseProfile("10")}>Curto — 10 decisões</button>
            <button onClick={() => chooseProfile("20")}>Padrão — 20 decisões</button>
            <button onClick={() => chooseProfile("30")}>Longo — 30 decisões</button>
          </div>
          <p>
            <button className="primary" onClick={() => quick(5)}>
              Fazer um exercício curto agora (5 decisões)
            </button>
          </p>
        </div>
      )}
      {info?.open_session && (
        <div className="card-panel highlight">
          <h2>Continuar sessão</h2>
          <p>
            {MODE_LABEL[info.open_session.mode]} · {info.open_session.done} de {info.open_session.target_count} decisões
          </p>
          <button className="primary" onClick={() => nav.go({ name: "train", sessionId: info.open_session!.id })}>
            Continuar de onde parei
          </button>
        </div>
      )}
      <div className="tiles">
        <button className="tile" onClick={() => nav.go({ name: "setup", mode: "representative" })}>
          <b>Treinar</b>
          <span>Escolha um tema e o número de decisões.</span>
        </button>
        <button className="tile" onClick={() => nav.go({ name: "hand" })}>
          <b>Mão completa</b>
          <span>Do flop ao river contra a estratégia do motor, com nota em cada decisão.</span>
        </button>
        <button className="tile" onClick={() => nav.go({ name: "study" })}>
          <b>Estudar</b>
          <span>Explore a estratégia completa de cada ponto.</span>
        </button>
        <button className="tile" onClick={() => nav.go({ name: "setup", mode: "review" })}>
          <b>Revisar erros</b>
          <span>{info ? `${info.review_due} situação(ões) para revisar agora.` : "…"}</span>
        </button>
        <button className="tile" onClick={() => nav.go({ name: "progress" })}>
          <b>Progresso</b>
          <span>Perda média de EV por tema, com tamanho da amostra.</span>
        </button>
        <button className="tile" onClick={() => nav.go({ name: "preset", step: 0 })}>
          <b>Sessão de estudo (~30 min)</b>
          <span>10 min de revisão, 15 min de um tema, 5 min para anotar uma regra prática.</span>
        </button>
        <button className="tile" onClick={() => quick(Number(profile ?? "20"))}>
          <b>Treino rápido</b>
          <span>{profile ?? "20"} decisões de todos os temas (amostra representativa).</span>
        </button>
      </div>
      {info && (
        <p className="muted small">
          Pacote {info.pack_id} v{info.pack_version} · motor postflop-solver @ {info.engine_rev.slice(0, 10)} · dados em{" "}
          {info.data_dir}
        </p>
      )}
    </section>
  );
}
