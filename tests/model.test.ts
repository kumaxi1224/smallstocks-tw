import { describe, it, expect } from "vitest";
import {
  esc,
  stale,
  change,
  shellOpacity,
  ruleText,
  latestCloseDate,
  livePrice,
  portfolioSummary,
} from "../src/model.ts";
import type { Quote, Rule, Snapshot, Position } from "../src/model.ts";
describe("display truthfulness", () => {
  it("uses the actual newest watched close date, even when it is not today", () => {
    const state = {
      watchlist: ["2330", "0050"],
      dailySummaries: {
        "2330": { days: [{ date: "2026-11-05" }] },
        "0050": { days: [{ date: "2026-11-04" }] },
        "not-watched": { days: [{ date: "2026-11-06" }] },
      },
    } as unknown as Snapshot;
    expect(latestCloseDate(state)).toBe("2026-11-05");
    expect(
      latestCloseDate({ watchlist: ["2330"], dailySummaries: {} }),
    ).toBeNull();
  });
  it("marks missing, old and future source timestamps as stale", () => {
    expect(stale(undefined, 100000)).toBe(true);
    expect(stale({ sourceAt: 1000 } as Quote, 500000)).toBe(true);
    expect(stale({ sourceAt: 1000 } as Quote, 100000)).toBe(false);
    expect(stale({ sourceAt: 100000 } as Quote, 100000)).toBe(false);
    expect(stale({ sourceAt: 106000 } as Quote, 100000)).toBe(true);
  });
  it("shows the last trade first, then the bid/ask average marked as a book price, never a trade", () => {
    const quote = (o: Partial<Quote>) =>
      ({ price: null, changePercent: null, ...o }) as Quote;
    expect(livePrice(quote({ price: 100, changePercent: 1, mid: 99 }))).toEqual(
      {
        price: 100,
        pct: 1,
        basis: "trade",
      },
    );
    expect(livePrice(quote({ mid: 99, midChangePercent: -1 }))).toEqual({
      price: 99,
      pct: -1,
      basis: "book",
    });
    expect(livePrice(quote({}))).toEqual({
      price: null,
      pct: null,
      basis: "none",
    });
    expect(livePrice(undefined).basis).toBe("none");
  });
  it("keeps signs without color and escapes untrusted source names", () => {
    expect(change(-1)).toBe("-1.00%");
    expect(change(1)).toBe("+1.00%");
    expect(esc('<img src=x onerror="x">')).not.toContain("<img");
  });
  it("describes the actual metric and direction", () => {
    expect(
      ruleText({
        symbol: "2330",
        metric: "price",
        direction: "below",
        threshold: 1000,
      } as Rule),
    ).toContain("成交價降至 1,000 元");
  });
});
import { closeDay } from "../src/model.ts";
describe("what a closed session shows", () => {
  const at = (time: string) => Date.parse(`2026-11-09T${time}+08:00`);
  const friday = { date: "2026-11-06", close: 2135, changePercent: -4.69 };
  const snapshot = (
    now: string,
    quote: Record<string, unknown>,
    days = [friday],
  ) =>
    ({
      now: at(now),
      watchlist: ["2330"],
      dailySummaries: { "2330": { days } },
      quotes: {
        "2330": {
          price: 2235,
          changePercent: 4.68,
          date: "20261109",
          ...quote,
        },
      },
    }) as unknown as Parameters<typeof closeDay>[0];
  it("shows today's closing trade once the session has ended, not last Friday", () => {
    const view = closeDay(
      snapshot("13:36:00", {
        tradeAt: at("13:30:00"),
        sourceAt: at("13:30:00"),
      }),
      "2330",
    );
    expect(view).toMatchObject({
      date: "2026-11-09",
      close: 2235,
      kind: "final",
    });
  });
  it("waits for a postponed close before treating a mid-session trade as the close", () => {
    const early = snapshot("13:32:00", {
      tradeAt: at("13:29:50"),
      sourceAt: at("13:31:00"),
    });
    expect(closeDay(early, "2330")?.kind).toBe("previous");
    const later = snapshot("13:36:00", {
      tradeAt: at("13:29:50"),
      sourceAt: at("13:35:00"),
    });
    expect(closeDay(later, "2330")?.kind).toBe("final");
  });
  it("prefers the official row for today, and falls back to the earlier day without today's data", () => {
    const official = snapshot(
      "15:30:00",
      { tradeAt: at("13:30:00"), sourceAt: at("13:30:00") },
      [friday, { date: "2026-11-09", close: 2236, changePercent: 4.7 }],
    );
    expect(closeDay(official, "2330")).toMatchObject({
      close: 2236,
      kind: "official",
    });
    const yesterdayQuote = snapshot("08:00:00", {
      date: "20261106",
      sourceAt: at("08:00:00"),
    });
    expect(closeDay(yesterdayQuote, "2330")).toMatchObject({
      date: "2026-11-06",
      kind: "previous",
    });
    expect(closeDay(snapshot("13:36:00", { price: null }), "2330")?.kind).toBe(
      "previous",
    );
  });
  it("carries the close's point change when the source gives one", () => {
    const officialDay = {
      date: "2026-11-09",
      close: 2236,
      changePercent: 4.7,
      change: 100,
    };
    const days: {
      date: string;
      close: number;
      changePercent: number;
      change?: number | null;
    }[] = [friday, officialDay];
    const official = snapshot(
      "15:30:00",
      { tradeAt: at("13:30:00"), sourceAt: at("13:30:00") },
      days,
    );
    expect(closeDay(official, "2330")).toMatchObject({
      kind: "official",
      changePoints: 100,
    });
    const fin = snapshot("13:36:00", {
      tradeAt: at("13:30:00"),
      sourceAt: at("13:30:00"),
      previous: 2135,
    });
    expect(closeDay(fin, "2330")).toMatchObject({
      kind: "final",
      changePoints: 100,
    });
  });
});
import { aging, ageSeconds, oldestDataAt, points } from "../src/model.ts";
describe("how old the data is", () => {
  const quote = (ageMs: number, now = 1_000_000) =>
    ({ sourceAt: now - ageMs }) as Quote;
  it("goes grey after a minute and expires after three", () => {
    expect(aging(quote(30_000), 1_000_000)).toBe(false);
    expect(aging(quote(70_000), 1_000_000)).toBe(true);
    expect(stale(quote(70_000), 1_000_000)).toBe(false);
    expect(aging(quote(200_000), 1_000_000)).toBe(false);
    expect(stale(quote(200_000), 1_000_000)).toBe(true);
    expect(ageSeconds(quote(70_000), 1_000_000)).toBe(70);
    expect(ageSeconds(undefined, 1_000_000)).toBeNull();
  });
  it("reports the oldest snapshot time among the watched symbols", () => {
    const s = {
      watchlist: ["a", "b", "c"],
      quotes: { a: { sourceAt: 500 }, b: { sourceAt: 300 } },
    } as unknown as Parameters<typeof oldestDataAt>[0];
    expect(oldestDataAt(s)).toBe(300);
    expect(oldestDataAt({ watchlist: ["x"], quotes: {} })).toBeNull();
  });
  it("formats a signed point change, trimmed like the price", () => {
    expect(points(12.5)).toBe("+12.5");
    expect(points(-3.25)).toBe("-3.25");
    expect(points(0)).toBe("0");
    expect(points(null)).toBe("—");
    expect(points(undefined)).toBe("—");
  });
});
import { displaySymbol, isYahooSymbol, terminalRows } from "../src/model.ts";
describe("Yahoo symbols", () => {
  it("routes by the core's rule: a ^ prefix, an '=' suffix, or no digit at all", () => {
    expect(isYahooSymbol("^IXIC")).toBe(true);
    expect(isYahooSymbol("^TWII")).toBe(true);
    expect(isYahooSymbol("VOO")).toBe(true);
    expect(isYahooSymbol("BRK.B")).toBe(true);
    expect(isYahooSymbol("BZ=F")).toBe(true);
    expect(isYahooSymbol("TWD=X")).toBe(true);
    expect(isYahooSymbol("6E=F")).toBe(true);
    expect(isYahooSymbol("2330")).toBe(false);
    expect(isYahooSymbol("00685L")).toBe(false);
    expect(isYahooSymbol("t00")).toBe(false);
  });
  it("displays indices without the ^ and leaves other codes alone", () => {
    expect(displaySymbol("^IXIC")).toBe("IXIC");
    expect(displaySymbol("^GSPC")).toBe("GSPC");
    expect(displaySymbol("VOO")).toBe("VOO");
    expect(displaySymbol("BZ=F")).toBe("BZ=F");
    expect(displaySymbol("TWD=X")).toBe("TWD=X");
    expect(displaySymbol("2330")).toBe("2330");
  });
  it("renders a Yahoo row from its live quote while Taiwan is closed", () => {
    const now = Date.parse("2026-11-12T23:00:00+08:00");
    const s = {
      now,
      marketOpen: false,
      demo: false,
      watchlist: ["^IXIC"],
      dailySummaries: {},
      settings: { hideNames: false },
      quotes: {
        "^IXIC": {
          symbol: "^IXIC",
          name: "Nasdaq Composite",
          price: 25466.19,
          previous: 25065.88,
          changePercent: 1.6,
          sourceAt: now - 30_000,
          receivedAt: now,
          date: "20261112",
          source: "Yahoo",
        },
      },
    } as unknown as Snapshot;
    const row = terminalRows(s);
    expect(row).toContain("IXIC");
    expect(row).not.toContain("^IXIC");
    expect(row).toContain("25,466.19");
    expect(row).toContain("OK");
    expect(row).not.toContain("CLOSE");
  });
});

