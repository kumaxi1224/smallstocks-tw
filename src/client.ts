import { invoke, isTauri } from "@tauri-apps/api/core";
import type { Snapshot, DailyView, Position, MonthlySnapshot } from "./model.ts";
import { parsePositionsImport } from "./model.ts";
export const desktop = isTauri();
let preview: Snapshot = {
  watchlist: ["t00", "2330", "0050"],
  groups: [{ name: "自選1", codes: ["t00", "2330", "0050"] }],
  activeGroup: 0,
  positions: [],
  monthlySnapshots: [],
  names: {},
  dailySummaries: {},
  intraday: {},
  portfolioHistory: {},
  settings: {
    appearance: "small",
    paused: false,
    refreshSeconds: 15,
    alwaysOnTop: true,
    hideNames: false,
    showChangePoints: true,
    quoteSource: "mis",
    sidecarPort: 18765,
    sidecarAutoStart: false,
    sidecarDir: "",
    windowOpacity: 1,
    telegramEnabled: false,
    telegramChatId: "",
    telegramTokenSet: false,
    positionDropAlert: true,
    backupEnabled: false,
    backupDir: "",
    chatWindow: false,
  },
  quotes: {},
  rules: [],
  events: [],
  demo: true,
  error: null,
  sourceNote: null,
  lastFetch: null,
  nextPoll: 0,
  marketOpen: true,
  now: Date.now(),
  revision: 0,
};
const old = localStorage.getItem("smallstocks-preview");
if (!desktop && old) {
  try {
    preview = JSON.parse(old);
  } catch {
    /* Preview settings may be discarded. */
  }
}
export async function request(
  tool: string,
  args: Record<string, unknown> = {},
): Promise<Snapshot> {
  if (desktop) return invoke<Snapshot>("core_request", { tool, args });
  preview.groups ??= [{ name: "自選1", codes: [...preview.watchlist] }];
  preview.activeGroup ??= 0;
  preview.positions ??= [];
  preview.monthlySnapshots ??= [];
  if (tool !== "import_positions") delete preview.positionsImport;
  if (tool === "add_lot") {
    const code = String(args.code ?? "")
      .trim()
      .toUpperCase();
    const lot = {
      date: String(args.date ?? ""),
      price: Number(args.price),
      shares: Number(args.shares),
    };
    const found = preview.positions.find((p) => p.code === code);
    if (found) found.lots.push(lot);
    else preview.positions.push({ code, lots: [lot] });
  }
  if (tool === "delete_lot") {
    const p = preview.positions.find((p) => p.code === args.code);
    if (p) {
      p.lots.splice(args.index as number, 1);
      if (!p.lots.length)
        preview.positions = preview.positions.filter((x) => x !== p);
    }
  }
  if (tool === "delete_position")
    preview.positions = preview.positions.filter((p) => p.code !== args.code);
  if (tool === "record_monthly_snapshot") {
    // Same first-write-wins rule as the core: an existing month is a no-op.
    const month = String(args.month ?? "");
    if (!preview.monthlySnapshots.some((s) => s.month === month))
      preview.monthlySnapshots.push({
        month,
        totalTwd: Number(args.totalTwd),
        twdValue: Number(args.twdValue),
        usdValue: (args.usdValue as number | null) ?? null,
        rate: (args.rate as number | null) ?? null,
        recordedOn: String(args.recordedOn ?? ""),
      });
  }
  if (tool === "delete_monthly_snapshot")
    preview.monthlySnapshots = preview.monthlySnapshots.filter(
      (s) => s.month !== args.month,
    );
  if (tool === "import_positions") {
    const parsed = parsePositionsImport(String(args.text ?? ""));
    for (const { code, lot } of parsed.lots) {
      const found = preview.positions.find((p) => p.code === code);
      if (found) found.lots.push(lot);
      else preview.positions.push({ code, lots: [lot] });
    }
    preview.positionsImport = {
      added: parsed.lots.length,
      failedLines: parsed.failedLines,
    };
  }
  if (tool === "set_settings") {
    // The preview mirrors the core's token discipline: a submitted token
    // is never stored, only the fact that one was set.
    const { telegramBotToken, telegramClearToken, ...rest } = args;
    Object.assign(preview.settings, rest);
    if (typeof telegramBotToken === "string" && telegramBotToken.trim())
      preview.settings.telegramTokenSet = true;
    if (telegramClearToken === true) preview.settings.telegramTokenSet = false;
  }
  if (tool === "set_watchlist") {
    preview.watchlist = args.symbols as string[];
    preview.groups[preview.activeGroup]!.codes = preview.watchlist;
  }
  if (tool === "add_watch_group") {
    const name =
      (args.name as string) || `自選${preview.groups.length + 1}`;
    preview.groups.push({ name, codes: [] });
    preview.activeGroup = preview.groups.length - 1;
    preview.watchlist = [];
  }
  if (tool === "rename_watch_group")
    preview.groups[args.index as number]!.name = args.name as string;
  if (tool === "delete_watch_group") {
    preview.groups.splice(args.index as number, 1);
    preview.activeGroup = Math.min(
      preview.activeGroup,
      preview.groups.length - 1,
    );
    preview.watchlist = preview.groups[preview.activeGroup]!.codes;
  }
  if (tool === "set_active_group") {
    preview.activeGroup = args.index as number;
    preview.watchlist = preview.groups[preview.activeGroup]!.codes;
  }
  if (tool === "upsert_rule") {
    const id = (args.id as string) ?? crypto.randomUUID();
    preview.rules = preview.rules.filter((r) => r.id !== id);
    preview.rules.push({
      id,
      ...args,
      firedDay: null,
    } as unknown as Snapshot["rules"][number]);
  }
  if (tool === "delete_rule")
    preview.rules = preview.rules.filter((r) => r.id !== args.id);
  preview.now = Date.now();
  if (tool !== "get_status") preview.revision++;
  const quoteCodes = [
    ...new Set([
      ...preview.watchlist,
      ...preview.positions.map((p) => p.code),
    ]),
  ];
  for (const [i, c] of quoteCodes.entries()) {
    const base = c === "t00" ? 24180 : c === "0050" ? 62.4 : 1040;
    const p = base + (i - 1) * 0.2;
    preview.quotes[c] = {
      symbol: c,
      name:
        c === "t00"
          ? "加權指數"
          : c === "2330"
            ? "台積電"
            : c === "0050"
              ? "元大台灣50"
              : "示範標的",
      price: p,
      previous: base - 3,
      bid: p - 0.5,
      ask: p + 0.5,
      changePercent: (p / (base - 3) - 1) * 100,
      sourceAt: preview.now,
      receivedAt: preview.now,
      date: "",
      source: "合成示範資料",
    };
  }
  preview.lastFetch = preview.now;
  preview.dailySummaries ??= {};
  preview.intraday ??= {};
  for (const c of preview.watchlist) {
    preview.dailySummaries[c] = await daily(
      c,
      new Date().toISOString().slice(0, 7),
    );
  }
  // Synthetic minute series cover held codes too, so the portfolio day
  // chart has something to aggregate in the preview.
  const intradayCodes = [
    ...new Set([
      ...preview.watchlist,
      ...preview.positions.map((p) => p.code),
    ]),
  ];
  for (const c of intradayCodes) {
    const start = new Date(preview.now);
    start.setHours(9, 0, 0, 0);
    const base = preview.quotes[c]!.price!;
    preview.intraday[c] = {
      series: {
        symbol: c,
        date: preview.now.toString(),
        source: "合成示範分鐘線",
        fetchedAt: preview.now,
        partial: false,
        points: Array.from({ length: 271 }, (_, i) => ({
          at: start.getTime() + i * 60000,
          price: base * (1 + Math.sin(i / 24) * 0.008),
        })),
      },
      pending: false,
      error: null,
      demo: true,
    };
  }
  // Synthetic aggregate history for the portfolio range tabs: one seeded
  // walk, decimated per range, so the preview chart has dates to draw.
  preview.portfolioHistory ??= {};
  {
    const walk: { at: number; value: number }[] = [];
    let v = 880000;
    for (let i = 260; i >= 0; i--) {
      v *= 1 + Math.sin(i / 17) * 0.004 + 0.0012;
      walk.push({ at: preview.now - i * 86400000, value: Math.round(v) });
    }
    for (const r of ["5D", "1M", "6M", "YTD", "1Y", "5Y", "MAX"]) {
      const n = r === "5D" ? 5 : r === "1M" ? 22 : 48;
      const step = Math.max(1, Math.floor(walk.length / n));
      const pts = walk.filter((_, i) => i % step === 0).slice(-n);
      preview.portfolioHistory[r] = {
        points: pts,
        baseline: pts[0]?.value ?? null,
        uncovered: 0,
        pending: false,
      };
    }
  }
  localStorage.setItem("smallstocks-preview", JSON.stringify(preview));
  return structuredClone(preview);
}
/**
 * Desktop only: the core writes the export_data payload to a file itself
 * and returns the full path written. A Blob download from the settings
 * page is silently ignored by WebView2, so the desktop export goes through
 * here instead; the browser preview keeps the exportData() Blob path.
 */
