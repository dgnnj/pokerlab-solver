// Typed wrappers over the Tauri commands defined in src-tauri/src/lib.rs.
import { invoke, Channel } from "@tauri-apps/api/core";

export type Mode = "representative" | "hard" | "review";

export interface SessionInfo {
  id: number;
  mode: Mode;
  theme: string | null;
  target_count: number;
  done: number;
  started_at: string;
  ended_at: string | null;
}

export interface HealthInfo {
  version: string;
  previous_version: string | null;
  first_run: boolean;
  updated: boolean;
  checks: string[];
}

export interface ChannelInfo {
  configured: boolean;
  endpoint: string;
  note: string;
}

export interface AppInfo {
  version: string;
  data_dir: string;
  db_path: string;
  db_schema: number;
  pack_id: string;
  pack_version: string;
  pack_title: string;
  pack_exercises: number;
  pack_license: string;
  pack_provenance: string;
  engine_rev: string;
  review_due: number;
  open_session: SessionInfo | null;
  health: HealthInfo;
  startup_error: string | null;
  update_channel: ChannelInfo;
}

export interface ThemeInfo {
  theme: string;
  label: string;
  exercises: number;
  level: number;
}

export interface ActionOption {
  index: number;
  label: string;
  kind: string;
}

export interface Presented {
  token: string;
  session: SessionInfo;
  exercise_id: string;
  theme_label: string;
  mode: Mode;
  hero_position: string;
  villain_position: string;
  hero_cards: [string, string];
  board: string[];
  street: string;
  pot_bb: number;
  to_call_bb: number;
  stack_bb: number;
  preflop_line: string;
  line: string[];
  actions: ActionOption[];
  pack_version: string;
  review_note: string | null;
}

export type Grade = "best" | "close" | "inaccuracy" | "mistake";

export interface ActionResult {
  index: number;
  label: string;
  frequency: number;
  ev_chips: number;
  ev_bb: number;
  loss_bb: number;
}

export interface Feedback {
  combo: string;
  chosen_index: number;
  best_index: number;
  best_ev_chips: number;
  chosen_ev_chips: number;
  loss_chips: number;
  loss_bb: number;
  loss_pct_pot: number;
  tolerance_chips: number;
  grade: Grade;
  chosen_frequency: number;
  actions: ActionResult[];
}

export interface FeedbackView {
  attempt_id: number;
  feedback: Feedback;
  grade_label: string;
  explanation: string[];
  equity: number;
  pot_bb: number;
  tolerance_bb: number;
  session: SessionInfo;
  in_review: boolean;
}

export interface ThemeStat {
  key: string;
  label: string;
  n: number;
  avg_loss_bb: number;
  total_loss_bb: number;
  best_or_close_pct: number;
}

export interface SessionSummary {
  session: SessionInfo;
  decisions: number;
  avg_loss_bb: number;
  total_loss_bb: number;
  grades: Record<string, number>;
  worst_themes: ThemeStat[];
}

export interface AttemptRow {
  id: number;
  session_id: number;
  created_at: string;
  exercise_id: string;
  combo: string;
  chosen_label: string;
  best_label: string;
  loss_bb: number;
  loss_pct_pot: number;
  grade: Grade;
  sampling_mode: Mode;
  theme: string;
  theme_label: string;
  street: string;
  position: string;
  board: string;
  pack_version: string;
  decision_ms: number;
  in_review: boolean;
}

export interface ProgressFilter {
  position?: string;
  street?: string;
  pot_type?: string;
  texture?: string;
  theme?: string;
  mode?: string;
  since?: string;
  pack_version?: string;
  group_by?: string;
}

export interface ProgressReport {
  total: ThemeStat;
  groups: ThemeStat[];
  recent: AttemptRow[];
}

export interface GridCell {
  hand: string;
  weight: number;
  strategy: number[];
  ev_bb: number[];
}

export interface ComboRow {
  combo: string;
  hand: string;
  weight: number;
  equity: number;
  strategy: number[];
  ev_bb: number[];
}

export interface StudyView {
  exercise_id: string;
  theme_label: string;
  position: string;
  board: string[];
  pot_bb: number;
  to_call_bb: number;
  stack_bb: number;
  preflop_line: string;
  line: string[];
  actions: ActionOption[];
  aggregate: number[];
  grid: GridCell[];
  combos: ComboRow[];
  highlight: string | null;
  solve_info: string;
  ranges_note: string;
}

export interface StudyEntry {
  exercise_id: string;
  theme_label: string;
  board: string;
  street: string;
  position: string;
  line: string[];
}

export interface BoardOption {
  solution_id: string;
  board: string;
  textures: string[];
  pack_exploitability_pct: number;
  pack_solve_ms: number;
  memory_mib: number;
}

export type EngineStatus =
  | { state: "idle" }
  | {
      state: "solving";
      solution_id: string;
      board: string;
      iteration: number;
      max_iterations: number;
      exploitability_pct: number | null;
      target_pct: number;
      elapsed_ms: number;
    }
  | {
      state: "ready";
      solution_id: string;
      board: string;
      exploitability_pct: number;
      pack_exploitability_pct: number;
      solve_ms: number;
      memory_mib: number;
      compressed: boolean;
      same_config_as_pack: boolean;
      threads: number;
    }
  | { state: "failed"; solution_id: string; board: string; error: string };

