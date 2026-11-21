import { useCallback, useEffect, useRef, useState } from "react";
import { api, AppInfo, errorText, UpdateCheck } from "../api";

type Phase =
  | { k: "loading" }
  | { k: "checking" }
  | { k: "skip"; reason: string }
  | { k: "available"; u: UpdateCheck }
  | { k: "installing"; done: number; total: number | null }
  | { k: "failed"; msg: string };

const CHECK_TIMEOUT_MS = 6000;

/** Opening step of the app: checks the signed update feed with a short deadline and lets the
 *  user start training immediately. Training never depends on the network. */
export function Launcher({ info, error, onDone: onDoneProp }: { info: AppInfo | null; error: string | null; onDone: () => void }) {
  const [phase, setPhase] = useState<Phase>({ k: "loading" });
  const started = useRef(false);
  const finished = useRef(false);
  const onDoneRef = useRef(onDoneProp);
  onDoneRef.current = onDoneProp;
  // Idempotent: auto-continue timers and the "Treinar agora" button may both fire.
  const onDone = useCallback(() => {
    if (finished.current) return;
    finished.current = true;
    onDoneRef.current();
  }, []);

  useEffect(() => {
    if (!info || started.current) return;
    started.current = true;
    void (async () => {
      const auto = (await api.getPref("auto_update").catch(() => "true")) !== "false";
      const postponed = await api.getPref("update_postponed_until").catch(() => null);
      if (!info.update_channel.configured) {
        setPhase({ k: "skip", reason: info.update_channel.note });
        setTimeout(onDone, 900);
        return;
      }
      if (!auto) {
        setPhase({ k: "skip", reason: "Atualização automática desativada nas configurações." });
        setTimeout(onDone, 700);
        return;
      }
      setPhase({ k: "checking" });
      const timeout = new Promise<UpdateCheck>((resolve) =>
        setTimeout(
          () =>
            resolve({ status: "error", current_version: info.version, version: null, notes: null, date: null, error: "tempo limite ao consultar o feed" }),
          CHECK_TIMEOUT_MS,
        ),
      );
      const u = await Promise.race([api.checkUpdate().catch((e) => ({ status: "error", error: errorText(e) }) as UpdateCheck), timeout]);
      if (u.status === "available" && u.version) {
        if (postponed && postponed === u.version) {
          setPhase({ k: "skip", reason: `Versão ${u.version} adiada; será oferecida de novo na próxima abertura.` });
          await api.setPref("update_postponed_until", "").catch(() => undefined);
          setTimeout(onDone, 1200);
        } else {
          setPhase({ k: "available", u });
        }
      } else if (u.status === "up_to_date") {
        setPhase({ k: "skip", reason: `Você já está na versão mais recente (${info.version}).` });
        setTimeout(onDone, 700);
      } else {
        setPhase({ k: "skip", reason: `Sem atualização agora: ${u.error ?? "feed indisponível"}. O treino funciona offline.` });
        setTimeout(onDone, 2500);
      }
    })();
  }, [info, onDone]);

  const install = async () => {
    setPhase({ k: "installing", done: 0, total: null });
    try {
      await api.installUpdate((e) => {
        if (e.event === "started") setPhase({ k: "installing", done: 0, total: e.data.contentLength });
        if (e.event === "progress") setPhase({ k: "installing", done: e.data.downloaded, total: e.data.contentLength });
      });
    } catch (e) {
      setPhase({ k: "failed", msg: errorText(e) });
    }
  };

  return (
    <div className="launcher">
      <div className="launcher-box">
        <h1>
          <span className="brand-mark" aria-hidden>♠</span> PokerLab
        </h1>
        <p className="muted">Treinador de cash game NLHE · v{info?.version ?? "…"}</p>
        {error && <div className="alert alert-error">{error}</div>}
        {phase.k === "loading" && <p>Carregando dados locais…</p>}
        {phase.k === "checking" && <p aria-live="polite">Verificando atualizações assinadas…</p>}
        {phase.k === "skip" && <p aria-live="polite">{phase.reason}</p>}
        {phase.k === "available" && (
          <div className="update-card">
            <h2>Nova versão {phase.u.version}</h2>
            {phase.u.notes && <pre className="notes">{phase.u.notes}</pre>}
            <p className="muted">O download é verificado pela assinatura do updater antes de instalar. Seu histórico fica preservado.</p>
            <div className="row">
              <button className="primary" onClick={install} autoFocus>
                Atualizar agora
              </button>
              <button
                onClick={async () => {
                  await api.setPref("update_postponed_until", phase.u.version ?? "").catch(() => undefined);
                  onDone();
                }}
              >
                Adiar
              </button>
            </div>
          </div>
        )}
        {phase.k === "installing" && (
          <div aria-live="polite">
            <p>Baixando e validando atualização…</p>
            <progress max={phase.total ?? undefined} value={phase.done} />
            <p className="muted">
              {(phase.done / 1048576).toFixed(1)} MiB{phase.total ? ` de ${(phase.total / 1048576).toFixed(1)} MiB` : ""}. O aplicativo
              reinicia ao final.
            </p>
          </div>
        )}
        {phase.k === "failed" && (
          <div className="alert alert-error" role="alert">
            <p>A atualização falhou: {phase.msg}</p>
            <p>A versão instalada continua funcionando. Você pode tentar de novo ou instalar manualmente pela página da release.</p>
            <div className="row">
              <button onClick={install}>Tentar de novo</button>
              <button onClick={onDone}>Continuar</button>
            </div>
          </div>
        )}
        {phase.k !== "installing" && (
          <button className="primary big" onClick={onDone} disabled={!info && !error}>
            Treinar agora
          </button>
        )}
      </div>
    </div>
  );
}