export async function exportDataFile(): Promise<string> {
  const r = await invoke<{ path: string }>("core_request", {
    tool: "export_data_file",
    args: {},
  });
  return r.path;
}
/**
 * Desktop only: sends the Telegram test message with the settings stored
 * in the core. The browser preview has no bot and no token, so it refuses.
 */
export async function telegramTest(): Promise<void> {
  if (!desktop) throw new Error("瀏覽器預覽無法發送 Telegram 訊息");
  await invoke("core_request", { tool: "telegram_test", args: {} });
}
/** Which of the locally stored Shioaji keys exist — booleans only. */
export interface ShioajiKeysStatus {
  apiKeySet: boolean;
  secretKeySet: boolean;
}
/**
 * Desktop only: the Shioaji keys live in their own file in the data
 * folder, outside the core and its state. These wrappers are the only
 * way in or out, and the values travel in exactly one direction (into
 * shioajiKeysSet); the status reports existence, never a value.
 */
export async function shioajiKeysStatus(): Promise<ShioajiKeysStatus> {
  if (!desktop) throw new Error("瀏覽器預覽沒有本機金鑰");
  return invoke<ShioajiKeysStatus>("shioaji_keys_status");
}
export async function shioajiKeysSet(
  apiKey: string,
  secretKey: string,
): Promise<void> {
  if (!desktop) throw new Error("瀏覽器預覽無法儲存金鑰");
  await invoke("shioaji_keys_set", { apiKey, secretKey });
}
export async function shioajiKeysClear(): Promise<void> {
  if (!desktop) throw new Error("瀏覽器預覽無法清除金鑰");
  await invoke("shioaji_keys_clear");
}
/**
 * Desktop only: ask the shell to start the sidecar now (same conditions
 * and probe as the automatic path). Returns a line describing the
 * outcome for the settings page to show.
 */
