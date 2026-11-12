export type Appearance = "small" | "gray" | "terminal" | "txt";
export interface Day {
  date: string;
  open: number;
  high: number;
  low: number;
  close: number;
  change: number | null;
  changePercent: number | null;
  volume: number | null;
  volumeUnit: string;
}
export interface DailyView {
  symbol: string;
  days: Day[];
  sources: string[];
  fetchedAt: number;
  pending: boolean;
  error: string | null;
  demo: boolean;
}
export interface Quote {
  symbol: string;
  name: string;
  price: number | null;
  previous: number | null;
  bid: number | null;
  ask: number | null;
  changePercent: number | null;
  /** Average of the best bid and ask: an order-book value, never a trade. */
  mid?: number | null;
  midChangePercent?: number | null;
  sourceAt: number;
  receivedAt: number;
  /** When the last trade happened, when the source said. */
  tradeAt?: number | null;
  /** Session high/low from the source; absent when the source gives none. */
  dayHigh?: number | null;
  dayLow?: number | null;
  date: string;
  source: string;
}
export interface Rule {
  id: string;
  symbol: string;
  metric: "price" | "changePercent" | "mid" | "midChangePercent";
  direction: "above" | "below";
  threshold: number;
  channel: "off" | "log" | "strip" | "card";
  repeat: "once" | "daily";
  enabled: boolean;
  firedDay: string | null;
}
export interface StockEvent {
  id: string;
  ruleId: string;
  symbol: string;
  text: string;
  sourceAt: number;
  detectedAt: number;
  channel: Rule["channel"];
  value: number;
  threshold: number;
  source?: string;
}
export interface WatchGroup {
  name: string;
  codes: string[];
}
/** One purchase lot of a held position; date is YYYY-MM-DD or "" when unrecorded. */
export interface Lot {
  date: string;
  price: number;
  shares: number;
}
export interface Position {
  code: string;
  lots: Lot[];
}
/** One month's recorded portfolio total (TWD equivalent), captured by the
 * core the first time the app ran that month with complete quotes. */
export interface MonthlySnapshot {
  /** "YYYY-MM". */
  month: string;
  totalTwd: number;
  twdValue: number;
  usdValue: number | null;
  rate: number | null;
  /** Taipei date the total was actually captured, "YYYY-MM-DD". */
  recordedOn: string;
}
export interface MonthlyRow extends MonthlySnapshot {
  /** Change against the previous recorded month; null for the oldest row. */
  change: number | null;
  changePct: number | null;
}
/**
 * Monthly snapshots as table rows, newest first. Each row's change compares
 * against the previous *recorded* month — a month with no snapshot simply
 * does not take part, so a gap never distorts the comparison — and the
 * oldest row has nothing to compare against (nulls).
 */