describe("window opacity", () => {
  it("defaults to fully opaque and clamps into 0.4–1", () => {
    expect(shellOpacity(undefined)).toBe(1);
    expect(shellOpacity(null)).toBe(1);
    expect(shellOpacity(Number.NaN)).toBe(1);
    expect(shellOpacity(1)).toBe(1);
    expect(shellOpacity(0.65)).toBe(0.65);
    expect(shellOpacity(0.4)).toBe(0.4);
    expect(shellOpacity(0.1)).toBe(0.4);
    expect(shellOpacity(0)).toBe(0.4);
    expect(shellOpacity(2)).toBe(1);
  });
});

describe("portfolio summary", () => {
  const tsm: Position = {
    code: "2330",
    lots: [
      { date: "2026-11-10", price: 1000, shares: 100 },
      { date: "", price: 1100, shares: 50 },
    ],
  };
  it("weights the average price across lots and values the position", () => {
    const s = portfolioSummary([tsm], {
      "2330": { price: 1200, previous: 1150 },
    });
    const r = s.positions[0]!;
    expect(r.shares).toBe(150);
    expect(r.cost).toBe(155_000);
    expect(r.avgPrice).toBeCloseTo(155_000 / 150);
    expect(r.marketValue).toBe(180_000);
    expect(r.dayPnl).toBe(7_500);
    expect(r.totalPnl).toBeCloseTo(25_000);
    expect(r.totalPnlPercent).toBeCloseTo((25_000 / 155_000) * 100);
    const twd = s.totals[0]!;
    expect(twd.currency).toBe("TWD");
    expect(twd.marketValue).toBe(180_000);
    expect(twd.dayPnl).toBe(7_500);
    expect(twd.dayPnlPercent).toBeCloseTo((7_500 / (1150 * 150)) * 100);
    expect(twd.totalPnlPercent).toBeCloseTo((25_000 / 155_000) * 100);
  });
  it("handles fractional US shares", () => {
    const s = portfolioSummary(
      [{ code: "NVDA", lots: [{ date: "", price: 168.8, shares: 9.43 }] }],
      { NVDA: { price: 180, previous: 175 } },
    );
    const r = s.positions[0]!;
    expect(r.currency).toBe("USD");
    expect(r.marketValue).toBeCloseTo(180 * 9.43);
    expect(r.dayPnl).toBeCloseTo(5 * 9.43);
    expect(r.totalPnl).toBeCloseTo((180 - 168.8) * 9.43);
  });
  it("leaves day P&L null when the previous close is unknown, and everything null without a price", () => {
    const s = portfolioSummary([tsm, { code: "0050", lots: [{ date: "", price: 100, shares: 10 }] }], {
      "2330": { price: 1200, previous: null },
    });
    expect(s.positions[0]!.dayPnl).toBeNull();
    expect(s.positions[0]!.totalPnl).toBeCloseTo(25_000);
    expect(s.positions[1]!.price).toBeNull();
    expect(s.positions[1]!.marketValue).toBeNull();
    expect(s.positions[1]!.totalPnl).toBeNull();
    const twd = s.totals[0]!;
    expect(twd.dayPnl).toBeNull();
    expect(twd.marketValue).toBe(180_000);
    expect(twd.totalPnl).toBeCloseTo(25_000);
    expect(twd.totalPnlPercent).toBeCloseTo((25_000 / 155_000) * 100);
  });
  it("totals TWD and USD separately, never summed together", () => {
    const s = portfolioSummary(
      [
        tsm,
        { code: "VOO", lots: [{ date: "", price: 600, shares: 6.3 }] },
      ],
      {
        "2330": { price: 1200, previous: 1150 },
        VOO: { price: 690, previous: 680 },
      },
    );
    expect(s.totals.map((t) => t.currency)).toEqual(["TWD", "USD"]);
    expect(s.totals[0]!.marketValue).toBe(180_000);
    expect(s.totals[1]!.marketValue).toBeCloseTo(690 * 6.3);
    expect(s.totals[1]!.dayPnl).toBeCloseTo(10 * 6.3);
  });
  it("reports no totals for an empty portfolio", () => {
    expect(portfolioSummary([], {})).toEqual({ positions: [], totals: [] });
  });
});
