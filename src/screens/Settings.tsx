import { useEffect, useState } from "react";
import { api, errorText, UpdateCheck } from "../api";
import type { Nav } from "../App";

export function Settings({ nav }: { nav: Nav }) {
  const info = nav.info;
  const [auto, setAuto] = useState(true);
  const [msg, setMsg] = useState<string | null>(null);
  const [check, setCheck] = useState<UpdateCheck | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void api.getPref("auto_update").then((v) => setAuto(v !== "false"));
  }, []);

  const toggleAuto = async () => {
    await api.setPref("auto_update", auto ? "false" : "true");
    setAuto(!auto);
  };

  const run = async (f: () => Promise<string>) => {
    setBusy(true);
    try {
      setMsg(await f());
    } catch (e) {
      setMsg(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section>
      <h1>Configurações</h1>
      {msg && (
        <div className="alert" role="status">
          {msg}
        </div>
      )}
      <h2>Atualizações</h2>
      <p>
        Canal: {info?.update_channel.note}
        {info?.update_channel.endpoint && <span className="muted small"> ({info.update_channel.endpoint})</span>}
      </p>
      <label className="radio">
        <input type="checkbox" checked={auto} onChange={toggleAuto} /> Verificar e oferecer atualizações automaticamente ao abrir
      </label>
      <div className="row">
        <button
          disabled={busy}
          onClick={async () => {
            setBusy(true);
            try {
              setCheck(await api.checkUpdate());
            } catch (e) {
              setMsg(errorText(e));
            } finally {
              setBusy(false);
            }
          }}
        >
          Verificar agora
        </button>
      </div>
      {check && (
        <p>
          {check.status === "available" && `Versão ${check.version} disponível — será instalada pelo launcher na próxima abertura (ou reinicie agora).`}
          {check.status === "up_to_date" && `Você está na versão mais recente (${check.current_version}).`}
          {check.status === "unconfigured" && check.error}
          {check.status === "error" && `Falha ao consultar: ${check.error}`}
        </p>
      )}
      <h2>Dados locais</h2>
      <p className="muted small">
        Banco: {info?.db_path} (schema v{info?.db_schema})
        <br />
        Pasta de dados: {info?.data_dir}
      </p>
      <div className="row">
        <button disabled={busy} onClick={() => run(api.exportCsv)}>
          Exportar histórico (CSV)
        </button>
        <button disabled={busy} onClick={() => run(api.exportHandsCsv)}>
          Exportar mãos completas (CSV)
        </button>
        <button disabled={busy} onClick={() => run(async () => `Backup criado em ${await api.backupNow()}`)}>
          Criar backup do banco
        </button>
      </div>
      <h2>Sobre</h2>
      <p>PokerLab v{info?.version}</p>
      {info?.packs.map((p) => (
        <div key={p.pack_id} className="pack-about">
          <p>
            Pacote {p.title} v{p.version} ({p.boards} flops, {p.exercises} exercícios)
          </p>
          <p className="muted small">{p.provenance}</p>
          <p className="muted small">{p.license}</p>
        </div>
      ))}
      <p className="muted small">
        Motor: postflop-solver @ {info?.engine_rev} (AGPL-3.0-or-later). Este programa é software livre sob AGPL-3.0-or-later; o
        código-fonte correspondente acompanha cada release. Uso para prática simulada e revisão pós-sessão.
      </p>
    </section>
  );
}