export function monthlyRows(
  snapshots: MonthlySnapshot[] | undefined,
): MonthlyRow[] {
  const sorted = [...(snapshots ?? [])].sort((a, b) =>
    a.month < b.month ? 1 : a.month > b.month ? -1 : 0,
  );
  return sorted.map((s, i) => {
    const prev = sorted[i + 1];
    if (!prev) return { ...s, change: null, changePct: null };
    const delta = s.totalTwd - prev.totalTwd;
    return {
      ...s,
      change: delta,
      changePct: prev.totalTwd !== 0 ? (delta / prev.totalTwd) * 100 : null,
    };
  });
}
export interface Snapshot {
  intraday: Record<string, IntradayView>;
  dailySummaries: Record<string, DailyView>;
  watchlist: string[];
  /** Watch groups; `watchlist` above is the active group's codes. */
  groups?: WatchGroup[];
  activeGroup?: number;
  /** Held positions with purchase lots; absent on old states. */
  positions?: Position[];
  /** Monthly portfolio snapshots; absent on old states. */
  monthlySnapshots?: MonthlySnapshot[];
  /** Display names the core learned from live quotes, keyed by code. This
   * persisted cache is the name fallback when no live quote is on hand
   * (after a restart, or in the close-day view); absent on old states. */
  names?: Record<string, string>;
  /** Tally of the import_positions call that produced this snapshot, if any. */
  positionsImport?: { added: number; failedLines: number[] };
  /** Tally of the import_data call that produced this snapshot, if any. */
  dataImport?: {
    groups: number;
    positions: number;
    lots: number;
    skippedSymbols: string[];
    skippedLots: number;
    skippedSnapshots: number;
  };
  settings: {
    alwaysOnTop?: boolean;
    appearance: Appearance;
    paused: boolean;
    refreshSeconds: number;
    rowDisplay?: "numbers" | "trend" | "minimal";
    showCodes?: boolean;
    hideNames?: boolean;
    showChangePoints?: boolean;
    quoteSource?: "mis" | "shioaji";
    sidecarPort?: number;
    /** Shell background opacity, 0.4–1.0; absent means fully opaque. */
    windowOpacity?: number;
    /** Telegram alert push. The bot token itself never leaves the core:
     * the snapshot carries only whether one is stored. */
    telegramEnabled?: boolean;
    telegramChatId?: string;
    telegramTokenSet?: boolean;
    /** Automatic state.json backups into a user-chosen folder. */
    backupEnabled?: boolean;
    backupDir?: string;
    /** Auxiliary PTT chat window (public t.me/s preview, no login). */
    chatWindow?: boolean;
  };
  quotes: Record<string, Quote>;
  rules: Rule[];
  events: StockEvent[];
  demo: boolean;
  error: string | null;
  sourceNote?: string | null;
  /** Latest state backup outcome, once one has run: when and where it
   * landed, or why it failed. Absent on old states. */
  lastBackupAt?: { at: string; path: string } | null;
  lastBackupError?: string | null;
  privacyShortcutError?: string | null;
  lastFetch: number | null;
  nextPoll: number;
  marketOpen: boolean;
  /** Holiday name when the official calendar closes a weekday session. */
  marketClosedReason?: string | null;
  now: number;
  revision: number;
}
export interface IntradayPoint {
  at: number;
  price: number | null;
}
export interface IntradayView {
  series: {
    symbol: string;
    date: string;
    source: string;
    fetchedAt: number;
    partial: boolean;
    points: IntradayPoint[];
  } | null;
  pending: boolean;
  error: string | null;
  demo: boolean;
}
export const esc = (s: string) =>
  s.replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ]!,
  );
export const number = (n: number | null | undefined) =>
  n == null
    ? "—"
    : new Intl.NumberFormat("en-US", { maximumFractionDigits: 2 }).format(n);
export const change = (n: number | null | undefined) =>
  n == null ? "—" : `${n > 0 ? "+" : ""}${n.toFixed(2)}%`;
/** A signed point change, e.g. +12.5 / -3.25, trimmed the same way as `number`. */
export const points = (n: number | null | undefined) =>
  n == null ? "—" : `${n > 0 ? "+" : ""}${number(n)}`;
/**
 * Formats a money amount by currency convention: TWD amounts are whole
 * dollars (rounded to the nearest dollar — Taiwan share settlements have no
 * cents), USD amounts keep two decimals. `signed` prefixes a "+" for gains,
 * matching `points`. Prices are not money in this sense and keep `number`.
 */
export function formatMoney(
  n: number | null | undefined,
  currency: Currency,
  signed = false,
): string {
  if (n == null || !Number.isFinite(n)) return "—";
  const digits = currency === "TWD" ? 0 : 2;
  const body = new Intl.NumberFormat("en-US", {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  }).format(n);
  return signed && n > 0 ? `+${body}` : body;
}
/** Shell background opacity, clamped to the range the core accepts (0.4–1). */
export const shellOpacity = (v: number | null | undefined) =>
  v == null || !Number.isFinite(v) ? 1 : Math.min(1, Math.max(0.4, v));
/** Whether re-rendering the view already on screen keeps its scroll place.
 * Poll ticks and background refreshes re-render the same view and must not
 * fling the user back to the top; a view switch — or an explicit reset from
 * a deliberate navigation action — starts at the top instead. */
export const shouldKeepScroll = (
  renderedTab: string | null,
  tab: string,
  resetPending: boolean,
) => !resetPending && renderedTab !== null && renderedTab === tab;
/**
 * The factors that decide the main window's target size, joined into one
 * signature. resize() compares it against the last applied signature and
 * does nothing while it is unchanged, so a size the user dragged by hand
 * survives poll re-renders; only a deliberate switch (another tab, another
 * appearance or display mode, a longer or shorter watchlist, demo or event
 * presence) re-asserts the computed size. The event count is bucketed the
 * way the height formulas consume it (presence, capped at three lines).
 */
