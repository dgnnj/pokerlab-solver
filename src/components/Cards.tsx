import { cardFace } from "../format";

export function PlayingCard({ card, size = "md" }: { card: string; size?: "sm" | "md" | "lg" }) {
  const f = cardFace(card);
  return (
    <span
      className={`card card-${size} ${f.red ? "card-red" : "card-black"} suit-${f.suit}`}
      role="img"
      aria-label={f.text}
      title={f.text}
    >
      <span className="card-rank">{f.rank}</span>
      <span className="card-suit">{f.symbol}</span>
    </span>
  );
}

export function CardRow({ cards, size }: { cards: string[]; size?: "sm" | "md" | "lg" }) {
  return (
    <span className="card-row">
      {cards.map((c) => (
        <PlayingCard key={c} card={c} size={size} />
      ))}
    </span>
  );
}
