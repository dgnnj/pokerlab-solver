import { describe, expect, it } from "vitest";
import { actionClass, bb, cardFace, formatLine, freq, pct } from "./format";

describe("format", () => {
  it("formats big blinds with comma", () => {
    expect(bb(1.82)).toBe("1,82 bb");
    expect(bb(0.001, 3)).toBe("0,001 bb");
  });
  it("formats percentages", () => {
    expect(pct(33.333)).toBe("33,3%");
    expect(freq(0.7416)).toBe("74,2%");
  });
  it("describes cards with text, not only color", () => {
    const c = cardFace("Th");
    expect(c.rank).toBe("10");
    expect(c.symbol).toBe("♥");
    expect(c.red).toBe(true);
    expect(c.text).toBe("10 de copas");
    expect(cardFace("As").red).toBe(false);
  });
  it("groups the line by street", () => {
    expect(formatLine(["BB passa", "BTN aposta 1,82 bb"])).toBe("Flop: BB passa, BTN aposta 1,82 bb");
    expect(formatLine(["BB passa", "BTN passa", "Turn: Th (overcard)", "BB passa"])).toBe(
      "Flop: BB passa, BTN passa · Turn Th (overcard): BB passa",
    );
    expect(formatLine(["BB passa", "BTN passa", "Turn: 2c (carta baixa)"])).toBe("Flop: BB passa, BTN passa · Turn 2c (carta baixa)");
    expect(formatLine([])).toBe("");
  });
  it("classifies actions", () => {
    expect(actionClass("fold", 0)).toBe("act-fold");
    expect(actionClass("check", 0)).toBe("act-passive");
    expect(actionClass("bet", 1)).toBe("act-agg1");
  });
});