export const sizeSignature = (f: {
  tab: string;
  appearance: string;
  rowDisplay?: string;
  watchCount: number;
  demo: boolean;
  eventCount: number;
}) =>
  [
    f.tab,
    f.appearance,
    f.rowDisplay ?? "",
    f.watchCount,
    f.demo ? 1 : 0,
    Math.min(f.eventCount, 3),
  ].join("|");
export const time = (n: number | null | undefined) =>
  n
    ? new Date(n).toLocaleTimeString("zh-TW", {
        hour12: false,
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
        timeZone: "Asia/Taipei",
      })
    : "尚未取得";
// Stock snapshots arrive about every one to two minutes; the core uses the same three minutes.
export const FRESH_MS = 180_000;
/**
 * Yahoo-routed symbols mirror the core's route rule: a ^ prefix, an =F/=X
 * suffix (futures and FX, whose bodies may contain digits), or no digit at
 * all (US tickers). Taiwan codes always contain a digit and never an '='.
 * Yahoo symbols have no TWSE close-day machinery: their rows always render
 * from the live quote, whatever the Taiwan session is doing.
 */
export const isYahooSymbol = (c: string) =>
  c.startsWith("^") || c.includes("=") || !/\d/.test(c);
/** How a stored symbol is written for the user: indices lose their ^ prefix. */
export const displaySymbol = (c: string) => c.replace(/^\^/, "");
export const stale = (q: Quote | undefined, now: number) =>
  !q || now - q.sourceAt > FRESH_MS || q.sourceAt > now + 5000;
/**
 * The number a live quote shows. The last trade when the source gave one; otherwise the average of
 * the best bid and ask, which is marked as an order price wherever it is drawn and is never called
 * a trade. "none" when neither exists.
 */
// Data is "aging" (drawn grey) after a minute and "stale" (expired) after three.
export const AGING_MS = 60_000;
export const aging = (q: Quote | undefined, now: number) =>
  !stale(q, now) && now - q!.sourceAt > AGING_MS;
/** Seconds since the source's own snapshot time, 0 when it is not older than now. */
export const ageSeconds = (q: Quote | undefined, now: number) =>
  q ? Math.max(0, Math.round((now - q.sourceAt) / 1000)) : null;
/** The oldest snapshot time among the watched symbols: the data is at least this fresh. */
export function oldestDataAt(
  s: Pick<Snapshot, "watchlist" | "quotes">,
): number | null {
  const times = s.watchlist
    .map((c) => s.quotes[c]?.sourceAt)
    .filter((t): t is number => typeof t === "number");
  return times.length ? Math.min(...times) : null;
}
export type PriceBasis = "trade" | "book" | "none";
export function livePrice(q: Quote | undefined): {
  price: number | null;
  pct: number | null;
  basis: PriceBasis;
} {
  if (q?.price != null)
    return { price: q.price, pct: q.changePercent ?? null, basis: "trade" };
  if (q?.mid != null)
    return { price: q.mid, pct: q.midChangePercent ?? null, basis: "book" };
  return { price: null, pct: null, basis: "none" };
}
export const BOOK_NOTE = "委託價（買賣中間價），非成交價";
/**
 * The close a closed-session view shows for one symbol.
 *  - "official": the official daily row for today (published after 15:00).
 *  - "final": today's last trade from the live source, once the session has ended (the closing
 *    auction trade at 13:30, or from 13:34 for a postponed close). It is labelled as awaiting the
 *    official daily data and is replaced by it when that arrives.
 *  - "previous": the latest earlier trading day, used before today's close is known.
 */
export interface CloseDay {
  date: string;
  close: number;
  changePercent: number | null;
  /** Point change of the close, when the source gives one. */
  changePoints: number | null;
  /** The day's trading range: the daily row's high/low, or the live
   * quote's session range for a "final" close. Null when unknown. */
  high: number | null;
  low: number | null;
  kind: "official" | "final" | "previous";
}
const taipeiDay = (ms: number) =>
  new Date(ms).toLocaleDateString("sv-SE", { timeZone: "Asia/Taipei" });