export async function sidecarStartNow(): Promise<string> {
  if (!desktop) throw new Error("瀏覽器預覽無法啟動 sidecar");
  return invoke<string>("sidecar_start_now");
}
/**
 * Desktop only: writes a state backup into the configured folder right
 * away and returns the full path written.
 */
export async function backupNow(): Promise<string> {
  if (!desktop) throw new Error("瀏覽器預覽無法備份設定");
  const r = await invoke<{ path: string }>("core_request", {
    tool: "backup_now",
    args: {},
  });
  return r.path;
}
/** The export_data payload as a JSON string (desktop core or preview). */
export async function exportData(): Promise<string> {
  if (desktop)
    return invoke<string>("core_request", { tool: "export_data", args: {} });
  return JSON.stringify(
    {
      app: "maxi-stocks",
      version: 1,
      groups: preview.groups ?? [],
      activeGroup: preview.activeGroup ?? 0,
      positions: preview.positions ?? [],
      monthlySnapshots: preview.monthlySnapshots ?? [],
    },
    null,
    2,
  );
}
/**
 * Applies an export_data JSON string through import_data. The desktop core
 * validates and tallies; the preview checks the envelope and applies the
 * file as-is. Rejects with the same messages the core would give.
 */
export async function importData(jsonText: string): Promise<Snapshot> {
  if (desktop)
    return invoke<Snapshot>("core_request", {
      tool: "import_data",
      args: { json: jsonText },
    });
  let v: {
    app?: string;
    version?: number;
    groups?: Snapshot["groups"];
    activeGroup?: number;
    positions?: Position[];
    monthlySnapshots?: MonthlySnapshot[];
  };
  try {
    v = JSON.parse(jsonText);
  } catch {
    throw new Error("匯入檔案不是有效的 JSON");
  }
  // Exports written before the v0.1.32 rebrand carry the legacy tag.
  if (v?.app !== "maxi-stocks" && v?.app !== "smallstocks-tw")
    throw new Error("這不是 Maxi stocks 的匯出檔案");
  if (v?.version !== 1) throw new Error("不支援的匯出檔案版本");
  if (!Array.isArray(v.groups) || !v.groups.length)
    throw new Error("匯出檔案沒有任何分組");
  preview.groups = v.groups;
  preview.activeGroup = Math.min(
    Math.max(0, v.activeGroup ?? 0),
    v.groups.length - 1,
  );
  preview.watchlist = [...(v.groups[preview.activeGroup]?.codes ?? [])];
  preview.positions = Array.isArray(v.positions) ? v.positions : [];
  preview.monthlySnapshots = Array.isArray(v.monthlySnapshots)
    ? v.monthlySnapshots
    : [];
  preview.dataImport = {
    groups: v.groups.length,
    positions: preview.positions.length,
    lots: preview.positions.reduce((n, p) => n + p.lots.length, 0),
    skippedSymbols: [],
    skippedLots: 0,
    skippedSnapshots: 0,
  };
  preview.revision++;
  localStorage.setItem("smallstocks-preview", JSON.stringify(preview));
  return structuredClone(preview);
}
export async function daily(symbol: string, month: string): Promise<DailyView> {
  if (desktop)
    return invoke("core_request", {
      tool: "get_daily_history",
      args: { symbol, month },
    });
  const days = [];
  const [y, m] = month.split("-").map(Number);
  const today = new Date().toISOString().slice(0, 10);
  for (let d = 1; d <= 31; d++) {
    const date = new Date(Date.UTC(y!, m! - 1, d));
    const key = date.toISOString().slice(0, 10);
    if (
      date.getUTCMonth() !== m! - 1 ||
      [0, 6].includes(date.getUTCDay()) ||
      key > today
    )
      continue;
    const close = Number((1000 + Math.sin(d / 4) * 25).toFixed(2));
    days.push({
      date: key,
      open: close - 2,
      high: close + 5,
      low: close - 8,
      close,
      change: 2,
      changePercent: (2 / (close - 2)) * 100,
      volume: 1000000 + d * 10000,
      volumeUnit: "股",
    });
  }
  return {
    symbol,
    days,
    sources: ["合成示範資料"],
    fetchedAt: Date.now(),
    pending: false,
    error: null,
    demo: true,
  };
}
export async function search(
  query: string,
): Promise<{ symbol: string; name: string }[]> {
  if (desktop) return invoke("search_stocks", { query });
  return [
    { symbol: "2330", name: "台積電" },
    { symbol: "0050", name: "元大台灣50" },
    { symbol: "6488", name: "環球晶" },
  ].filter((r) => r.name.includes(query) || r.symbol.includes(query));
}
