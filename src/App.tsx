import { useCallback, useEffect, useState } from "react";
import { api, AppInfo, errorText, Mode, SessionSummary } from "./api";
import { Launcher } from "./screens/Launcher";
import { Home } from "./screens/Home";
import { TrainSetup } from "./screens/TrainSetup";
import { Train } from "./screens/Train";
import { Summary } from "./screens/Summary";
import { Study } from "./screens/Study";
import { Progress } from "./screens/Progress";
import { Settings } from "./screens/Settings";
import { Preset } from "./screens/Preset";

export type Screen =
  | { name: "launcher" }
  | { name: "home" }
  | { name: "setup"; mode: Mode }
  | { name: "train"; sessionId: number; returnTo?: Screen }
  | { name: "summary"; summary: SessionSummary; returnTo?: Screen }
  | { name: "study"; exerciseId?: string; highlight?: string | null; back?: Screen }
  | { name: "progress" }
  | { name: "settings" }
  | { name: "preset"; step: number; sessionId?: number };

export interface Nav {
  go: (s: Screen) => void;
  info: AppInfo | null;
  refresh: () => Promise<void>;
}

const MENU: { label: string; screen: Screen; key: string }[] = [
  { label: "Início", screen: { name: "home" }, key: "home" },
  { label: "Treinar", screen: { name: "setup", mode: "representative" }, key: "setup" },
  { label: "Estudar", screen: { name: "study" }, key: "study" },
  { label: "Revisar erros", screen: { name: "setup", mode: "review" }, key: "review" },
  { label: "Progresso", screen: { name: "progress" }, key: "progress" },
  { label: "Configurações", screen: { name: "settings" }, key: "settings" },
];

function activeKey(s: Screen): string {
  if (s.name === "setup") return s.mode === "review" ? "review" : "setup";
  if (s.name === "train" || s.name === "summary") return "setup";
  return s.name;
}

export default function App() {
  const [screen, setScreen] = useState<Screen>({ name: "launcher" });
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setInfo(await api.appInfo());
    } catch (e) {
      setErr(errorText(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const nav: Nav = { go: setScreen, info, refresh };

  if (screen.name === "launcher") {
    return <Launcher info={info} error={err} onDone={() => { void refresh(); setScreen({ name: "home" }); }} />;
  }

  return (
    <div className="shell">
      <nav className="sidebar" aria-label="Navegação principal">
        <div className="brand">
          <span className="brand-mark" aria-hidden>♠</span> PokerLab
        </div>
        {MENU.map((m) => (
          <button
            key={m.key}
            className={`nav-item ${activeKey(screen) === m.key ? "active" : ""}`}
            aria-current={activeKey(screen) === m.key ? "page" : undefined}
            onClick={() => {
              void refresh();
              setScreen(m.screen);
            }}
          >
            {m.label}
            {m.key === "review" && info && info.review_due > 0 && <span className="badge">{info.review_due}</span>}
          </button>
        ))}
        <div className="sidebar-foot muted">
          v{info?.version ?? "?"} · pacote {info?.pack_version ?? "?"}
        </div>
      </nav>
      <main className="content">
        {info?.startup_error && (
          <div className="alert alert-error" role="alert">
            Falha ao iniciar o treinador: {info.startup_error}. Pasta de dados: {info.data_dir}
          </div>
        )}
        {screen.name === "home" && <Home nav={nav} />}
        {screen.name === "setup" && <TrainSetup nav={nav} mode={screen.mode} />}
        {screen.name === "train" && <Train nav={nav} sessionId={screen.sessionId} returnTo={screen.returnTo} />}
        {screen.name === "summary" && <Summary nav={nav} summary={screen.summary} returnTo={screen.returnTo} />}
        {screen.name === "study" && (
          <Study nav={nav} exerciseId={screen.exerciseId} highlight={screen.highlight ?? null} back={screen.back} />
        )}
        {screen.name === "progress" && <Progress />}
        {screen.name === "settings" && <Settings nav={nav} />}
        {screen.name === "preset" && <Preset nav={nav} step={screen.step} />}
      </main>
    </div>
  );
}