export function closeDay(
  s: Pick<Snapshot, "dailySummaries"> &
    Partial<Pick<Snapshot, "quotes" | "now">>,
  c: string,
): CloseDay | undefined {
  const d = s.dailySummaries[c]?.days.at(-1);
  const q = s.quotes?.[c];
  if (s.now && q && q.price != null) {
    const today = taipeiDay(s.now),
      closeAt = Date.parse(`${today}T13:30:00+08:00`),
      traded = q.tradeAt ?? 0;
    const sameDay = q.date === today.replaceAll("-", "");
    const settled =
      traded >= closeAt ||
      (s.now >= closeAt + 4 * 60_000 && q.sourceAt >= closeAt);
    if (d?.date === today)
      return {
        date: d.date,
        close: d.close,
        changePercent: d.changePercent,
        changePoints: d.change,
        high: d.high ?? null,
        low: d.low ?? null,
        kind: "official",
      };
    if (sameDay && settled && s.now >= closeAt)
      return {
        date: today,
        close: q.price,
        changePercent: q.changePercent,
        changePoints: q.previous != null ? q.price - q.previous : null,
        high: q.dayHigh ?? null,
        low: q.dayLow ?? null,
        kind: "final",
      };
  }
  return d
    ? {
        date: d.date,
        close: d.close,
        changePercent: d.changePercent,
        changePoints: d.change,
        high: d.high ?? null,
        low: d.low ?? null,
        kind: s.now && d.date === taipeiDay(s.now) ? "official" : "previous",
      }
    : undefined;
}
export function latestCloseDate(
  s: Pick<Snapshot, "watchlist" | "dailySummaries"> &
    Partial<Pick<Snapshot, "quotes" | "now">>,
): string | null {
  const dates = s.watchlist
    .map((c) => closeDay(s, c)?.date)
    .filter((d): d is string => Boolean(d));
  return dates.sort().at(-1) ?? null;
}
export const ruleText = (r: Rule) =>
  `${displaySymbol(r.symbol)} · ${METRIC_LABEL[r.metric] ?? r.metric}${r.direction === "above" ? "升至" : "降至"} ${number(r.threshold)}${r.metric === "price" || r.metric === "mid" ? " 元" : "%"}`;
/** The price inputs a portfolio valuation needs for one code. `previous` is
 * the prior close behind today's change; either may be missing. */
export interface PositionQuote {
  price: number | null;
  previous: number | null;
}
export type Currency = "TWD" | "USD";
export interface PositionSummary {
  code: string;
  currency: Currency;
  /** Total shares across all lots. */
  shares: number;
  /** Volume-weighted average purchase price; null when no shares are held. */
  avgPrice: number | null;
  /** Total purchase cost, Σ price × shares over the lots. */
  cost: number;
  price: number | null;
  marketValue: number | null;
  /** (price − previous) × shares; null when price or previous is unknown. */
  dayPnl: number | null;
  /** dayPnl as a percent of yesterday's value (previous × shares). */
  dayPnlPercent: number | null;
  /** (price − avgPrice) × shares; null when there is no price. */
  totalPnl: number | null;
  /** totalPnl as a percent of cost; null when there is no price. */
  totalPnlPercent: number | null;
}
export interface CurrencyTotal {
  currency: Currency;
  /** Σ market value over positions that have a price; null when none do. */
  marketValue: number | null;
  /** Σ day P&L over positions with a known previous close; null when none. */
  dayPnl: number | null;
  /** dayPnl as a percent of yesterday's market value of those same positions. */
  dayPnlPercent: number | null;
  /** Σ total P&L over priced positions; null when none are priced. */
  totalPnl: number | null;
  /** totalPnl as a percent of the cost of the priced positions. */
  totalPnlPercent: number | null;
}
export interface PortfolioSummary {
  positions: PositionSummary[];
  /** One entry per currency actually held, TWD first. Never summed across. */
  totals: CurrencyTotal[];
}
/**
 * Values a portfolio from purchase lots and caller-supplied prices. Pure: the
 * caller decides where each code's price/previous came from (live quote, or
 * the official close after the Taiwan session). Yahoo-routed codes are USD,
 * everything else TWD, and the two currencies are totalled separately.
 */
