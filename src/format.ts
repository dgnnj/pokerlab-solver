// Display helpers (pt-BR). Pure functions, unit-tested in format.test.ts.

export function bb(v: number, digits = 2): string {
  return `${v.toLocaleString("pt-BR", { minimumFractionDigits: digits, maximumFractionDigits: digits })} bb`;
}

export function pct(v: number, digits = 1): string {
  return `${v.toLocaleString("pt-BR", { minimumFractionDigits: digits, maximumFractionDigits: digits })}%`;
}

export function freq(v: number): string {
  return pct(v * 100, 1);
}

const SUIT_SYMBOL: Record<string, string> = { s: "♠", h: "♥", d: "♦", c: "♣" };
const SUIT_NAME: Record<string, string> = { s: "espadas", h: "copas", d: "ouros", c: "paus" };

export interface CardFace {
  rank: string;
  suit: string;
  symbol: string;
  suitName: string;
  red: boolean;
  text: string;
}

export function cardFace(card: string): CardFace {
  const rank = card[0].toUpperCase() === "T" ? "10" : card[0].toUpperCase();
  const suit = card[1].toLowerCase();
  return {
    rank,
    suit,
    symbol: SUIT_SYMBOL[suit] ?? "?",
    suitName: SUIT_NAME[suit] ?? suit,
    red: suit === "h" || suit === "d",
    text: `${rank} de ${SUIT_NAME[suit] ?? suit}`,
  };
}

export const GRADE_LABEL: Record<string, string> = {
  best: "Melhor ação",
  close: "Próxima",
  inaccuracy: "Imprecisão",
  mistake: "Erro",
};

export const MODE_LABEL: Record<string, string> = {
  representative: "Amostra representativa",
  hard: "Foco em decisões difíceis",
  review: "Revisão de erros",
};

/** Action color class by kind and order (aggressive actions get warmer colors). */
export function actionClass(kind: string, index: number): string {
  if (kind === "fold") return "act-fold";
  if (kind === "check" || kind === "call") return "act-passive";
  return index % 2 === 0 ? "act-agg2" : "act-agg1";
}

export function shortDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString("pt-BR", { dateStyle: "short", timeStyle: "short" });
}
