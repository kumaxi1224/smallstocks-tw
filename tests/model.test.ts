import { describe, it, expect } from "vitest";
import {
  esc,
  stale,
  change,
  shellOpacity,
  shouldKeepScroll,
  sizeSignature,
  ruleText,
  latestCloseDate,
  livePrice,
  portfolioSummary,
  combinedTotals,
  monthlyRows,
  portfolioIntradaySeries,
  nextPositionSort,
  sortedPositions,
  formatMoney,
  formatWan,
  axisTicks,
} from "../src/model.ts";
import type {
  Quote,
  Rule,
  Snapshot,
  Position,
  CurrencyTotal,
  MonthlySnapshot,
} from "../src/model.ts";
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
  it("carries the day's high and low: the official row's, else the quote's session range", () => {
    const days: {
      date: string;
      close: number;
      changePercent: number;
      high?: number;
      low?: number;
    }[] = [
      friday,
      { date: "2026-11-09", close: 2236, changePercent: 4.7, high: 2250, low: 2200 },
    ];
    const official = snapshot(
      "15:30:00",
      { tradeAt: at("13:30:00"), sourceAt: at("13:30:00") },
      days,
    );
    expect(closeDay(official, "2330")).toMatchObject({ high: 2250, low: 2200 });
    const fin = snapshot("13:36:00", {
      tradeAt: at("13:30:00"),
      sourceAt: at("13:30:00"),
      dayHigh: 2245,
      dayLow: 2195,
    });
    expect(closeDay(fin, "2330")).toMatchObject({
      kind: "final",
      high: 2245,
      low: 2195,
    });
    const noRange = snapshot("13:36:00", {
      tradeAt: at("13:30:00"),
      sourceAt: at("13:30:00"),
    });
    expect(closeDay(noRange, "2330")).toMatchObject({ high: null, low: null });
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

describe("scroll place on re-render", () => {
  it("keeps the scroll place when the same view re-renders", () => {
    expect(shouldKeepScroll("settings", "settings", false)).toBe(true);
    expect(shouldKeepScroll("positions", "positions", false)).toBe(true);
  });
  it("starts at the top on a view switch or before the first render", () => {
    expect(shouldKeepScroll("watch", "settings", false)).toBe(false);
    expect(shouldKeepScroll("manage", "watch", false)).toBe(false);
    expect(shouldKeepScroll(null, "watch", false)).toBe(false);
  });
  it("starts at the top when a navigation action asked for a reset", () => {
    expect(shouldKeepScroll("watch", "watch", true)).toBe(false);
    expect(shouldKeepScroll("rules", "rules", true)).toBe(false);
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
    expect(r.dayPnlPercent).toBeCloseTo((7_500 / (1150 * 150)) * 100);
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
    expect(r.dayPnlPercent).toBeCloseTo((5 / 175) * 100);
    expect(r.totalPnl).toBeCloseTo((180 - 168.8) * 9.43);
  });
  it("leaves day P&L null when the previous close is unknown, and everything null without a price", () => {
    const s = portfolioSummary([tsm, { code: "0050", lots: [{ date: "", price: 100, shares: 10 }] }], {
      "2330": { price: 1200, previous: null },
    });
    expect(s.positions[0]!.dayPnl).toBeNull();
    expect(s.positions[0]!.dayPnlPercent).toBeNull();
    expect(s.positions[0]!.totalPnl).toBeCloseTo(25_000);
    expect(s.positions[1]!.price).toBeNull();
    expect(s.positions[1]!.dayPnlPercent).toBeNull();
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
describe("position sorting", () => {
  // 9999 has no quote at all: market value and both P&Ls are null. 2317 has
  // a price but no previous close, so only its day P&L is null.
  const positions: Position[] = [
    { code: "2330", lots: [{ date: "", price: 1000, shares: 100 }] },
    { code: "0050", lots: [{ date: "", price: 100, shares: 300 }] },
    { code: "2317", lots: [{ date: "", price: 200, shares: 50 }] },
    { code: "9999", lots: [{ date: "", price: 50, shares: 10 }] },
  ];
  const summary = portfolioSummary(positions, {
    "2330": { price: 1200, previous: 1150 },
    "0050": { price: 90, previous: 100 },
    "2317": { price: 210, previous: null },
  });
  const codes = (list: Position[]) => list.map((p) => p.code);
  const sorted = (key: "code" | "shares" | "marketValue" | "dayPnl" | "totalPnl", dir: "asc" | "desc") =>
    codes(sortedPositions(positions, summary.positions, { key, dir }));
  it("keeps the recorded order when no sort is chosen", () => {
    expect(codes(sortedPositions(positions, summary.positions, null))).toEqual([
      "2330",
      "0050",
      "2317",
      "9999",
    ]);
  });
  it("sorts by code", () => {
    expect(sorted("code", "asc")).toEqual(["0050", "2317", "2330", "9999"]);
    expect(sorted("code", "desc")).toEqual(["9999", "2330", "2317", "0050"]);
  });
  it("sorts by shares", () => {
    expect(sorted("shares", "desc")).toEqual(["0050", "2330", "2317", "9999"]);
    expect(sorted("shares", "asc")).toEqual(["9999", "2317", "2330", "0050"]);
  });
  it("sorts by market value, unpriced rows last in both directions", () => {
    expect(sorted("marketValue", "desc")).toEqual(["2330", "0050", "2317", "9999"]);
    expect(sorted("marketValue", "asc")).toEqual(["2317", "0050", "2330", "9999"]);
  });
  it("sorts by day P&L, unknown values last in both directions", () => {
    expect(sorted("dayPnl", "desc")).toEqual(["2330", "0050", "2317", "9999"]);
    expect(sorted("dayPnl", "asc")).toEqual(["0050", "2330", "2317", "9999"]);
  });
  it("sorts by total P&L, unknown values last in both directions", () => {
    expect(sorted("totalPnl", "desc")).toEqual(["2330", "2317", "0050", "9999"]);
    expect(sorted("totalPnl", "asc")).toEqual(["0050", "2317", "2330", "9999"]);
  });
  it("does not mutate the recorded order", () => {
    sorted("shares", "desc");
    expect(codes(positions)).toEqual(["2330", "0050", "2317", "9999"]);
  });
  it("cycles header clicks: numbers start descending, code ascending, same column flips", () => {
    expect(nextPositionSort(null, "shares")).toEqual({ key: "shares", dir: "desc" });
    expect(nextPositionSort(null, "code")).toEqual({ key: "code", dir: "asc" });
    expect(nextPositionSort({ key: "shares", dir: "desc" }, "shares")).toEqual({
      key: "shares",
      dir: "asc",
    });
    expect(nextPositionSort({ key: "code", dir: "asc" }, "code")).toEqual({
      key: "code",
      dir: "desc",
    });
    expect(nextPositionSort({ key: "shares", dir: "asc" }, "dayPnl")).toEqual({
      key: "dayPnl",
      dir: "desc",
    });
  });
});
describe("portfolio intraday series", () => {
  const t0 = 1_700_000_000_000 - (1_700_000_000_000 % 60000);
  const min = (n: number) => t0 + n * 60000;
  it("forward-fills each leg across its own gaps and weights by shares", () => {
    const s = portfolioIntradaySeries(
      [
        { code: "2330", lots: [{ date: "", price: 1000, shares: 100 }] },
        { code: "0050", lots: [{ date: "", price: 50, shares: 50 }] },
      ],
      {
        // 2330 samples at minute 0 and 2 only; minute 1 must reuse 10.
        "2330": [
          { at: min(0), price: 10 },
          { at: min(2), price: 12 },
        ],
        // 0050 first trades at minute 1; before that it counts at its
        // previous close of 20.
        "0050": [{ at: min(1), price: 22 }],
      },
      { "2330": 10, "0050": 20 },
    );
    expect(s.points).toEqual([
      { at: min(0), value: 100 * 10 + 50 * 20 },
      { at: min(1), value: 100 * 10 + 50 * 22 },
      { at: min(2), value: 100 * 12 + 50 * 22 },
    ]);
    expect(s.previousTotal).toBe(2000);
    expect(s.high).toBe(2300);
    expect(s.low).toBe(2000);
    expect(s.lastValue).toBe(2300);
    expect(s.firstAt).toBe(min(0));
    expect(s.lastAt).toBe(min(2));
  });
  it("counts a leg with no samples at all at its previous close throughout", () => {
    const s = portfolioIntradaySeries(
      [
        { code: "2330", lots: [{ date: "", price: 1000, shares: 10 }] },
        { code: "0050", lots: [{ date: "", price: 50, shares: 5 }] },
      ],
      { "2330": [{ at: min(0), price: 110 }], "0050": [] },
      { "2330": 100, "0050": 60 },
    );
    expect(s.points).toEqual([{ at: min(0), value: 10 * 110 + 5 * 60 }]);
    expect(s.previousTotal).toBe(10 * 100 + 5 * 60);
  });
  it("has no points before any price is known, and excludes USD positions", () => {
    const s = portfolioIntradaySeries(
      [
        { code: "2330", lots: [{ date: "", price: 1000, shares: 2.5 }] },
        { code: "VOO", lots: [{ date: "", price: 600, shares: 6.3 }] },
      ],
      {
        "2330": [
          { at: min(0), price: null },
          { at: min(1), price: 40 },
        ],
        VOO: [{ at: min(0), price: 690 }],
      },
      { "2330": null, VOO: 680 },
    );
    // Minute 0: the only TWD leg has neither a sample price nor a previous
    // close, so no aggregate is invented for it; VOO never counts at all.
    expect(s.points).toEqual([{ at: min(1), value: 100 }]);
    expect(s.previousTotal).toBeNull();
  });
  it("returns an empty series for an empty or USD-only portfolio", () => {
    expect(portfolioIntradaySeries([], {}, {})).toEqual({
      points: [],
      previousTotal: null,
      firstAt: null,
      lastAt: null,
      lastValue: null,
      high: null,
      low: null,
    });
    const usd = portfolioIntradaySeries(
      [{ code: "VOO", lots: [{ date: "", price: 600, shares: 6.3 }] }],
      { VOO: [{ at: min(0), price: 690 }] },
      { VOO: 680 },
    );
    expect(usd.points).toEqual([]);
    expect(usd.previousTotal).toBeNull();
  });
});
describe("window size signature", () => {
  const base = {
    tab: "positions",
    appearance: "small",
    rowDisplay: "numbers",
    watchCount: 12,
    demo: false,
    eventCount: 0,
  };
  it("is stable while nothing that decides the size changes", () => {
    // A poll re-render with the same factors must produce the same
    // signature, so resize() leaves a hand-dragged window size alone.
    expect(sizeSignature(base)).toBe(sizeSignature({ ...base }));
    // Event counts above the three-line cap bucket identically.
    expect(sizeSignature({ ...base, eventCount: 3 })).toBe(
      sizeSignature({ ...base, eventCount: 500 }),
    );
  });
  it("changes on every deliberate switch", () => {
    const sig = sizeSignature(base);
    expect(sizeSignature({ ...base, tab: "watch" })).not.toBe(sig);
    expect(sizeSignature({ ...base, appearance: "gray" })).not.toBe(sig);
    expect(sizeSignature({ ...base, rowDisplay: "minimal" })).not.toBe(sig);
    expect(sizeSignature({ ...base, watchCount: 13 })).not.toBe(sig);
    expect(sizeSignature({ ...base, demo: true })).not.toBe(sig);
    expect(sizeSignature({ ...base, eventCount: 1 })).not.toBe(sig);
    expect(sizeSignature({ ...base, eventCount: 2 })).not.toBe(
      sizeSignature({ ...base, eventCount: 1 }),
    );
  });
});
describe("money formatting", () => {
  it("rounds TWD amounts to whole dollars and keeps USD at two decimals", () => {
    expect(formatMoney(6495466.53, "TWD")).toBe("6,495,467");
    expect(formatMoney(-148818.99, "TWD", true)).toBe("-148,819");
    expect(formatMoney(1234.5, "TWD", true)).toBe("+1,235");
    expect(formatMoney(15436.12, "USD")).toBe("15,436.12");
    expect(formatMoney(371850, "USD")).toBe("371,850.00");
    expect(formatMoney(63, "USD", true)).toBe("+63.00");
    expect(formatMoney(null, "TWD")).toBe("—");
    expect(formatMoney(0, "TWD", true)).toBe("0");
  });
  it("shortens axis amounts in 萬 with at most one decimal", () => {
    expect(formatWan(6402819)).toBe("640萬");
    expect(formatWan(6295000)).toBe("630萬");
    expect(formatWan(12300)).toBe("1.2萬");
    expect(formatWan(10000)).toBe("1萬");
    expect(formatWan(9500)).toBe("9500");
    expect(formatWan(0)).toBe("0");
  });
  it("picks nice axis ticks inside the span", () => {
    expect(axisTicks(6200000, 6500000, 4)).toEqual([
      6200000, 6300000, 6400000, 6500000,
    ]);
    const t = axisTicks(95, 108, 4);
    expect(t.length).toBeGreaterThanOrEqual(2);
    expect(t.every((v) => v >= 95 && v <= 108)).toBe(true);
    expect(axisTicks(100, 100)).toEqual([100]);
    expect(axisTicks(0, 3, 4)).toEqual([0, 1, 2, 3]);
  });
});
describe("combined TWD totals", () => {
  const total = (
    currency: CurrencyTotal["currency"],
    marketValue: number | null,
    dayPnl: number | null,
  ): CurrencyTotal => ({
    currency,
    marketValue,
    dayPnl,
    dayPnlPercent: null,
    totalPnl: null,
    totalPnlPercent: null,
  });
  it("passes a TWD-only portfolio through without a rate", () => {
    const c = combinedTotals([total("TWD", 6_495_467, -148_819)], null);
    expect(c).not.toBeNull();
    expect(c!.total).toBe(6_495_467);
    expect(c!.dayPnl).toBe(-148_819);
    expect(c!.rate).toBeNull();
    // dayPct is measured against yesterday's combined value.
    expect(c!.dayPnlPercent).toBeCloseTo((-148_819 / 6_644_286) * 100, 10);
  });
  it("converts USD at the rate and reports the rate it used", () => {
    const c = combinedTotals(
      [total("TWD", 100_000, 1_000), total("USD", 15_436.12, 360.41)],
      32.97,
    );
    expect(c).not.toBeNull();
    expect(c!.total).toBeCloseTo(100_000 + 15_436.12 * 32.97, 6);
    expect(c!.dayPnl).toBeCloseTo(1_000 + 360.41 * 32.97, 6);
    expect(c!.rate).toBe(32.97);
    const previous = c!.total - c!.dayPnl;
    expect(c!.dayPnlPercent).toBeCloseTo((c!.dayPnl / previous) * 100, 10);
  });
  it("is null when USD holdings have no usable rate", () => {
    const totals = [total("TWD", 100_000, 1_000), total("USD", 500, 5)];
    expect(combinedTotals(totals, null)).toBeNull();
    expect(combinedTotals(totals, undefined)).toBeNull();
    expect(combinedTotals(totals, 0)).toBeNull();
    expect(combinedTotals(totals, -3)).toBeNull();
    expect(combinedTotals(totals, Number.NaN)).toBeNull();
  });
  it("is null with no holdings or an unknown currency total", () => {
    expect(combinedTotals([], 32.97)).toBeNull();
    expect(combinedTotals([total("TWD", null, null)], null)).toBeNull();
    expect(combinedTotals([total("TWD", 100_000, null)], null)).toBeNull();
    expect(
      combinedTotals(
        [total("TWD", 100_000, 1_000), total("USD", null, 5)],
        32.97,
      ),
    ).toBeNull();
  });
});
describe("monthly snapshot rows", () => {
  const snap = (month: string, totalTwd: number): MonthlySnapshot => ({
    month,
    totalTwd,
    twdValue: totalTwd,
    usdValue: null,
    rate: null,
    recordedOn: `${month}-01`,
  });
  it("sorts newest first without mutating the input order", () => {
    const input = [snap("2026-09", 100), snap("2026-11", 130), snap("2026-10", 120)];
    const rows = monthlyRows(input);
    expect(rows.map((r) => r.month)).toEqual(["2026-11", "2026-10", "2026-09"]);
    expect(input.map((s) => s.month)).toEqual(["2026-09", "2026-11", "2026-10"]);
  });
  it("compares each month against the previous recorded month, gaps included", () => {
    // No October snapshot: November still compares against September,
    // the previous month that has a record.
    const rows = monthlyRows([snap("2026-09", 100), snap("2026-11", 130)]);
    expect(rows[0]!.month).toBe("2026-11");
    expect(rows[0]!.change).toBe(30);
    expect(rows[0]!.changePct).toBeCloseTo(30, 10);
    const rows2 = monthlyRows([
      snap("2026-09", 200),
      snap("2026-10", 190),
      snap("2026-11", 209),
    ]);
    expect(rows2[0]!.change).toBe(19);
    expect(rows2[0]!.changePct).toBeCloseTo(10, 10);
    expect(rows2[1]!.change).toBe(-10);
    expect(rows2[1]!.changePct).toBeCloseTo(-5, 10);
  });
  it("gives the oldest row no change and tolerates empty input", () => {
    const rows = monthlyRows([snap("2026-11", 130), snap("2026-10", 120)]);
    expect(rows[1]!.change).toBeNull();
    expect(rows[1]!.changePct).toBeNull();
    expect(monthlyRows([])).toEqual([]);
    expect(monthlyRows(undefined)).toEqual([]);
    const single = monthlyRows([snap("2026-11", 130)]);
    expect(single[0]!.change).toBeNull();
  });
});