export function portfolioSummary(
  positions: Position[],
  quotesByCode: Record<string, PositionQuote>,
): PortfolioSummary {
  const rows: PositionSummary[] = positions.map((p) => {
    const currency: Currency = isYahooSymbol(p.code) ? "USD" : "TWD";
    const shares = p.lots.reduce((n, l) => n + l.shares, 0);
    const cost = p.lots.reduce((n, l) => n + l.price * l.shares, 0);
    const avgPrice = shares > 0 ? cost / shares : null;
    const q = quotesByCode[p.code];
    const price = q?.price ?? null;
    const previous = q?.previous ?? null;
    const marketValue = price != null ? price * shares : null;
    const dayPnl =
      price != null && previous != null ? (price - previous) * shares : null;
    const prevValue = previous != null ? previous * shares : null;
    const dayPnlPercent =
      dayPnl != null && prevValue != null && prevValue !== 0
        ? (dayPnl / prevValue) * 100
        : null;
    const totalPnl =
      price != null && avgPrice != null ? (price - avgPrice) * shares : null;
    const totalPnlPercent =
      totalPnl != null && cost > 0 ? (totalPnl / cost) * 100 : null;
    return {
      code: p.code,
      currency,
      shares,
      avgPrice,
      cost,
      price,
      marketValue,
      dayPnl,
      dayPnlPercent,
      totalPnl,
      totalPnlPercent,
    };
  });
  const totals: CurrencyTotal[] = [];
  for (const currency of ["TWD", "USD"] as Currency[]) {
    const mine = rows.filter((r) => r.currency === currency);
    if (!mine.length) continue;
    const priced = mine.filter((r) => r.marketValue != null);
    const marketValue = priced.length
      ? priced.reduce((n, r) => n + r.marketValue!, 0)
      : null;
    const withDay = mine.filter((r) => r.dayPnl != null);
    const dayPnl = withDay.length
      ? withDay.reduce((n, r) => n + r.dayPnl!, 0)
      : null;
    const prevValue = withDay.reduce(
      (n, r) => n + quotesByCode[r.code]!.previous! * r.shares,
      0,
    );
    const dayPnlPercent =
      dayPnl != null && prevValue !== 0 ? (dayPnl / prevValue) * 100 : null;
    const totalPnl = priced.length
      ? priced.reduce((n, r) => n + (r.totalPnl ?? 0), 0)
      : null;
    const pricedCost = priced.reduce((n, r) => n + r.cost, 0);
    const totalPnlPercent =
      totalPnl != null && pricedCost > 0 ? (totalPnl / pricedCost) * 100 : null;
    totals.push({
      currency,
      marketValue,
      dayPnl,
      dayPnlPercent,
      totalPnl,
      totalPnlPercent,
    });
  }
  return { positions: rows, totals };
}
export interface CombinedTotal {
  /** Total market value expressed in TWD. */
  total: number;
  /** Today's P&L expressed in TWD. */
  dayPnl: number;
  /** dayPnl as a percent of yesterday's combined value; null at zero. */
  dayPnlPercent: number | null;
  /** The USD/TWD rate used, or null when no conversion was needed. */
  rate: number | null;
}
/**
 * Combines the per-currency portfolio totals into a single TWD figure for
 * the header: TWD as-is plus USD converted at `rate` (TWD per USD, from
 * the internal TWD=X quote). Returns null whenever the combined number
 * would be a guess: no holdings at all, a currency total whose market
 * value or day P&L is unknown, or USD holdings without a usable rate.
 * A TWD-only portfolio needs no rate.
 */
export function combinedTotals(
  totals: CurrencyTotal[],
  rate: number | null | undefined,
): CombinedTotal | null {
  if (!totals.length) return null;
  let total = 0;
  let dayPnl = 0;
  let usedRate: number | null = null;
  for (const t of totals) {
    if (t.marketValue == null || t.dayPnl == null) return null;
    let factor = 1;
    if (t.currency === "USD") {
      if (rate == null || !Number.isFinite(rate) || rate <= 0) return null;
      factor = rate;
      usedRate = rate;
    }
    total += t.marketValue * factor;
    dayPnl += t.dayPnl * factor;
  }
  const previous = total - dayPnl;
  return {
    total,
    dayPnl,
    dayPnlPercent: previous !== 0 ? (dayPnl / previous) * 100 : null,
    rate: usedRate,
  };
}
/** Columns of the positions table a click on the header can sort by. */
export type PositionSortKey =
  | "code"
  | "shares"
  | "marketValue"
  | "dayPnl"
  | "totalPnl";
