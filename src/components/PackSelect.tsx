import { useEffect, useState } from "react";
import { api, PackInfo } from "../api";

/** Last pack chosen on any screen ("" = every pack), kept in the preferences. */
export function usePreferredPack(packs: PackInfo[] | undefined, allowAll: boolean) {
  const [pack, setPack] = useState<string | null>(null);
  useEffect(() => {
    void api
      .getPref("preferred_pack")
      .then((v) => setPack(v ?? ""))
      .catch(() => setPack(""));
  }, []);
  const known = (id: string) => (id === "" ? allowAll : !!packs?.some((p) => p.pack_id === id));
  const value = pack !== null && known(pack) ? pack : allowAll ? "" : (packs?.[0]?.pack_id ?? "");
  const choose = (id: string) => {
    setPack(id);
    void api.setPref("preferred_pack", id).catch(() => undefined);
  };
  return [value, choose, pack !== null] as const;
}

export function PackSelect({
  packs,
  value,
  onChange,
  allLabel,
}: {
  packs: PackInfo[];
  value: string;
  onChange: (id: string) => void;
  /** Label of the "every pack" option; omitted when a pack is required. */
  allLabel?: string;
}) {
  return (
    <label>
      Spot
      <select value={value} onChange={(e) => onChange(e.target.value)} aria-label="Spot">
        {allLabel && <option value="">{allLabel}</option>}
        {packs.map((p) => (
          <option key={p.pack_id} value={p.pack_id}>
            {p.title}
          </option>
        ))}
      </select>
    </label>
  );
}