export interface HandEvent {
  street: string;
  kind: "action" | "card";
  hero: boolean;
  label: string;
  pot_bb: number;
}

export interface HandDecision {
  seq: number;
  street: string;
  board: string[];
  line: string[];
  pot_bb: number;
  to_call_bb: number;
  actions: ActionOption[];
  chosen_index: number;
  chosen_label: string;
  feedback: Feedback | null;
  grade_label: string | null;
  explanation: string[];
  equity: number | null;
  tolerance_bb: number | null;
  not_evaluable: string | null;
}

export interface HandOutcome {
  showdown: boolean;
  folded: string | null;
  winner: "hero" | "villain" | "split";
  pot_bb: number;
  rake_bb: number;
  hero_invested_bb: number;
  hero_net_bb: number;
  hero_total_bb: number | null;
  hero_hand: string | null;
  villain_hand: string | null;
  summary: string;
}

export interface HandSummary {
  decisions: number;
  graded: number;
  total_loss_bb: number;
  avg_loss_bb: number;
  grades: Record<string, number>;
}

export interface HandView {
  token: string;
  finished: boolean;
  solution_id: string;
  hero_position: string;
  villain_position: string;
  hero_cards: [string, string];
  villain_cards: [string, string] | null;
  board: string[];
  street: string;
  pot_bb: number;
  to_call_bb: number;
  hero_stack_bb: number;
  villain_stack_bb: number;
  preflop_line: string;
  events: HandEvent[];
  actions: ActionOption[];
  decisions: HandDecision[];
  outcome: HandOutcome | null;
  cut: string | null;
  summary: HandSummary;
  started_at: string;
}

export interface HandRow {
  id: number;
  started_at: string;
  ended_at: string | null;
  hero_position: string;
  hero_combo: string;
  villain_combo: string;
  flop: string;
  final_board: string | null;
  line: string | null;
  winner: string | null;
  showdown: boolean | null;
  hero_net_bb: number | null;
  decisions: number;
  graded: number;
  total_loss_bb: number;
  cut_reason: string | null;
  pack_version: string;
}

export interface HandStats {
  hands: number;
  finished: number;
  net_bb: number;
  decisions: number;
  graded: number;
  total_loss_bb: number;
  avg_loss_per_decision_bb: number;
  avg_loss_per_hand_bb: number;
}

export interface HandHistory {
  stats: HandStats;
  by_position: [string, HandStats][];
  recent: HandRow[];
}

export interface UpdateCheck {
  status: "up_to_date" | "available" | "unconfigured" | "error";
  current_version: string;
  version: string | null;
  notes: string | null;
  date: string | null;
  error: string | null;
}

export type DownloadEvent =
  | { event: "started"; data: { contentLength: number | null } }
  | { event: "progress"; data: { downloaded: number; contentLength: number | null } }
  | { event: "finished" };

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  listThemes: () => invoke<ThemeInfo[]>("list_themes"),
  listStudyEntries: () => invoke<StudyEntry[]>("list_study_entries"),
  startSession: (mode: Mode, theme: string | null, count: number) =>
    invoke<SessionInfo>("start_session", { mode, theme, count }),
  nextExercise: (sessionId: number) => invoke<Presented | null>("next_exercise", { sessionId }),
  submitAnswer: (token: string, action: number, decisionMs: number) =>
    invoke<FeedbackView>("submit_answer", { token, action, decisionMs }),
  endSession: (sessionId: number) => invoke<SessionSummary>("end_session", { sessionId }),
  markReview: (attemptId: number, tags?: string) => invoke<void>("mark_review", { attemptId, tags }),
  addNote: (sessionId: number | null, text: string) => invoke<void>("add_note", { sessionId, text }),
  listNotes: () => invoke<[string, string][]>("list_notes"),
  studyNode: (exerciseId: string, highlight?: string | null) =>
    invoke<StudyView>("study_node", { exerciseId, highlight: highlight ?? null }),
  progress: (filter: ProgressFilter) => invoke<ProgressReport>("progress", { filter }),
  history: (limit?: number) => invoke<AttemptRow[]>("history", { limit }),
  exportCsv: () => invoke<string>("export_csv"),
  backupNow: () => invoke<string>("backup_now"),
  getPref: (key: string) => invoke<string | null>("get_pref", { key }),
  setPref: (key: string, value: string) => invoke<void>("set_pref", { key, value }),
  handBoards: () => invoke<BoardOption[]>("hand_boards"),
  handPrepare: (solutionId: string) => invoke<EngineStatus>("hand_prepare", { solutionId }),
  handStatus: () => invoke<EngineStatus>("hand_status"),
  handCancel: () => invoke<EngineStatus>("hand_cancel"),
  handStart: (hero: string) => invoke<HandView>("hand_start", { hero }),
  handAct: (token: string, action: number, decisionMs: number) =>
    invoke<HandView>("hand_act", { token, action, decisionMs }),
  handView: (token: string) => invoke<HandView>("hand_view", { token }),
  handHistory: (limit?: number) => invoke<HandHistory>("hand_history", { limit }),
  checkUpdate: () => invoke<UpdateCheck>("check_update"),
  installUpdate: (onEvent: (e: DownloadEvent) => void) => {
    const ch = new Channel<DownloadEvent>();
    ch.onmessage = onEvent;
    return invoke<void>("install_update", { onEvent: ch });
  },
};

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}