export type PositionSortDir = "asc" | "desc";
export interface PositionSort {
  key: PositionSortKey;
  dir: PositionSortDir;
}
/**
 * The sort a header click asks for next: a fresh column starts descending
 * for numbers (biggest first) and ascending for the code column; clicking
 * the active column again flips its direction.
 */
export function nextPositionSort(
  current: PositionSort | null,
  key: PositionSortKey,
): PositionSort {
  if (current?.key === key)
    return { key, dir: current.dir === "asc" ? "desc" : "asc" };
  return { key, dir: key === "code" ? "asc" : "desc" };
}
/**
 * Orders positions by one summary column, using the values portfolioSummary
 * computed (never recomputed here). Rows whose value is unknown — a P&L
 * whose inputs are missing — sink to the bottom in both directions, and
 * equal values keep the recorded order. A null sort returns the recorded
 * order untouched.
 */
export function sortedPositions(
  positions: Position[],
  rows: PositionSummary[],
  sort: PositionSort | null,
): Position[] {
  if (!sort) return positions;
  const byCode = new Map(rows.map((r) => [r.code, r]));
  const sign = sort.dir === "asc" ? 1 : -1;
  const key = sort.key;
  return [...positions].sort((a, b) => {
    if (key === "code")
      return sign * (a.code < b.code ? -1 : a.code > b.code ? 1 : 0);
    const va = byCode.get(a.code)?.[key] ?? null;
    const vb = byCode.get(b.code)?.[key] ?? null;
    if (va == null) return vb == null ? 0 : 1;
    if (vb == null) return -1;
    return sign * (va - vb);
  });
}
/** One minute of the portfolio's aggregate value on the session grid. */
export interface PortfolioValuePoint {
  at: number;
  value: number;
}
export interface PortfolioIntraday {
  /** Aggregate values per sampled minute, oldest first. Minutes where no
   * held code had any known price are omitted, never invented. */
  points: PortfolioValuePoint[];
  /** Σ shares × previous close over the TWD positions whose previous close
   * is known; null when none is. The dashed baseline of the day chart. */
  previousTotal: number | null;
  firstAt: number | null;
  lastAt: number | null;
  lastValue: number | null;
  high: number | null;
  low: number | null;
}
/**
 * Aggregates the day's minute samples of the TWD holdings into one total
 * market-value series — the portfolio equivalent of Google Finance's 1D
 * chart. Display-only: samples come from the intraday (Yahoo minute) path
 * and the result never feeds rules or events.
 *
 * Each code's price on the shared minute grid is its latest sample at or
 * before that minute (forward-fill across gaps in its own samples); before
 * its first sample it counts at its previous close, so the total starts at
 * the previous-close baseline. A code with neither a sample yet nor a known
 * previous close contributes nothing at that minute, and a minute where no
 * code contributes is left out. USD (Yahoo-routed) positions are excluded:
 * the chart is the TWD book only.
 */
export function portfolioIntradaySeries(
  positions: Position[],
  samplesByCode: Record<string, IntradayPoint[] | null | undefined>,
  previousByCode: Record<string, number | null | undefined>,
): PortfolioIntraday {
  const empty: PortfolioIntraday = {
    points: [],
    previousTotal: null,
    firstAt: null,
    lastAt: null,
    lastValue: null,
    high: null,
    low: null,
  };
  const legs = positions
    .filter((p) => !isYahooSymbol(p.code))
    .map((p) => ({
      shares: p.lots.reduce((n, l) => n + l.shares, 0),
      previous: previousByCode[p.code] ?? null,
      samples: (samplesByCode[p.code] ?? [])
        .filter((pt) => pt.price != null && Number.isFinite(pt.price))
        .map((pt) => ({
          minute: Math.floor(pt.at / 60000) * 60000,
          price: pt.price!,
        }))
        .sort((a, b) => a.minute - b.minute),
    }))
    .filter((l) => l.shares > 0);
  if (!legs.length) return empty;
  const withPrev = legs.filter((l) => l.previous != null);
  const previousTotal = withPrev.length
    ? withPrev.reduce((n, l) => n + l.previous! * l.shares, 0)
    : null;
  const grid = [
    ...new Set(legs.flatMap((l) => l.samples.map((s) => s.minute))),
  ].sort((a, b) => a - b);
  const points: PortfolioValuePoint[] = [];
  const cursor = legs.map(() => 0);
  const known: (number | null)[] = legs.map((l) => l.previous);
  for (const minute of grid) {
    legs.forEach((leg, i) => {
      while (
        cursor[i]! < leg.samples.length &&
        leg.samples[cursor[i]!]!.minute <= minute
      ) {
        known[i] = leg.samples[cursor[i]!]!.price;
        cursor[i]!++;
      }
    });
    let total = 0,
      any = false;
    legs.forEach((leg, i) => {
      if (known[i] != null) {
        total += known[i]! * leg.shares;
        any = true;
      }
    });
    if (any) points.push({ at: minute, value: total });
  }
  if (!points.length) return { ...empty, previousTotal };
  const values = points.map((p) => p.value);
  return {
    points,
    previousTotal,
    firstAt: points[0]!.at,
    lastAt: points.at(-1)!.at,
    lastValue: points.at(-1)!.value,
    high: Math.max(...values),
    low: Math.min(...values),
  };
}
/**
 * Parses pasted position lots, one per line: `CODE, DATE, PRICE, SHARES` with
 * the date optional (three fields read as CODE, PRICE, SHARES); separators may
 * be commas, tabs or spaces. Mirrors the core's import_positions line rules so
 * the browser preview behaves like the desktop core. Codes are only lightly
 * normalized here (trim + uppercase); the core canonicalizes for real.
 */
export function parsePositionsImport(text: string): {
  lots: { code: string; lot: Lot }[];
  failedLines: number[];
} {
  const lots: { code: string; lot: Lot }[] = [];
  const failedLines: number[] = [];
  const validDate = (s: string) => {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(s)) return false;
    const d = new Date(`${s}T00:00:00Z`);
    return !Number.isNaN(d.getTime()) && d.toISOString().slice(0, 10) === s;
  };
  text.split(/\r?\n/).forEach((line, i) => {
    if (!line.trim()) return;
    const fields = line
      .split(/[,，\t]/)
      .flatMap((part) => part.split(/\s+/))
      .filter(Boolean);
    const [code, a, b, c] =
      fields.length === 4
        ? [fields[0], fields[1], fields[2], fields[3]]
        : fields.length === 3
          ? [fields[0], "", fields[1], fields[2]]
          : [undefined, undefined, undefined, undefined];
    const price = Number(b),
      shares = Number(c);
    if (
      !code ||
      (a !== "" && !validDate(a!)) ||
      !/^[A-Za-z0-9.^=]+$/.test(code) ||
      !Number.isFinite(price) ||
      price <= 0 ||
      !Number.isFinite(shares) ||
      shares <= 0
    ) {
      failedLines.push(i + 1);
      return;
    }
    lots.push({
      code: code.toUpperCase(),
      lot: { date: a ?? "", price, shares },
    });
  });
  return { lots, failedLines };
}
const METRIC_LABEL: Record<string, string> = {
  price: "成交價",
  changePercent: "漲跌幅",
  mid: "買賣中間價（委託價）",
  midChangePercent: "漲跌幅（買賣中間價）",
};
// Adapted from Terminalol's escaped fixed-width rows; stock-specific data stays separate.
export function terminalRows(s: Snapshot): string {
  return s.watchlist
    .map((c) => {
      const q = s.quotes[c];
      const d =
        !s.marketOpen && !s.demo && !isYahooSymbol(c)
          ? closeDay(s, c)
          : undefined;
      const name = [
        ...(s.settings.hideNames
          ? displaySymbol(c)
          : (q?.name ?? "等待資料")),
      ].slice(0, 8).join("");
      const cells = [...name].reduce(
        (n, c) => n + (/[^\u0000-\u00ff]/.test(c) ? 2 : 1),
        0,
      );
      return `${displaySymbol(c).padEnd(8)}${name}${" ".repeat(Math.max(0, 16 - cells))} ${number(d?.close ?? livePrice(q).price).padStart(10)} ${change(d ? d.changePercent : livePrice(q).pct).padStart(9)}  ${d ? "CLOSE " + d.date : stale(q, s.now) ? "STALE" : livePrice(q).basis === "book" ? "BOOK" : livePrice(q).basis === "none" ? "NO PRICE" : "OK"}`;
    })
    .join("\n");
}
