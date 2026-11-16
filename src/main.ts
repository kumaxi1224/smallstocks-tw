import "./style.css";
import "./disguise.css";
import "./minimal.css";
import {
  isMac,
  primaryPressed,
  privacyPressed,
  privacyShortcutLabel,
} from "./platform.ts";
import {
  currentMonitor,
  getCurrentWindow,
  LogicalSize,
  PhysicalPosition,
} from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { mcpSetup } from "./mcp-setup.ts";
import type { McpKind } from "./mcp-setup.ts";
import { desktop, request, search, daily, exportData, exportDataFile, importData, telegramTest, backupNow, shioajiKeysStatus, shioajiKeysSet, shioajiKeysClear, sidecarStartNow } from "./client.ts";
import {
  esc,
  number,
  change,
  points,
  shellOpacity,
  shioajiKeysLabel,
  shouldKeepScroll,
  sizeSignature,
  time,
  stale,
  ruleText,
  displaySymbol,
  isYahooSymbol,
  latestCloseDate,
  livePrice,
  closeDay,
  aging,
  oldestDataAt,
  portfolioSummary,
  combinedTotals,
  monthlyRows,
  portfolioIntradaySeries,
  nextPositionSort,
  sortedPositions,
  formatMoney,
  BOOK_NOTE,
} from "./model.ts";
import { historyBody } from "./daily-view.ts";
import { trendCell, portfolioChart } from "./inline-trend.ts";
import { disguiseMarkup, bindDisguise, updateDisguise } from "./disguise.ts";
import {
  minimalMarkup,
  bindMinimal,
  updateMinimal,
  hideDetail,
} from "./minimal.ts";
import type { SkinAction } from "./disguise.ts";
import type {
  DailyView,
  Snapshot,
  Appearance,
  Rule,
  PositionQuote,
  PositionSort,
  PositionSortKey,
} from "./model.ts";

document.body.dataset.platform = isMac ? "macos" : "windows";
const app = document.querySelector<HTMLDivElement>("#app")!;
let state: Snapshot;
let tab = "watch";
/** The view render() last painted, and a one-shot reset request from a
 * deliberate navigation action (group switch, rule edit). Same-view
 * re-renders keep the scroll place; view switches start at the top. */
let renderedTab: string | null = null;
let scrollResetPending = false;
let editing: Rule | undefined;
let busy = false;
let searchVersion = 0;
let dailySymbol = "";
let dailyMonth = new Intl.DateTimeFormat("sv-SE", {
  timeZone: "Asia/Taipei",
  year: "numeric",
  month: "2-digit",
}).format(new Date());
let dailyVersion = 0;
let dailyData: DailyView | undefined;
let dailyBusy = false;
let lastWindowTitle = "";
let mcpMessage = "";
let mcpExe = "";
// navigator.clipboard first; the old selection copy covers a web view that refuses it.
async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text);
  } catch (error) {
    const box = document.createElement("textarea");
    box.value = text;
    box.style.position = "fixed";
    box.style.opacity = "0";
    document.body.append(box);
    box.select();
    const ok = document.execCommand("copy");
    box.remove();
    if (!ok) throw error;
  }
}
async function loadDaily() {
  if (dailyBusy) return;
  const version = ++dailyVersion;
  const requestedSymbol = dailySymbol,
    requestedMonth = dailyMonth;
  dailyBusy = true;
  try {
    const data = await daily(requestedSymbol, requestedMonth);
    if (
      version !== dailyVersion ||
      tab !== "daily" ||
      requestedSymbol !== dailySymbol ||
      requestedMonth !== dailyMonth
    )
      return;
    dailyData = data;
    const el = document.querySelector("#daily-body");
    if (el) el.innerHTML = historyBody(data);
  } catch (e) {
    showError(e);
  } finally {
    dailyBusy = false;
    if (
      tab === "daily" &&
      dailySymbol &&
      (requestedSymbol !== dailySymbol || requestedMonth !== dailyMonth)
    )
      void loadDaily();
  }
}
function dailyPage() {
  if (!state.watchlist.includes(dailySymbol))
    dailySymbol = state.watchlist[0] ?? "";
  content.innerHTML = `<div class="pane"><h1>一天一點，回看走勢</h1><p class="intro">正式日行情與收盤摘要，不會觸發即時通知。</p><div class="daily-controls"><label>自選標的<select id="daily-symbol">${state.watchlist.map((c) => `<option value="${esc(c)}" ${c === dailySymbol ? "selected" : ""}>${esc(state.quotes[c]?.name ?? displaySymbol(c))} · ${esc(displaySymbol(c))}</option>`).join("")}</select></label><label>月份<input id="daily-month" type="month" min="2000-01" max="${new Date(state.now).toLocaleDateString("sv-SE", { timeZone: "Asia/Taipei" }).slice(0, 7)}" value="${dailyMonth}"></label></div><div id="daily-body">${dailyData && dailyData.symbol === dailySymbol ? historyBody(dailyData) : '<div class="empty">正在取得每日行情…</div>'}</div></div>`;
  document.querySelector("#daily-symbol")!.addEventListener("change", (e) => {
    dailySymbol = (e.target as HTMLSelectElement).value;
    dailyData = undefined;
    dailyVersion++;
    dailyBusy = false;
    void loadDaily();
  });
  document.querySelector("#daily-month")!.addEventListener("change", (e) => {
    dailyMonth = (e.target as HTMLInputElement).value;
    dailyData = undefined;
    dailyVersion++;
    dailyBusy = false;
    void loadDaily();
  });
  if (dailySymbol) void loadDaily();
  else document.querySelector("#daily-body")!.textContent = "請先加入自選股。";
}
app.innerHTML = `<main class="shell"><header data-tauri-drag-region><span class="brand" data-tauri-drag-region><i></i><b>Maxi</b><span>stocks</span></span><span id="header-assets" class="header-assets"></span><div class="window-actions"><button id="row-display" title="切換數字、走勢與極簡" aria-label="切換顯示內容">數字</button><button id="manage" aria-label="＋ 管理自選" title="管理自選">＋</button><button id="pin" class="pin-btn" aria-pressed="true" aria-label="視窗保持置頂" title="已置頂：點一下取消置頂"><svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><path d="M9.8 1.2 14.8 6.2 13.4 7.6 12.3 7.2 9.4 10.1 9.9 12 8.5 13.4 5.6 10.5 2.2 13.9 1.5 13.2 4.9 9.8 2 6.9 3.4 5.5 5.3 6 8.2 3.1 7.8 2z" fill="currentColor"/></svg></button><button id="stealth" class="stealth-btn" aria-label="隱藏全部視窗" title="完全隱藏（含通知），監看持續。叫回：${privacyShortcutLabel}"><svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"><path d="M8 3.5C4.6 3.5 2 6 1 8c1 2 3.6 4.5 7 4.5s6-2.5 7-4.5c-1-2-3.6-4.5-7-4.5zm0 1.8a2.7 2.7 0 1 1 0 5.4 2.7 2.7 0 0 1 0-5.4z" fill="currentColor"/><path d="M2.3 2.3 13.7 13.7" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg></button><button id="hide" aria-label="縮到工作列" title="縮到工作列；監看持續。叫回：點工作列按鈕，或按 ${privacyShortcutLabel}">−</button><button id="quit" aria-label="結束程式" title="結束程式；監看也會停止">✕</button></div></header><nav aria-label="主選單"><button data-tab="watch" class="active">自選</button><button data-tab="positions">持股</button><button data-tab="daily">行情</button><button data-tab="rules">提醒 <span id="rule-count"></span></button><button data-tab="history">紀錄</button><button data-tab="settings" class="gear">外觀與設定</button></nav><div id="banner"></div><section id="content"></section><footer><span id="health"></span><span id="source">TWSE MIS</span></footer><div id="message" role="status" aria-live="polite"></div></main>`;
const content = app.querySelector<HTMLElement>("#content")!;
const showError = (e: unknown) => {
  const el = document.querySelector<HTMLElement>("#message")!;
  el.textContent = String(e);
  el.classList.add("visible");
  setTimeout(() => el.classList.remove("visible"), 6000);
};
async function mutate(tool: string, args: Record<string, unknown>) {
  try {
    state = await request(tool, args);
    apply();
    render();
  } catch (e) {
    showError(e);
  }
}
function skinAction(action: SkinAction) {
  if (["hide", "minimize", "maximize"].includes(action)) {
    hideDetail();
    if (!desktop) {
      showError("此操作由桌面視窗提供");
      return;
    }
    const win = getCurrentWindow();
    void (
      action === "hide"
        ? win.hide()
        : action === "minimize"
          ? win.minimize()
          : win.toggleMaximize()
    ).catch(showError);
    return;
  }
  if (action === "small") {
    tab = "watch";
    void mutate("set_settings", { appearance: "small" });
    return;
  }
  if (action === "pause" || action === "resume") {
    void mutate("set_settings", { paused: action === "pause" });
    return;
  }
  tab = action;
  editing = undefined;
  if (action === "manage") manage();
  else render();
}
// Other tabs (rates, alerts, history, settings) need more room than the watch strip. The window
// grows downward only: the width stays what it is, so a width the user chose is never replaced by a
// default. The default width is applied only when the appearance changes.
const defaultWidth = (a: string) =>
  a === "terminal" ? 520 : a === "txt" ? 500 : 360;
const PANE_HEIGHT = 420;
let sizedFor = "";
/** Signature of the factors the last resize() call sized for. While it is
 * unchanged, resize() leaves the window alone: re-asserting the computed
 * size on every poll re-render would undo a height the user dragged by
 * hand (pane pages always computed the same PANE_HEIGHT). */
let lastSizeSig = "";
async function keepOnScreen(width: number, height: number) {
  try {
    const win = getCurrentWindow(),
      scale = await win.scaleFactor(),
      monitor = await currentMonitor(),
      pos = await win.outerPosition();
    if (!monitor) return;
    const area = monitor.workArea ?? {
      position: monitor.position,
      size: monitor.size,
    };
    const right = area.position.x + area.size.width,
      bottom = area.position.y + area.size.height;
    const w = Math.round(width * scale),
      h = Math.round(height * scale);
    const x = Math.max(area.position.x, Math.min(pos.x, right - w)),
      y = Math.max(area.position.y, Math.min(pos.y, bottom - h));
    if (x !== pos.x || y !== pos.y)
      await win.setPosition(new PhysicalPosition(x, y));
  } catch {
    /* Placement is a courtesy; the resize itself already happened. */
  }
}
async function resize() {
  if (!desktop) return;
  const sig = sizeSignature({
    tab,
    appearance: state.settings.appearance,
    rowDisplay: state.settings.rowDisplay,
    watchCount: state.watchlist.length,
    demo: state.demo,
    eventCount: state.events.length,
  });
  if (sig === lastSizeSig) return;
  lastSizeSig = sig;
  const wide = tab !== "watch";
  const a = state.settings.appearance;
  const compact = a === "small" || a === "gray";
  const win = getCurrentWindow();
  const sameAppearance = sizedFor === a;
  sizedFor = a;
  if (!wide && state.settings.rowDisplay === "minimal") {
    await fitMinimal();
    return;
  }
  const width = sameAppearance
    ? Math.max(
        300,
        Math.round((await win.innerSize()).width / (await win.scaleFactor())),
      )
    : defaultWidth(a);
  const height = wide
    ? PANE_HEIGHT
    : compact
      ? Math.min(
          410,
          Math.max(
            100,
            76 +
              28 +
              // Numbers rows stack two lines (34px stride); trend rows
              // stay single-line (24px). Minimal returned via fitMinimal.
              state.watchlist.length *
                (state.settings.rowDisplay === "trend" ? 24 : 34) +
              (state.demo ? 20 : 0) +
              (state.events.length ? 18 : 0),
          ),
        )
      : a === "terminal"
        ? Math.min(
            480,
            Math.max(
              280,
              210 +
                state.watchlist.length * 18 +
                (state.demo ? 18 : 0) +
                Math.min(state.events.length, 3) * 18,
            ),
          )
        : Math.min(
            480,
            Math.max(
              300,
              210 + state.watchlist.length * 19 + (state.demo ? 19 : 0),
            ),
          );
  await win.setMinSize(new LogicalSize(300, 100));
  await win.setSize(new LogicalSize(width, height)).catch(showError);
  await keepOnScreen(width, height);
}
let fittingMinimal = false,
  fitAgain = false;
async function fitMinimal() {
  if (!desktop || tab !== "watch" || state.settings.rowDisplay !== "minimal")
    return;
  if (fittingMinimal) {
    fitAgain = true;
    return;
  }
  const list = content.querySelector<HTMLElement>(".minimal-list");
  if (!list) return;
  fittingMinimal = true;
  try {
    const win = getCurrentWindow(),
      scale = await win.scaleFactor(),
      size = await win.innerSize();
    const px = (el: Element | null) =>
      el ? el.getBoundingClientRect().height : 0;
    const padding = (el: Element | null) => {
      if (!el) return 0;
      const css = getComputedStyle(el);
      return parseFloat(css.paddingTop) + parseFloat(css.paddingBottom);
    };
    let height = px(list) + 2;
    const native = content.querySelector(".native-frame");
    if (native) {
      height +=
        px(native.querySelector(".native-title")) +
        px(native.querySelector(".notepad-menubar")) +
        px(native.querySelector(".notepad-status:not([hidden])")) +
        px(native.querySelector(".console-command")) +
        padding(native.querySelector(".terminal-screen")) +
        padding(native.querySelector(".minimal-native")) +
        px(native.querySelector(".minimal-native-demo")) +
        px(native.querySelector(".console-minimal-log"));
    } else {
      height +=
        px(app.querySelector(".shell>header")) +
        px(app.querySelector(".shell>nav")) +
        px(app.querySelector("#banner")) +
        px(app.querySelector(".shell>footer"));
    }
    const popup = content.querySelector(".native-menu[open] .native-popup");
    if (popup)
      height = Math.max(height, popup.getBoundingClientRect().bottom + 6);
    height = Math.max(native ? 100 : 28, Math.min(410, Math.ceil(height)));
    if (
      tab === "watch" &&
      state.settings.rowDisplay === "minimal" &&
      Math.abs(size.height / scale - height) > 1
    )
      await invoke("fit_minimal_height", { height });
  } catch (e) {
    showError(e);
  } finally {
    fittingMinimal = false;
    if (fitAgain) {
      fitAgain = false;
      queueMicrotask(() => void fitMinimal());
    }
  }
}
let minimalObserver: ResizeObserver | null = null;
function observeMinimal() {
  minimalObserver?.disconnect();
  const list = content.querySelector(".minimal-list");
  if (!list) return;
  minimalObserver = new ResizeObserver(() => void fitMinimal());
  minimalObserver.observe(list);
}
function openStock(code: string) {
  dailySymbol = code;
  dailyData = undefined;
  tab = "daily";
  render();
}
/** Live slider preview while the opacity setting is being dragged; the
 * persisted value only lands on the slider's change event. */
let opacityPreview: number | null = null;
function apply() {
  document.documentElement.style.setProperty(
    "--bg-alpha",
    String(opacityPreview ?? shellOpacity(state.settings.windowOpacity)),
  );
  document.body.dataset.skin =
    tab === "watch" && ["terminal", "txt"].includes(state.settings.appearance)
      ? "native"
      : "small";
  const windowTitle =
    document.body.dataset.skin === "native"
      ? state.settings.appearance === "txt"
        ? isMac
          ? "工作紀錄.txt"
          : "工作紀錄.txt - 記事本"
        : isMac
          ? "work — sync — zsh"
          : "PowerShell"
      : "Maxi stocks";
  if (desktop && windowTitle !== lastWindowTitle) {
    lastWindowTitle = windowTitle;
    void getCurrentWindow().setTitle(windowTitle).catch(showError);
  }
  const toggle = document.querySelector<HTMLButtonElement>("#row-display")!;
  toggle.hidden =
    tab !== "watch" || !["small", "gray"].includes(state.settings.appearance);
  toggle.textContent =
    state.settings.rowDisplay === "trend"
      ? !state.marketOpen && !state.demo
        ? "近日線"
        : "當日線"
      : state.settings.rowDisplay === "minimal"
        ? "極簡"
        : "數字";
  toggle.setAttribute("aria-description", "目前：" + toggle.textContent);
  document.body.dataset.page = tab;
  document.body.dataset.appearance = state.settings.appearance;
  document.body.dataset.density = state.settings.rowDisplay ?? "numbers";
  const pinned = state.settings.alwaysOnTop !== false;
  const pin = document.querySelector<HTMLElement>("#pin");
  if (pin) {
    pin.setAttribute("aria-pressed", String(pinned));
    pin.title = pinned ? "已置頂：點一下取消置頂" : "未置頂：點一下保持置頂";
  }
  document.querySelector("#rule-count")!.textContent = String(
    state.rules.filter((r) => r.enabled).length,
  );
  document.querySelector("#banner")!.innerHTML = state.demo
    ? '<span class="demo-label">示範資料</span><span>合成行情 · 不代表真實市場</span>'
    : "";
  const missing = state.watchlist.filter((c) => !state.quotes[c]).length;
  const old = state.watchlist.some((c) => stale(state.quotes[c], state.now));
  document.querySelector("#health")!.textContent = state.settings.paused
    ? "Ⅱ 已暫停監看"
    : state.error
      ? "○ 連線異常 · 正在退避重試"
      : missing
        ? "○ 部分行情尚未取得"
        : state.demo
          ? "● 示範模式"
          : !state.marketOpen
            ? "○ 非一般交易時段 · 僅供查看"
            : old
              ? "○ 來源資料已過期"
              : "● 本機監看中";
  document.querySelector<HTMLElement>("#health")!.title =
    state.error ??
    "成交價規則只依新鮮成交資料；買賣中間價規則依委託價（非成交價）。首次觀測只建立基準";
  const sourceLabel = Object.values(state.quotes).some(
    (q) => q.source === "Shioaji",
  )
    ? "Shioaji"
    : "TWSE MIS";
  document.querySelector("#source")!.textContent =
    (state.demo
      ? "DEMO"
      : state.marketOpen && oldestDataAt(state)
        ? `${sourceLabel} · 資料 ${time(oldestDataAt(state)!)}`
        : `${sourceLabel} · 取得 ${time(state.lastFetch)}`) +
    (state.sourceNote ? ` · ${state.sourceNote}` : "");
  document.querySelector<HTMLElement>("#source")!.title =
    state.sourceNote ??
    (state.marketOpen && oldestDataAt(state)
      ? `各檔資料時間不同，顯示最舊的一筆（最新取得於 ${time(state.lastFetch)}）。證交所個股約每 1 至 2 分鐘更新一次；Shioaji 為即時逐筆。`
      : "");
  // Header combined total (≈ TWD), updated with the footer: the
  // per-currency portfolio totals with USD converted at the internal
  // TWD=X quote the poll fetches while USD positions exist. Hidden
  // whenever the combined figure would be a guess (combinedTotals null).
  const assetsEl = document.querySelector<HTMLElement>("#header-assets");
  if (assetsEl) {
    const totals = portfolioSummary(
      state.positions ?? [],
      positionQuotes(),
    ).totals;
    const combined = combinedTotals(
      totals,
      state.quotes["TWD=X"]?.price ?? null,
    );
    if (combined) {
      const sign =
        combined.dayPnl === 0 ? "flat" : combined.dayPnl > 0 ? "up" : "down";
      assetsEl.innerHTML = `總資產 ≈ ${formatMoney(combined.total, "TWD")} · 今日 <span class="pos-num ${sign}">${formatMoney(combined.dayPnl, "TWD", true)}${combined.dayPnlPercent != null ? `（${change(combined.dayPnlPercent)}）` : ""}</span>`;
      assetsEl.title =
        combined.rate != null
          ? `估算 · 美元匯率 ${number(combined.rate)} · 總資產＝台幣市值＋美元市值×匯率`
          : "總資產＝台幣持股市值（無美元持股，不需換算）";
      // The same complete total feeds the monthly record: the first
      // computable total of a month is that month's snapshot.
      maybeRecordMonthlySnapshot(
        combined.total,
        totals.find((t) => t.currency === "TWD")?.marketValue ?? 0,
        totals.find((t) => t.currency === "USD")?.marketValue ?? null,
        combined.rate,
      );
    } else {
      assetsEl.textContent = "";
      assetsEl.title = "";
    }
  }
  if (!state.demo && !state.marketOpen) {
    const date = latestCloseDate(state);
    const label = date
      ? (date.startsWith(
          new Date(state.now)
            .toLocaleDateString("sv-SE", { timeZone: "Asia/Taipei" })
            .slice(0, 4),
        )
          ? date.slice(5).replace("-", "/")
          : date.replaceAll("-", "/")) +
        (state.watchlist.some((c) => closeDay(state, c)?.kind === "final")
          ? " 收盤（待官方日資料）"
          : " 收盤")
      : "取得收盤資料中";
    document.querySelector("#health")!.textContent =
      `${state.settings.paused ? "Ⅱ 已暫停" : state.marketClosedReason ? `○ 休市（${state.marketClosedReason}）` : "○ 休市"} · ${label}`;
    document.querySelector("#source")!.textContent = "TWSE / TPEx · 日資料";
  }
  if (
    state.settings.rowDisplay === "trend" &&
    ["small", "gray"].includes(state.settings.appearance)
  )
    document.querySelector("#source")!.textContent = state.demo
      ? "DEMO · 分時示意"
      : !state.marketOpen
        ? "近日收盤走勢"
        : "分鐘線 · 可能延遲";
}
/**
 * Records this month's portfolio snapshot from the header total, once:
 * only when no snapshot exists for the current Taipei month yet and this
 * session has not already tried. The call goes through request() (not
 * mutate()) because apply() runs inside render — the fresh snapshot
 * arrives with the next poll's state, and a failure is deliberately
 * silent: the monthly tab is a record, never a reason to nag. The core
 * dedupes by month, so a retry after a restart is harmless.
 */
function maybeRecordMonthlySnapshot(
  totalTwd: number,
  twdValue: number,
  usdValue: number | null,
  rate: number | null,
) {
  const today = new Date(state.now).toLocaleDateString("sv-SE", {
    timeZone: "Asia/Taipei",
  });
  const month = today.slice(0, 7);
  if (
    (state.monthlySnapshots ?? []).some((s) => s.month === month) ||
    monthlyRecordTried.has(month)
  )
    return;
  monthlyRecordTried.add(month);
  void request("record_monthly_snapshot", {
    month,
    totalTwd,
    twdValue,
    usdValue,
    rate,
    recordedOn: today,
  }).catch(() => undefined);
}
function rows() {
  const commonDate = latestCloseDate(state);
  const hideName = Boolean(state.settings.hideNames);
  // Numbers mode stacks the change in two lines and adds a day-range cell;
  // the trend cell already spends the row's width on the line itself, so
  // trend rows keep the single-line percent only.
  const numbersMode = state.settings.rowDisplay !== "trend";
  const showPts = numbersMode && state.settings.showChangePoints !== false;
  return (
    state.watchlist
      .map((c) => {
        const q = state.quotes[c];
        // Yahoo symbols render from their live quote around the clock; the
        // Taiwan close-day view (official/final close) is a TWSE-only idea.
        const closed = !state.marketOpen && !state.demo && !isYahooSymbol(c);
        const d = closed ? closeDay(state, c) : undefined;
        const live = livePrice(q);
        const pct = closed ? d?.changePercent : live.pct;
        const pts = closed
          ? (d?.changePoints ?? null)
          : live.price != null && q?.previous != null
            ? live.price - q.previous
            : null;
        const stamp = closed
          ? (d?.date ?? "尚未取得正式收盤")
          : time(q?.sourceAt);
        // Session range for the day-range cell: the close day's high/low
        // when closed, the live quote's otherwise (every source fills
        // them: MIS h/l, the sidecar, Yahoo's regularMarketDayHigh/Low).
        const dayHi = closed ? (d?.high ?? null) : (q?.dayHigh ?? null);
        const dayLo = closed ? (d?.low ?? null) : (q?.dayLow ?? null);
        const name =
          q?.name ||
          state.names?.[c] ||
          (c === "t00" ? "加權指數" : c === "o00" ? "櫃買指數" : "");
        const sign = (pct ?? 0) > 0 ? "up" : (pct ?? 0) < 0 ? "down" : "flat";
        const trend =
          state.settings.rowDisplay === "trend" ? trendCell(state, c) : null;
        return `<button class="quote ${sign}${numbersMode ? " two-line" : ""}${!closed && stale(q, state.now) ? " stale" : !closed && aging(q, state.now) ? " aging" : ""}" data-quote="${esc(c)}" title="${esc(name && name !== c ? `${name}（${displaySymbol(c)}）` : displaySymbol(c))} · ${closed ? "正式收盤" : "來源時間"} ${esc(stamp)} · ${!closed && live.basis === "book" ? BOOK_NOTE : "價格"} ${number(closed ? d?.close : live.price)}${trend ? " · " + esc(trend.title) : ""} · 點擊查看行情"><span class="stock-name">${hideName || state.settings.showCodes || !name ? `<span class="stock-code">${esc(displaySymbol(c))}</span>` : ""}${hideName ? "" : `<strong>${esc(name)}</strong>`}</span>${trend ? `<span class="trend-slot">${trend.html}</span>` : `<span class="price${!closed && live.basis === "book" ? " book" : ""}">${number(closed ? d?.close : live.price)}</span>`}<span class="change">${showPts ? `<span class="change-points">${pts != null ? points(pts) : ""}</span>` : ""}<span class="change-pct">${change(pct)}</span></span>${numbersMode ? `<span class="day-range"><span class="dr-line"><i>低</i>${number(dayLo)}</span><span class="dr-line"><i>高</i>${number(dayHi)}</span></span>` : ""}${closed && d && d.date !== commonDate ? `<span class="as-of">${d.date.slice(5).replace("-", "/")}</span>` : ""}</button>`;
      })
      .join("") || '<div class="empty">加入第一檔自選，讓重要變化來找你。</div>'
  );
}
const viewGroups = () =>
  state.groups?.length
    ? state.groups
    : [{ name: "自選1", codes: state.watchlist }];
const viewActive = () =>
  Math.max(0, Math.min(state.activeGroup ?? 0, viewGroups().length - 1));
function groupTabs() {
  const groups = viewGroups(),
    active = viewActive();
  return `<div class="group-tabs" role="tablist" aria-label="自選分組">${groups.map((g, i) => `<button type="button" role="tab" aria-selected="${i === active}" class="group-tab${i === active ? " active" : ""}" data-group="${i}">${esc(g.name)}</button>`).join("")}${groups.length < 10 ? '<button type="button" class="group-tab add" data-group-add title="新增分組" aria-label="新增分組">＋</button>' : ""}</div>`;
}
/** The position whose lot detail is open on the portfolio page, if any. */
let expandedPosition: string | null = null;
/** Market tab chosen on the portfolio page; null until the user picks one,
 * when the page defaults to Taiwan if it holds anything, else the US. */
let positionsMarket: "TWD" | "USD" | "monthly" | null = null;
/** Months this session has already tried to record a snapshot for, so the
 * per-second apply() attempts the core call at most once per month per
 * session even when it fails; the core's first-write-wins idempotency
 * covers restarts. */
const monthlyRecordTried = new Set<string>();
/** Column sort chosen on the portfolio page; null until a header is
 * clicked, when the table keeps its recorded order and shows no arrow. */
let positionsSort: PositionSort | null = null;
/** Outcome line of the last positions import, kept across re-renders. */
let positionsMessage = "";
/** Outcome line of the last data export/import on the settings page. */
let dataMessage = "";
/** Outcome line of the last Telegram test send on the settings page. */
let telegramMessage = "";
/** Which locally stored Shioaji keys exist (booleans only, fetched at
 * startup like the program path); the keys themselves never reach here. */
let shioajiKeys = { apiKeySet: false, secretKeySet: false };
/** Outcome line of the last sidecar key/start action on the settings page. */
let sidecarMessage = "";
/** Outcome line of the last manual backup on the settings page. */
let backupMessage = "";
/** Prices the portfolio page was last rendered with; the poll compares
 * against it so the page follows live quotes without a full tab re-render
 * policy of its own. */
let positionsSig = "";
/** Last time the positions page asked the core to fetch minute samples for
 * held Taiwan codes; the ask is throttled to the core's own 60-second
 * intraday freshness window so sitting on the page never hammers Yahoo. */
let lastIntradayAsk = 0;
/** Aggregated points behind the portfolio chart currently on screen, kept
 * for the hover readout, with the X domain the hover maps fractions onto
 * (the session hours for 1D, the points' own span for the date ranges);
 * the readout's resting text beside it. */
let pfHover: {
  points: { at: number; value: number }[];
  baseline: number | null;
  domain: [number, number];
  mode: "time" | "date";
} | null = null;
let pfReadoutDefault = "";
/** Range tab of the portfolio chart; 1D is the intraday aggregation, the
 * rest ride the core's portfolioHistory. Kept across re-renders. */
const PF_RANGES = ["1D", "5D", "1M", "6M", "YTD", "1Y", "5Y", "MAX"] as const;
type PfRange = (typeof PF_RANGES)[number];
let pfRange: PfRange = "1D";
const hm = (at: number) =>
  new Date(at).toLocaleTimeString("zh-TW", {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
    timeZone: "Asia/Taipei",
  });
const ymd = (at: number) =>
  new Date(at)
    .toLocaleDateString("sv-SE", { timeZone: "Asia/Taipei" })
    .replaceAll("-", "/");
/**
 * Asks the core to queue minute-sample fetches for every held Taiwan code —
 * the only trigger for the portfolio day chart's data. Display-only: the
 * samples ride the existing intraday queue (never the alert path), the
 * core's 60-second freshness check dedupes repeat asks, and each response
 * is discarded here — fresh series arrive with the next poll's snapshot.
 */
function ensurePositionIntraday() {
  const at = Date.now();
  if (at - lastIntradayAsk < 60_000) return;
  const codes = (state.positions ?? [])
    .map((p) => p.code)
    .filter((c) => !isYahooSymbol(c));
  if (!codes.length) return;
  lastIntradayAsk = at;
  for (const c of codes)
    void request("get_intraday_history", { symbol: c }).catch(() => {});
}
/** Last time the positions page asked the core to fetch daily closes for
 * the held Taiwan codes (the range chart's history); throttled like the
 * intraday ask — the core's own 30-minute freshness check does the rest. */
let lastHistoryAsk = 0;
/**
 * Asks the core to queue Yahoo daily-close fetches for every held Taiwan
 * code so the range tabs have aggregates ready. Display-only like the
 * intraday ask: the response is discarded, results arrive on the snapshot.
 */
function ensurePortfolioHistory() {
  const at = Date.now();
  if (at - lastHistoryAsk < 60_000) return;
  if (!(state.positions ?? []).some((p) => !isYahooSymbol(p.code))) return;
  lastHistoryAsk = at;
  void request("ensure_portfolio_history", {}).catch(() => {});
}
/**
 * Price inputs for every held code, from the same sources the watch rows use:
 * live quotes while the market runs; after the Taiwan close, Taiwan codes use
 * the official close day (its previous close is derived from the close's own
 * change, the same numbers the rows show). Yahoo codes always use the live
 * quote, around the clock.
 */
function positionQuotes(): Record<string, PositionQuote> {
  const out: Record<string, PositionQuote> = {};
  for (const p of state.positions ?? []) {
    const c = p.code;
    const closed = !state.marketOpen && !state.demo && !isYahooSymbol(c);
    const d = closed ? closeDay(state, c) : undefined;
    if (d) {
      const previous =
        d.changePoints != null
          ? d.close - d.changePoints
          : d.changePercent != null && d.changePercent > -100
            ? d.close / (1 + d.changePercent / 100)
            : null;
      out[c] = { price: d.close, previous };
    } else {
      const q = state.quotes[c];
      out[c] = { price: livePrice(q).price, previous: q?.previous ?? null };
    }
  }
  return out;
}
function positionsSignature() {
  const quotes = positionQuotes();
  return JSON.stringify([
    state.marketOpen,
    // The range chart follows its tab and the core's history aggregates: a
    // finished fetch (more points, pending cleared) re-renders the page.
    pfRange,
    ...PF_RANGES.slice(1).map((r) => {
      const e = state.portfolioHistory?.[r];
      return e ? `${e.points.length}:${e.pending ? 1 : 0}` : "-";
    }),
    (state.positions ?? []).map((p) => [
      p.code,
      quotes[p.code]?.price ?? null,
      quotes[p.code]?.previous ?? null,
      // The day chart follows the minute samples too: a refetched series
      // (new fetchedAt / more points) re-renders the page like a price move.
      state.intraday[p.code]?.series?.fetchedAt ?? 0,
      state.intraday[p.code]?.series?.points.length ?? 0,
    ]),
  ]);
}
/**
 * The portfolio chart card (TWD book only), with the range tabs above the
 * chart. 1D is the day chart: the aggregate minute-value line from
 * model.portfolioIntradaySeries over the intraday samples the snapshot
 * carries, drawn by inline-trend.portfolioChart; its headline figures
 * deliberately repeat the TWD summary card (live/close pricing), while the
 * line itself aggregates the Yahoo minute samples — the two can differ by a
 * sample's delay, and the card says samples may lag. The other ranges come
 * from the core's portfolioHistory backcast (see pfHistorySection).
 */
function portfolioChartSection(
  summary: ReturnType<typeof portfolioSummary>,
  quotes: Record<string, PositionQuote>,
) {
  pfHover = null;
  const twd = summary.totals.find((t) => t.currency === "TWD");
  if (!twd) return "";
  const tabs = `<div class="pf-ranges" role="tablist" aria-label="走勢範圍">${PF_RANGES.map((r) => `<button type="button" role="tab" aria-selected="${r === pfRange}" class="pf-range${r === pfRange ? " active" : ""}" data-pf-range="${r}">${r}</button>`).join("")}</div>`;
  if (pfRange !== "1D") return pfHistorySection(twd, tabs);
  const held = (state.positions ?? []).filter((p) => !isYahooSymbol(p.code));
  const samples: Record<string, { at: number; price: number | null }[]> = {};
  const previous: Record<string, number | null> = {};
  let uncovered = 0;
  for (const p of held) {
    const pts = state.intraday[p.code]?.series?.points ?? null;
    samples[p.code] = pts ?? [];
    previous[p.code] = quotes[p.code]?.previous ?? null;
    if (!pts?.some((pt) => pt.price != null) && previous[p.code] == null)
      uncovered++;
  }
  const agg = portfolioIntradaySeries(state.positions ?? [], samples, previous);
  const sign =
    twd.dayPnl == null || twd.dayPnl === 0
      ? "flat"
      : twd.dayPnl > 0
        ? "up"
        : "down";
  const head = `<div class="pf-head"><h2>組合總值走勢（台幣）</h2><span class="pf-now pos-num ${sign}">${formatMoney(twd.marketValue, "TWD")}${twd.dayPnl != null ? `　${formatMoney(twd.dayPnl, "TWD", true)}${twd.dayPnlPercent != null ? `（${change(twd.dayPnlPercent)}）` : ""}` : ""}</span></div>`;
  if (agg.points.length < 2) {
    pfReadoutDefault =
      agg.previousTotal != null
        ? `昨收 ${formatMoney(agg.previousTotal, "TWD")} · 尚無今日走勢`
        : "尚無今日走勢";
    return `<div class="pf-card">${head}${tabs}<p class="pf-empty">${pfReadoutDefault}（分時資料取得後自動更新）</p></div>`;
  }
  const firstPt = agg.points[0]!.at;
  const pfDayStart =
    Math.floor((firstPt + 28800000) / 86400000) * 86400000 - 28800000;
  pfHover = {
    points: agg.points,
    baseline: agg.previousTotal,
    domain: [pfDayStart + 9 * 3600000, pfDayStart + 13.5 * 3600000],
    mode: "time",
  };
  const meta = [
    agg.previousTotal != null
      ? `昨收 ${formatMoney(agg.previousTotal, "TWD")}`
      : "",
    `高 ${formatMoney(agg.high, "TWD")}`,
    `低 ${formatMoney(agg.low, "TWD")}`,
    `${hm(agg.firstAt!)}–${hm(agg.lastAt!)}`,
    uncovered ? `${uncovered} 檔尚無資料，未計入` : "",
    "分時樣本可能延遲",
  ]
    .filter(Boolean)
    .join(" · ");
  pfReadoutDefault = meta;
  const label = `台幣持股組合總值日內走勢，目前 ${formatMoney(agg.lastValue, "TWD")}${agg.previousTotal != null ? `，昨收 ${formatMoney(agg.previousTotal, "TWD")}` : ""}`;
  return `<div class="pf-card">${head}${tabs}<div class="pf-chart-wrap">${portfolioChart(agg.points, agg.previousTotal, sign, label, "time")}</div><p class="pf-readout">${esc(meta)}</p></div>`;
}
/**
 * The portfolio chart card on a multi-day range: the core's backcast
 * aggregate for the chosen range (see history.rs — current holdings over
 * Yahoo daily closes). The headline switches from the day P/L to the range
 * change against the range's first point, the chart's dashed baseline.
 */
function pfHistorySection(
  twd: { marketValue: number | null },
  tabs: string,
) {
  const entry = state.portfolioHistory?.[pfRange];
  const head = (right: string, signClass = "flat") =>
    `<div class="pf-head"><h2>組合總值走勢（台幣）</h2><span class="pf-now pos-num ${signClass}">${right}</span></div>`;
  if (!entry || entry.points.length < 2) {
    const msg =
      !entry || entry.pending ? "歷史資料取得中…" : "區間資料不足";
    return `<div class="pf-card">${head(twd.marketValue != null ? formatMoney(twd.marketValue, "TWD") : "")}${tabs}<p class="pf-empty">${msg}</p></div>`;
  }
  const first = entry.points[0]!,
    last = entry.points.at(-1)!;
  const base = entry.baseline;
  const delta = base != null ? last.value - base : null;
  const pct =
    base != null && base !== 0 && delta != null ? (delta / base) * 100 : null;
  const signClass =
    delta == null || delta === 0 ? "flat" : delta > 0 ? "up" : "down";
  const right = `${formatMoney(last.value, "TWD")}　區間 ${delta != null ? formatMoney(delta, "TWD", true) : "—"}${pct != null ? `（${change(pct)}）` : ""}`;
  pfHover = {
    points: entry.points,
    baseline: base,
    domain: [first.at, last.at],
    mode: "date",
  };
  const meta = [
    `${ymd(first.at)}–${ymd(last.at)}`,
    entry.uncovered ? `${entry.uncovered} 檔無歷史資料，未計入` : "",
    "以目前持股回推 · 日線資料 Yahoo（僅供走勢參考）",
  ]
    .filter(Boolean)
    .join(" · ");
  pfReadoutDefault = meta;
  const label = `台幣持股組合總值 ${pfRange} 走勢，目前 ${formatMoney(last.value, "TWD")}${base != null ? `，區間起點 ${formatMoney(base, "TWD")}` : ""}`;
  return `<div class="pf-card">${head(right, signClass)}${tabs}<div class="pf-chart-wrap">${portfolioChart(entry.points, base, signClass, label, "date")}</div><p class="pf-readout">${esc(meta)}</p></div>`;
}
function positionsPage() {
  const positions = state.positions ?? [];
  const quotes = positionQuotes();
  const summary = portfolioSummary(positions, quotes);
  const today = new Date(state.now).toLocaleDateString("sv-SE", {
    timeZone: "Asia/Taipei",
  });
  const sign = (n: number | null | undefined) =>
    n == null || n === 0 ? "flat" : n > 0 ? "up" : "down";
  const cards = summary.totals
    .map(
      (t) => `<div class="pos-card"><h2>${t.currency === "TWD" ? "新台幣" : "美元"} 持股</h2><div class="pos-line"><span>總市值</span><span class="pos-total">${t.marketValue == null ? "—" : formatMoney(t.marketValue, t.currency)}</span></div><div class="pos-line"><span>今日損益</span><span class="pos-num ${sign(t.dayPnl)}">${t.dayPnl == null ? "—" : formatMoney(t.dayPnl, t.currency, true)}${t.dayPnlPercent != null ? `（${change(t.dayPnlPercent)}）` : ""}</span></div><div class="pos-line"><span>總損益</span><span class="pos-num ${sign(t.totalPnl)}">${t.totalPnl == null ? "—" : formatMoney(t.totalPnl, t.currency, true)}${t.totalPnlPercent != null ? `（${change(t.totalPnlPercent)}）` : ""}</span></div></div>`,
    )
    .join("");
  const byCode = new Map(summary.positions.map((r) => [r.code, r]));
  const rowFor = (p: (typeof positions)[number]) => {
    const r = byCode.get(p.code)!;
    const name = state.quotes[p.code]?.name || state.names?.[p.code] || "";
    const expanded = expandedPosition === p.code;
    const lots = expanded
      ? `<div class="pos-lots">${p.lots
          .map(
            (l, i) =>
              `<div class="pos-lot"><span>${esc(l.date || "未記錄日期")}</span><span>購入價 ${number(l.price)}</span><span>${number(l.shares)} 股</span><button type="button" data-del-lot="${esc(p.code)}:${i}">刪除</button></div>`,
          )
          .join("")}<div class="pos-lot-actions"><button type="button" class="secondary" data-add-lot="${esc(p.code)}">＋ 加一筆</button><button type="button" class="secondary" data-del-position="${esc(p.code)}">刪除整檔持股</button></div></div>`
      : "";
    return `<div class="pos-row-wrap"><div class="pos-row" data-expand="${esc(p.code)}" title="點擊展開購入批次"><span class="pos-name"><span class="stock-code">${esc(displaySymbol(p.code))}</span><strong>${esc(name)}</strong></span><span class="pos-cell">${number(r.shares)}</span><span class="pos-cell">${number(r.avgPrice)}</span><span class="pos-cell">${number(r.price)}</span><span class="pos-cell">${formatMoney(r.marketValue, r.currency)}</span><span class="pos-cell pos-pnl pos-num ${sign(r.dayPnl)}"><span>${r.dayPnl == null ? "—" : formatMoney(r.dayPnl, r.currency, true)}</span><span class="pos-pct">${r.dayPnlPercent == null ? "—" : `(${change(r.dayPnlPercent)})`}</span></span><span class="pos-cell pos-pnl pos-num ${sign(r.totalPnl)}"><span>${r.totalPnl == null ? "—" : formatMoney(r.totalPnl, r.currency, true)}</span><span class="pos-pct">${r.totalPnlPercent == null ? "—" : `(${change(r.totalPnlPercent)})`}</span></span></div>${lots}</div>`;
  };
  // Positions are split by market behind tabs, like the watchlist groups:
  // Taiwan (TWD) and US (USD) never stack on one page. Both tabs always
  // show; the panel carries the chosen market's subtotal and table, or a
  // plain empty note when that market holds nothing yet. A position's
  // market comes from its summary currency; rows inside a market keep the
  // recorded order until a column header sorts them (positionsSort).
  const hasTwd = positions.some((p) => byCode.get(p.code)!.currency === "TWD");
  const market = positionsMarket ?? (hasTwd ? "TWD" : "USD");
  const mine = positions.filter((p) => byCode.get(p.code)!.currency === market);
  const marketTotal = summary.totals.find((t) => t.currency === market);
  const ordered = sortedPositions(mine, summary.positions, positionsSort);
  const sortHead = (key: PositionSortKey, label: string) => {
    const active = positionsSort?.key === key;
    const arrow = active ? (positionsSort!.dir === "desc" ? " ▼" : " ▲") : "";
    return `<button type="button" class="pos-sort${active ? " active" : ""}" data-pos-sort="${key}" title="依此欄排序">${label}${arrow}</button>`;
  };
  const tabs = `<div class="group-tabs pos-tabs" role="tablist" aria-label="持股市場">${(
    [
      ["TWD", "台股"],
      ["USD", "美股"],
      ["monthly", "月度"],
    ] as const
  )
    .map(
      ([m, label]) =>
        `<button type="button" role="tab" aria-selected="${m === market}" class="group-tab${m === market ? " active" : ""}" data-pos-market="${m}">${label}</button>`,
    )
    .join("")}</div>`;
  // The monthly tab replaces the holdings table with the snapshot record:
  // one row per month the app ran with complete quotes, newest first, each
  // compared against the previous recorded month. Rows delete individually
  // (a mis-captured month should not be permanent).
  const monthlyPanel = () => {
    const rows = monthlyRows(state.monthlySnapshots);
    const rule =
      '<p class="note">每月第一次開啟 app 且報價齊全時，自動記錄當下總資產（約當台幣）</p>';
    if (!rows.length)
      return `<div class="pos-section"><div class="empty">尚無月度紀錄</div>${rule}</div>`;
    const body = rows
      .map(
        (r) => `<div class="monthly-row"><span class="monthly-month">${esc(r.month)}</span><span class="pos-cell">${formatMoney(r.totalTwd, "TWD")}</span><span class="pos-cell pos-num ${sign(r.change)}">${r.change == null ? "—" : `${formatMoney(r.change, "TWD", true)}${r.changePct != null ? `（${change(r.changePct)}）` : ""}`}</span><span class="pos-cell monthly-date">${esc(r.recordedOn)}</span><button type="button" class="monthly-del" data-del-month="${esc(r.month)}" title="刪除這筆月度紀錄" aria-label="刪除 ${esc(r.month)} 的月度紀錄">✕</button></div>`,
      )
      .join("");
    return `<div class="pos-section"><div class="monthly-table"><div class="monthly-head"><span>月份</span><span>總資產（約當台幣）</span><span>月變化</span><span>記錄日期</span><span></span></div>${body}</div>${rule}</div>`;
  };
  const panel =
    market === "monthly"
      ? monthlyPanel()
      : mine.length
        ? `<div class="pos-section"><div class="pos-section-head"><h2>${market === "TWD" ? "台股" : "美股"}</h2><span class="pos-section-total">${formatMoney(marketTotal?.marketValue ?? null, market)}</span></div><div class="pos-table"><div class="pos-head">${sortHead("code", "代號 · 名稱")}${sortHead("shares", "股數")}<span>均價</span><span>現價</span>${sortHead("marketValue", "市值")}${sortHead("dayPnl", "今日損益")}${sortHead("totalPnl", "總損益")}</div>${ordered.map(rowFor).join("")}</div></div>`
        : '<div class="empty">尚無持股紀錄</div>';
  // Entry sits at the bottom of the page, below the holdings themselves:
  // the summary, chart and table are what the page is opened for, the form
  // and paste-import only when something new is being recorded.
  const entry = `<form id="lot-form"><div class="form-grid"><label>代號<input name="code" id="lot-code" required placeholder="2330 或 VOO"></label><label>購入日期<input name="date" type="date" value="${today}"></label><label>購入價（每股）<input name="price" type="number" step="any" required placeholder="例如 2395"></label><label>股數<input name="shares" type="number" step="any" required placeholder="例如 64，可含碎股"></label></div><button class="primary" type="submit">新增一筆</button></form><div class="pos-import"><h2>貼上匯入</h2><p class="note">一行一筆：代號, 日期, 購入價, 股數；日期可省略（代號, 購入價, 股數）。分隔可用逗號、Tab 或空白，價格不要加千分位逗號。</p><textarea id="import-text" rows="4" placeholder="2330, 2026-11-10, 2395, 64"></textarea><button id="import-btn" class="secondary" type="button">匯入</button>${positionsMessage ? `<p class="note">${esc(positionsMessage)}</p>` : ""}</div>`;
  // The tab strip (and with it the monthly record) stays reachable while
  // snapshots exist even if every position was later deleted.
  const listBlock =
    positions.length || (state.monthlySnapshots ?? []).length
      ? `<div class="positions-list">${tabs}${panel}</div>`
      : '<div class="empty">還沒有持股紀錄。用下方表單新增一筆，或用「貼上匯入」一次貼上多筆，組合市值與每日損益就會自動計算。</div>';
  // The summary cards and day chart describe the live holdings; the
  // monthly tab is a historical record, so both hide there.
  return `<div class="pane"><h1>你的投資組合</h1><p class="intro">記錄每檔的購入批次，市值與損益用目前報價自動計算；台股收盤後以官方收盤計。台幣與美元分開彙總，不相加。</p>${positions.length && market !== "monthly" ? `<div class="pos-summary">${cards}</div>${portfolioChartSection(summary, quotes)}` : ""}${listBlock}${entry}</div>`;
}
function watch() {
  const a = state.settings.appearance;
  const last = state.events[0];
  const closed = !state.marketOpen && !state.demo;
  if (a === "terminal" || a === "txt") return disguiseMarkup(state);
  if (state.settings.rowDisplay === "minimal")
    return `<div class="minimal-bar"><span class="minimal-grip" data-tauri-drag-region><i data-tauri-drag-region></i></span>${state.demo ? '<span class="minimal-demo-tag">示範</span>' : ""}${minimalMarkup(state)}<span class="minimal-bar-actions"><button id="minimal-cycle" aria-label="切換顯示內容" title="回到數字">≡</button><button id="minimal-stealth" class="stealth-btn" aria-label="隱藏全部視窗" title="完全隱藏（含通知），監看持續。叫回：${privacyShortcutLabel}"><svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"><path d="M8 3.5C4.6 3.5 2 6 1 8c1 2 3.6 4.5 7 4.5s6-2.5 7-4.5c-1-2-3.6-4.5-7-4.5zm0 1.8a2.7 2.7 0 1 1 0 5.4 2.7 2.7 0 0 1 0-5.4z" fill="currentColor"/><path d="M2.3 2.3 13.7 13.7" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg></button><button id="minimal-pin" class="pin-btn" aria-pressed="${state.settings.alwaysOnTop !== false}" aria-label="視窗保持置頂" title="${state.settings.alwaysOnTop !== false ? "已置頂：點一下取消置頂" : "未置頂：點一下保持置頂"}"><svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><path d="M9.8 1.2 14.8 6.2 13.4 7.6 12.3 7.2 9.4 10.1 9.9 12 8.5 13.4 5.6 10.5 2.2 13.9 1.5 13.2 4.9 9.8 2 6.9 3.4 5.5 5.3 6 8.2 3.1 7.8 2z" fill="currentColor"/></svg></button><button id="minimal-settings" aria-label="極簡顯示設定" title="顯示設定">⋯</button></span></div>`;
  return `${groupTabs()}<div class="quotes">${rows()}</div>${last ? `<div class="latest"><span class="latest-dot"></span><span>${esc(last.text)}</span></div>` : ""}`;
}
function render() {
  hideDetail();
  minimalObserver?.disconnect();
  document.body.dataset.page = tab;
  apply();
  content.onkeydown = null;
  // Same-view re-renders (poll ticks, background refreshes, setting toggles)
  // keep the scroll place; only a view switch — or an explicit reset from a
  // navigation action — goes back to the top.
  const savedScrollTop = shouldKeepScroll(renderedTab, tab, scrollResetPending)
    ? content.scrollTop
    : 0;
  scrollResetPending = false;
  renderedTab = tab;
  document
    .querySelectorAll("[data-tab]")
    .forEach((e) =>
      e.classList.toggle("active", (e as HTMLElement).dataset.tab === tab),
    );
  if (tab === "watch") content.innerHTML = watch();
  if (tab === "positions") {
    content.innerHTML = positionsPage();
    positionsSig = positionsSignature();
    ensurePositionIntraday();
    ensurePortfolioHistory();
  }
  if (tab === "daily") dailyPage();
  if (tab === "settings")
    content.innerHTML = `<div class="pane"><h1>讓它融入你的桌面</h1><p class="intro">同一份自選與提醒，換一種工作時的樣子。</p><div class="appearance-grid">${[
      ["small", "small", "品牌配色 · 紅漲綠跌"],
      ["gray", "small 灰階", "熟悉的元件，更安靜"],
      [
        "terminal",
        "terminal",
        isMac ? "Mac 終端機 · zsh 外觀" : "Windows Terminal · 工程工作",
      ],
      [
        "txt",
        "txt",
        isMac ? "文字編輯 TextEdit · 純文字" : "Windows 記事本 · 日常工作",
      ],
    ]
      .map(
        ([id, name, sub]) =>
          `<button class="appearance ${id === state.settings.appearance ? "selected" : ""}" data-look="${id}"><span class="look-sample ${id}">${id === "terminal" ? "&gt; monitor --local" : id === "txt" ? "追蹤筆記.txt" : "●　2330　+0.8%"}</span><strong>${name}</strong><small>${sub}</small></button>`,
      )
      .join(
        "",
      )}</div><div class="setting-row"><label>顯示內容<select id="display-mode"><option value="numbers" ${state.settings.rowDisplay === "numbers" || !state.settings.rowDisplay ? "selected" : ""}>純數字</option><option value="trend" ${state.settings.rowDisplay === "trend" ? "selected" : ""}>單行走勢</option><option value="minimal" ${state.settings.rowDisplay === "minimal" ? "selected" : ""}>極簡：方向與名稱</option></select></label><label class="setting-check"><input id="show-codes" type="checkbox" ${state.settings.showCodes ? "checked" : ""}> 顯示代號</label><label class="setting-check"><input id="hide-names" type="checkbox" ${state.settings.hideNames ? "checked" : ""}> 只顯示代號（隱藏名稱）</label><label class="setting-check"><input id="show-change-points" type="checkbox" ${state.settings.showChangePoints !== false ? "checked" : ""}> 顯示漲跌點數</label></div><div class="note density-note">極簡方向為當日漲跌；— 平盤，○ 資料未確認。名稱尚未取得時會使用代號。</div><div class="setting-row"><label class="setting-check"><input id="always-on-top" type="checkbox" ${state.settings.alwaysOnTop !== false ? "checked" : ""}> 視窗保持置頂</label></div><div class="setting-row"><div><strong>視窗透明度</strong><small>只讓主視窗背景變透明，文字與數字維持不透明。</small></div><span class="opacity-control"><input id="window-opacity" type="range" min="40" max="100" step="5" value="${Math.round(shellOpacity(state.settings.windowOpacity) * 100)}"><output id="window-opacity-value">${Math.round(shellOpacity(state.settings.windowOpacity) * 100)}%</output></span></div><div class="setting-row"><div><strong>監看狀態</strong><small>隱藏視窗仍會監看，暫停則停止判定。</small></div><button id="pause" class="secondary">${state.settings.paused ? "繼續監看" : "暫停監看"}</button></div>${desktop ? `<div class="setting-row"><div><strong>程式</strong><small>找不到系統匣時，從這裡重新啟動或結束。結束後就不再監看。</small></div><span class="update-actions"><button id="restart-app" class="secondary">重新啟動</button><button id="quit-app" class="secondary">結束程式</button></span></div>` : ""}<label class="setting-row">更新間隔（秒）<input id="refresh" type="number" min="5" max="300" value="${state.settings.refreshSeconds}"></label><button id="save-refresh" class="secondary">儲存更新間隔</button><div class="setting-row"><label>行情來源<select id="quote-source"><option value="mis" ${state.settings.quoteSource !== "shioaji" ? "selected" : ""}>證交所 MIS（快照，約 1–2 分鐘）</option><option value="shioaji" ${state.settings.quoteSource === "shioaji" ? "selected" : ""}>永豐 Shioaji（即時逐筆）</option></select></label><label>sidecar 連接埠<input id="sidecar-port" type="number" min="1024" max="65535" value="${state.settings.sidecarPort ?? 18765}"></label></div><div class="note">Shioaji 需先完成永豐 API 開通並在本機啟動 sidecar（見 sidecar/README）；sidecar 無回應或缺檔時自動退回 MIS，頁尾會註明。${state.sourceNote ? `<br>目前狀態：${esc(state.sourceNote)}` : ""}</div>${desktop ? `<div class="setting-row"><div><strong>Shioaji 自動啟動</strong><small>開啟後，程式啟動時會自動把 sidecar 跑起來（行情來源需為永豐 Shioaji），不用再手動執行 run-sidecar.bat。金鑰只存放在這台電腦 %APPDATA%\\MaxiStocks 資料夾的獨立檔案，不進設定檔、不上傳，也不會被自動備份或匯出帶走。</small></div><label class="setting-check"><input id="sidecar-autostart" type="checkbox" ${state.settings.sidecarAutoStart ? "checked" : ""}> 自動啟動 sidecar</label></div><div class="setting-row"><label>API Key<input id="shioaji-api-key" type="password" autocomplete="off" placeholder="${shioajiKeys.apiKeySet ? "已設定，輸入新值可更換" : "尚未設定"}"></label><label>Secret Key<input id="shioaji-secret-key" type="password" autocomplete="off" placeholder="${shioajiKeys.secretKeySet ? "已設定，輸入新值可更換" : "尚未設定"}"></label></div><div class="setting-row"><label>sidecar 資料夾路徑<input id="sidecar-dir" type="text" placeholder="例如 C:\\MaxiStocks-Sidecar" value="${esc(state.settings.sidecarDir ?? "")}"></label></div><div class="setting-row"><span class="update-actions"><button id="shioaji-keys-save" class="secondary">儲存金鑰</button><button id="shioaji-keys-clear" class="secondary">清除金鑰</button><button id="sidecar-dir-save" class="secondary">儲存資料夾路徑</button><button id="sidecar-start-now" class="secondary">立即啟動 sidecar</button></span></div><div class="note">金鑰狀態：${esc(shioajiKeysLabel(shioajiKeys))}。更換金鑰時兩欄都要重新輸入。${sidecarMessage ? `<br>${esc(sidecarMessage)}` : ""}</div>` : ""}${desktop ? `<div class="setting-row"><div><strong>agent（MCP，選配）</strong><small id="mcp-status">${esc(mcpMessage || "複製後貼到 Claude、Codex、Gemini 或其他支援本機 MCP 的程式；基本監看不需要它")}</small></div><span class="update-actions"><button id="mcp-json" class="secondary">複製 JSON 設定</button><button id="mcp-claude" class="secondary">Claude Code 指令</button><button id="mcp-codex" class="secondary">Codex 指令</button><button id="mcp-gemini" class="secondary">Gemini 指令</button></span></div>` : ""}<div class="setting-row"><div><strong>自選與持股資料</strong><small>把自選分組、持股（全部購入批次）與月度紀錄匯出成一份 JSON 備份，設定與規則不包含在內；匯入會整份覆蓋目前的自選分組、持股與月度紀錄。</small></div><span class="update-actions"><button id="export-data" class="secondary">匯出自選與持股</button><button id="import-data-btn" class="secondary">匯入自選與持股</button><input id="import-file" type="file" accept=".json,application/json" hidden></span></div>${dataMessage ? `<div class="note">${esc(dataMessage)}</div>` : ""}${desktop ? `<div class="setting-row"><div><strong>Telegram 推播</strong><small>提醒觸發（細條／卡片）時，同步發一則訊息到你的 Telegram。bot token 只保存在這台電腦的設定檔裡，不會出現在狀態、匯出或任何紀錄中。</small></div><label class="setting-check"><input id="tg-enabled" type="checkbox" ${state.settings.telegramEnabled ? "checked" : ""}> 啟用推播</label></div><div class="setting-row"><div><strong>持股跌幅警報</strong><small>任一持股今日跌幅達到 5% 或 10% 時，各發一次卡片通知（同一檔同一級別每天只發一次）。仍需啟用上方 Telegram 推播，才會同步送到 Telegram。</small></div><label class="setting-check"><input id="position-drop-alert" type="checkbox" ${state.settings.positionDropAlert !== false ? "checked" : ""}> 啟用警報</label></div><div class="setting-row"><label>chat id<input id="tg-chat" type="text" inputmode="numeric" placeholder="例如 1926601428" value="${esc(state.settings.telegramChatId ?? "")}"></label><label>bot token<input id="tg-token" type="password" autocomplete="off" placeholder="${state.settings.telegramTokenSet ? "已設定，輸入新值可更換" : "尚未設定"}"></label></div><div class="setting-row"><span class="update-actions"><button id="tg-save" class="secondary">儲存 Telegram 設定</button><button id="tg-clear-token" class="secondary">清除 token</button><button id="tg-test" class="secondary">發送測試訊息</button></span></div>${telegramMessage ? `<div class="note">${esc(telegramMessage)}</div>` : ""}<div class="setting-row"><div><strong>PTT 股票聊天視窗</strong><small>開啟後另開小視窗顯示 PTT 股票聊天室的公開預覽（免登入 Telegram）；關閉開關或直接關掉小視窗都會收起來。</small></div><label class="setting-check"><input id="chat-window" type="checkbox" ${state.settings.chatWindow ? "checked" : ""}> 開啟小視窗</label></div><div class="setting-row"><div><strong>自動備份</strong><small>每天把設定檔（自選、持股、規則與設定）備份一份到指定資料夾，保留最新 14 份。把資料夾指向雲端同步目錄（例如 Google 雲端硬碟的本機同步資料夾）就會自動上雲。</small></div><label class="setting-check"><input id="backup-enabled" type="checkbox" ${state.settings.backupEnabled ? "checked" : ""}> 啟用自動備份</label></div><div class="setting-row"><label>備份資料夾路徑<input id="backup-dir" type="text" placeholder="例如 D:\\Google Drive\\MaxiStocks 備份" value="${esc(state.settings.backupDir ?? "")}"></label></div><div class="setting-row"><span class="update-actions"><button id="backup-save" class="secondary">儲存備份設定</button><button id="backup-now" class="secondary">立即備份</button></span></div><div class="note">${backupMessage ? esc(backupMessage) + "<br>" : ""}${state.lastBackupAt ? `上次備份：${esc(state.lastBackupAt.at)} → ${esc(state.lastBackupAt.path)}` : state.lastBackupError ? `上次備份失敗：${esc(state.lastBackupError)}` : "尚無備份紀錄"}</div>` : ""}<div class="note">所有外觀預設無聲。灰階通知跟隨外觀。<br>${privacyShortcutLabel} 隱藏全部視窗，從系統匣恢復。<br>${state.privacyShortcutError ? esc(state.privacyShortcutError) + "<br>" : ""}資料：TWSE MIS；來源時間與取得時間分開顯示。<br>首次觀測只建立基準；過期或缺少成交價時不觸發。<br>${desktop ? "agent：上方按鈕複製連接設定，說明見 README。" : "瀏覽器僅顯示合成預覽；桌面版才會實際監看與提供 MCP。"}</div></div>`;
  if (tab === "rules")
    content.innerHTML = `<div class="pane"><h1>只在你在意的時候</h1><p class="intro">首次觀測建立基準，之後跨過門檻才提醒。</p><form id="rule-form"><div class="form-grid"><label>股票代號<input name="symbol" required placeholder="2330" value="${esc(editing?.symbol ?? state.watchlist.find((c) => c !== "t00") ?? "2330")}"></label><label>觀測項目<select name="metric"><option value="price">成交價（元）</option><option value="changePercent" ${editing?.metric === "changePercent" ? "selected" : ""}>漲跌幅（成交價，%）</option><option value="mid" ${editing?.metric === "mid" ? "selected" : ""}>買賣中間價（委託價，元）</option><option value="midChangePercent" ${editing?.metric === "midChangePercent" ? "selected" : ""}>漲跌幅（買賣中間價，%）</option></select></label><label>方向<select name="direction"><option value="above">由下往上跨過</option><option value="below" ${editing?.direction === "below" ? "selected" : ""}>由上往下跨過</option></select></label><label>門檻<input name="threshold" type="number" step="any" required placeholder="例如 1000" value="${editing?.threshold ?? ""}"></label><label>通知<select name="channel">${[
      ["strip", "細條"],
      ["card", "卡片"],
      ["log", "只記錄"],
      ["off", "關閉"],
    ]
      .map(
        ([v, t]) =>
          `<option value="${v}" ${editing?.channel === v ? "selected" : ""}>${t}</option>`,
      )
      .join(
        "",
      )}</select></label><label>頻率<select name="repeat"><option value="daily">每交易日一次</option><option value="once" ${editing?.repeat === "once" ? "selected" : ""}>只通知一次</option></select></label></div><button class="primary" type="submit">${editing ? "儲存修改" : "建立提醒"}</button>${editing ? '<button type="button" id="cancel-edit">取消</button>' : ""}</form><div class="rule-list">${state.rules.map((r) => `<div class="rule"><div><strong>${esc(ruleText(r))}</strong><small>${r.enabled ? "監看中" : "已停用"} · ${r.repeat === "daily" ? "每日一次" : "一次性"} · ${r.channel}${r.firedDay ? " · 最近觸發 " + esc(r.firedDay) : ""}</small></div><div><button data-edit="${r.id}" title="修改">修改</button><button data-toggle="${r.id}">${r.enabled ? "停用" : "啟用"}</button><button data-delete="${r.id}" title="刪除">×</button></div></div>`).join("") || '<p class="empty">尚未建立提醒，也可以請 agent 幫你設定。</p>'}</div></div>`;
  if (tab === "history")
    content.innerHTML = `<div class="pane"><h1>發生過的事</h1><p class="intro">最多保留 500 則，時間為資料來源的台灣時間。</p>${state.events.map((e) => `<div class="event"><time>${time(e.sourceAt)}</time><div><strong>${esc(e.text)}</strong><small>偵測於 ${time(e.detectedAt)} · 規則 ${esc(e.ruleId.slice(0, 8))}</small></div></div>`).join("") || '<div class="empty">目前還沒有符合規則的事件。<br>不必一直盯著，有事會告訴你。</div>'}</div>`;

  content.scrollTop = savedScrollTop;
  if (
    tab === "watch" &&
    ["terminal", "txt"].includes(state.settings.appearance)
  )
    bindDisguise(
      content,
      state,
      skinAction,
      openStock,
      () => void fitMinimal(),
    );
  bind();
  if (tab === "watch" && state.settings.rowDisplay === "minimal")
    observeMinimal();
  void resize();
}
function manage() {
  document.body.dataset.page = "manage";
  renderedTab = "manage";
  apply();
  const groups = viewGroups(),
    active = viewActive();
  content.innerHTML = `<div class="pane"><h1>你的自選清單</h1><p class="intro">貼上多檔代號，以空白、逗號或換行分隔。每組最多 30 檔；監看涵蓋所有分組，切換分頁不影響提醒。</p><div class="group-manage"><span>分組</span>${groups.map((g, i) => `<button type="button" class="group-tab${i === active ? " active" : ""}" data-manage-group="${i}">${esc(g.name)}</button>`).join("")}<button type="button" id="group-rename" class="secondary">改名</button><button type="button" id="group-delete" class="secondary">刪除分組</button></div><label>搜尋名稱或代號<input id="search" placeholder="例如：台積電、0050" autocomplete="off"></label><div id="results"></div><label>目前自選（${esc(groups[active]!.name)}）<textarea id="symbols" rows="5">${esc(state.watchlist.join(", "))}</textarea></label><button id="save-symbols" class="primary">儲存自選</button><button id="back-watch">返回</button><p class="note">有啟用中規則的標的，請先停用規則再移除。<br>加權：t00　櫃買：o00</p></div>`;
  if (desktop) void getCurrentWindow().setSize(new LogicalSize(500, 460));
  const regroup = (run: () => Promise<void>) =>
    void (async () => {
      await run();
      manage();
    })();
  document
    .querySelectorAll<HTMLButtonElement>("[data-manage-group]")
    .forEach((b) =>
      b.addEventListener("click", () =>
        regroup(() =>
          mutate("set_active_group", { index: Number(b.dataset.manageGroup) }),
        ),
      ),
    );
  document.querySelector("#group-rename")!.addEventListener("click", () => {
    const current = groups[active]!.name;
    const name = prompt("分組名稱", current);
    if (name == null || !name.trim() || name.trim() === current) return;
    regroup(() =>
      mutate("rename_watch_group", { index: active, name: name.trim() }),
    );
  });
  document.querySelector("#group-delete")!.addEventListener("click", () => {
    if (groups.length <= 1) {
      showError("至少保留一個分組");
      return;
    }
    if (
      !confirm(
        `刪除分組「${groups[active]!.name}」？只在這個分組的自選會停止監看，其他分組不受影響。`,
      )
    )
      return;
    regroup(() => mutate("delete_watch_group", { index: active }));
  });
  document.querySelector("#save-symbols")!.addEventListener("click", () => {
    tab = "watch";
    const symbols = document
      .querySelector<HTMLTextAreaElement>("#symbols")!
      .value.split(/[\s,，;；]+/)
      .filter(Boolean);
    void mutate("set_watchlist", { symbols });
  });
  document.querySelector("#back-watch")!.addEventListener("click", () => {
    tab = "watch";
    render();
  });
  document.querySelector("#search")!.addEventListener("input", async (e) => {
    const version = ++searchVersion;
    const query = (e.target as HTMLInputElement).value;
    await new Promise((r) => setTimeout(r, 350));
    if (version !== searchVersion) return;
    try {
      const results = await search(query);
      if (version !== searchVersion) return;
      document.querySelector("#results")!.innerHTML = results
        .map(
          (r) =>
            `<button type="button" data-add="${esc(r.symbol)}">＋ ${esc(r.name)} <small>${esc(r.symbol)}</small></button>`,
        )
        .join("");
      document.querySelectorAll<HTMLButtonElement>("[data-add]").forEach(
        (b) =>
          (b.onclick = () => {
            const field =
              document.querySelector<HTMLTextAreaElement>("#symbols")!;
            const list = field.value.split(/[\s,，;；]+/).filter(Boolean);
            if (!list.includes(b.dataset.add!)) list.push(b.dataset.add!);
            field.value = list.join(", ");
          }),
      );
    } catch (e) {
      showError(e);
    }
  });
}
function bind() {
  // Portfolio day chart hover: swap the readout line for the aggregate
  // value at the hovered session minute (latest point at or before it),
  // restoring the resting summary on leave.
  const pfWrap = document.querySelector<HTMLElement>(".pf-chart-wrap");
  const pfReadout = document.querySelector<HTMLElement>(".pf-readout");
  if (pfWrap && pfReadout && pfHover?.points.length) {
    const data = pfHover;
    pfWrap.addEventListener("mousemove", (e) => {
      const rect = pfWrap.getBoundingClientRect();
      if (!rect.width) return;
      const frac = Math.max(
        0,
        Math.min(1, (e.clientX - rect.left) / rect.width),
      );
      const target =
        data.domain[0] + frac * (data.domain[1] - data.domain[0]);
      let pt = data.points[0]!;
      for (const p of data.points) {
        if (p.at <= target) pt = p;
        else break;
      }
      const pct =
        data.baseline != null && data.baseline !== 0
          ? `（${change((pt.value / data.baseline - 1) * 100)}）`
          : "";
      const when = data.mode === "time" ? hm(pt.at) : ymd(pt.at);
      pfReadout.textContent = `${when} 總值 ${formatMoney(pt.value, "TWD")}${pct}`;
    });
    pfWrap.addEventListener("mouseleave", () => {
      pfReadout.textContent = pfReadoutDefault;
    });
  }
  // Portfolio chart range tabs: switch the module state and re-render in
  // place (same tab, so the scroll place survives per shouldKeepScroll).
  document
    .querySelectorAll<HTMLButtonElement>("[data-pf-range]")
    .forEach((b) =>
      b.addEventListener("click", () => {
        pfRange = b.dataset.pfRange as PfRange;
        render();
      }),
    );
  // Settings-page data export/import: on desktop the core writes the JSON
  // to a file itself (a Blob download is silently ignored by WebView2) and
  // the outcome line reports the path; the browser preview keeps the Blob
  // download. The import replaces groups and positions wholesale, so it
  // confirms first and reports the core's tally afterwards.
  document.querySelector("#export-data")?.addEventListener("click", () => {
    void (async () => {
      try {
        if (desktop) {
          const path = await exportDataFile();
          dataMessage = `已匯出到 ${path}`;
        } else {
          const text = await exportData();
          const day = new Date(state.now)
            .toLocaleDateString("sv-SE", { timeZone: "Asia/Taipei" })
            .replaceAll("-", "");
          const url = URL.createObjectURL(
            new Blob([text], { type: "application/json" }),
          );
          const a = document.createElement("a");
          a.href = url;
          a.download = `maxi-stocks-data-${day}.json`;
          a.click();
          setTimeout(() => URL.revokeObjectURL(url), 5000);
          dataMessage = "已匯出自選與持股（JSON 備份檔）。";
        }
      } catch (e) {
        dataMessage = `匯出失敗：${String(e)}`;
      }
      render();
    })();
  });
  const importFile =
    document.querySelector<HTMLInputElement>("#import-file");
  document
    .querySelector("#import-data-btn")
    ?.addEventListener("click", () => importFile?.click());
  importFile?.addEventListener("change", () => {
    const file = importFile.files?.[0];
    importFile.value = "";
    if (!file) return;
    void (async () => {
      try {
        const text = await file.text();
        if (
          !confirm(
            "匯入會覆蓋目前的自選分組、持股與月度紀錄（設定與規則不受影響），確定要匯入這份備份？",
          )
        )
          return;
        state = await importData(text);
        const r = state.dataImport;
        dataMessage = r
          ? `匯入完成：${r.groups} 個分組、${r.positions} 檔持股、${r.lots} 筆購入批次${r.skippedSymbols.length ? `；跳過無效代號 ${r.skippedSymbols.join("、")}` : ""}${r.skippedLots ? `；跳過 ${r.skippedLots} 筆不合法的批次` : ""}${r.skippedSnapshots ? `；跳過 ${r.skippedSnapshots} 筆不合法的月度紀錄` : ""}。`
          : "匯入完成。";
        apply();
      } catch (e) {
        dataMessage = `匯入失敗：${String(e)}`;
      }
      render();
    })();
  });
  document
    .querySelector(".minimal-grip")
    ?.addEventListener("pointerdown", hideDetail);
  document
    .querySelector("#minimal-cycle")
    ?.addEventListener(
      "click",
      () => void mutate("set_settings", { rowDisplay: "numbers" }),
    );
  document.querySelector("#minimal-settings")?.addEventListener("click", () => {
    tab = "settings";
    render();
  });
  document.querySelector("#display-mode")?.addEventListener(
    "change",
    (e) =>
      void mutate("set_settings", {
        rowDisplay: (e.target as HTMLSelectElement).value,
      }),
  );
  document.querySelector("#always-on-top")?.addEventListener(
    "change",
    (e) =>
      void mutate("set_settings", {
        alwaysOnTop: (e.target as HTMLInputElement).checked,
      }),
  );
  document.querySelector("#quote-source")?.addEventListener(
    "change",
    (e) =>
      void mutate("set_settings", {
        quoteSource: (e.target as HTMLSelectElement).value,
      }),
  );
  document.querySelector("#sidecar-port")?.addEventListener(
    "change",
    (e) =>
      void mutate("set_settings", {
        sidecarPort: Number((e.target as HTMLInputElement).value),
      }),
  );
  // Shioaji sidecar auto-start: the toggle and the folder path are plain
  // settings; the keys are not — they go to their own local file through
  // the dedicated commands, and only their existence is ever read back.
  document.querySelector("#sidecar-autostart")?.addEventListener(
    "change",
    (e) =>
      void mutate("set_settings", {
        sidecarAutoStart: (e.target as HTMLInputElement).checked,
      }),
  );
  document.querySelector("#sidecar-dir-save")?.addEventListener("click", () => {
    const dir =
      document.querySelector<HTMLInputElement>("#sidecar-dir")!.value.trim();
    sidecarMessage = "sidecar 資料夾路徑已儲存。";
    void mutate("set_settings", { sidecarDir: dir });
  });
  document.querySelector("#shioaji-keys-save")?.addEventListener("click", () => {
    const apiKey =
      document.querySelector<HTMLInputElement>("#shioaji-api-key")!.value;
    const secretKey =
      document.querySelector<HTMLInputElement>("#shioaji-secret-key")!.value;
    void (async () => {
      try {
        await shioajiKeysSet(apiKey, secretKey);
        shioajiKeys = await shioajiKeysStatus();
        sidecarMessage = "金鑰已儲存到這台電腦。";
      } catch (e) {
        sidecarMessage = `儲存金鑰失敗：${String(e)}`;
      }
      render();
    })();
  });
  document.querySelector("#shioaji-keys-clear")?.addEventListener("click", () => {
    void (async () => {
      try {
        await shioajiKeysClear();
        shioajiKeys = await shioajiKeysStatus();
        sidecarMessage = "已清除本機金鑰，自動啟動會在重新儲存金鑰前停用。";
      } catch (e) {
        sidecarMessage = `清除金鑰失敗：${String(e)}`;
      }
      render();
    })();
  });
  document.querySelector("#sidecar-start-now")?.addEventListener("click", () => {
    void (async () => {
      try {
        sidecarMessage = await sidecarStartNow();
      } catch (e) {
        sidecarMessage = `啟動失敗：${String(e)}`;
      }
      render();
    })();
  });
  // Telegram push settings: the toggle applies at once; chat id and the
  // write-only token go through the save button (a blank token field means
  // "keep the stored one" — clearing has its own button).
  document.querySelector("#tg-enabled")?.addEventListener(
    "change",
    (e) =>
      void mutate("set_settings", {
        telegramEnabled: (e.target as HTMLInputElement).checked,
      }),
  );
  document.querySelector("#position-drop-alert")?.addEventListener(
    "change",
    (e) =>
      void mutate("set_settings", {
        positionDropAlert: (e.target as HTMLInputElement).checked,
      }),
  );
  // PTT chat window: the toggle applies at once; the core owns the
  // setting and lib.rs creates or closes the window to match. Closing
  // the window by hand flips the setting off in the core, so the next
  // render shows this toggle off again.
  document.querySelector("#chat-window")?.addEventListener(
    "change",
    (e) =>
      void mutate("set_settings", {
        chatWindow: (e.target as HTMLInputElement).checked,
      }),
  );
  document.querySelector("#tg-save")?.addEventListener("click", () => {
    const chatId =
      document.querySelector<HTMLInputElement>("#tg-chat")!.value.trim();
    const token =
      document.querySelector<HTMLInputElement>("#tg-token")!.value.trim();
    telegramMessage = "Telegram 設定已儲存。";
    void mutate("set_settings", {
      telegramChatId: chatId,
      ...(token ? { telegramBotToken: token } : {}),
    });
  });
  document.querySelector("#tg-clear-token")?.addEventListener("click", () => {
    telegramMessage = "已清除 bot token，推播會在重新設定前停用。";
    void mutate("set_settings", { telegramClearToken: true });
  });
  document.querySelector("#tg-test")?.addEventListener("click", () => {
    void (async () => {
      try {
        await telegramTest();
        telegramMessage = "測試訊息已送出，請查看 Telegram。";
      } catch (e) {
        telegramMessage = `測試失敗：${String(e)}`;
      }
      render();
    })();
  });
  // State backups: toggle applies at once, the folder path via its save
  // button, and 立即備份 reports the path the core wrote.
  document.querySelector("#backup-enabled")?.addEventListener(
    "change",
    (e) =>
      void mutate("set_settings", {
        backupEnabled: (e.target as HTMLInputElement).checked,
      }),
  );
  document.querySelector("#backup-save")?.addEventListener("click", () => {
    const dir =
      document.querySelector<HTMLInputElement>("#backup-dir")!.value.trim();
    backupMessage = "備份設定已儲存。";
    void mutate("set_settings", { backupDir: dir });
  });
  document.querySelector("#backup-now")?.addEventListener("click", () => {
    void (async () => {
      try {
        const path = await backupNow();
        backupMessage = `已備份到 ${path}`;
      } catch (e) {
        backupMessage = `備份失敗：${String(e)}`;
      }
      render();
    })();
  });
  for (const [id, kind] of [
    ["#mcp-json", "json"],
    ["#mcp-claude", "claude"],
    ["#mcp-codex", "codex"],
    ["#mcp-gemini", "gemini"],
  ] as [string, McpKind][]) {
    document.querySelector(id)?.addEventListener("click", () => {
      const note = (text: string) => {
        mcpMessage = text;
        const el = document.querySelector("#mcp-status");
        if (el) el.textContent = text;
      };
      // Copy inside the click itself: macOS's web view refuses a clipboard write that comes after
      // an await. The program's path is fetched at start-up for that reason.
      if (!mcpExe) return note("複製失敗：還沒取得程式位置，請稍後再試");
      void copyText(mcpSetup(kind, mcpExe))
        .then(() =>
          note(
            "已複製。貼到 agent 程式的 MCP 設定；設定內不含密碼，連線密碼只存在本機。",
          ),
        )
        .catch((e) => note("複製失敗：" + String(e)));
    });
  }
  document.querySelector("#restart-app")?.addEventListener("click", () => {
    void invoke("restart_app").catch(showError);
  });
  document.querySelector("#quit-app")?.addEventListener("click", () => {
    void invoke("quit_app").catch(showError);
  });
  document.querySelector("#show-codes")?.addEventListener(
    "change",
    (e) =>
      void mutate("set_settings", {
        showCodes: (e.target as HTMLInputElement).checked,
      }),
  );
  document.querySelector("#hide-names")?.addEventListener(
    "change",
    (e) =>
      void mutate("set_settings", {
        hideNames: (e.target as HTMLInputElement).checked,
      }),
  );
  document.querySelector("#show-change-points")?.addEventListener(
    "change",
    (e) =>
      void mutate("set_settings", {
        showChangePoints: (e.target as HTMLInputElement).checked,
      }),
  );
  const opacityInput =
    document.querySelector<HTMLInputElement>("#window-opacity");
  opacityInput?.addEventListener("input", () => {
    // Preview only: apply() prefers the preview over the persisted value, so
    // the per-second poll cannot snap the shell back mid-drag.
    opacityPreview = Number(opacityInput.value) / 100;
    document.documentElement.style.setProperty(
      "--bg-alpha",
      String(opacityPreview),
    );
    const out = document.querySelector("#window-opacity-value");
    if (out) out.textContent = `${opacityInput.value}%`;
  });
  opacityInput?.addEventListener("change", () => {
    const value = Number(opacityInput.value) / 100;
    void mutate("set_settings", { windowOpacity: value }).then(() => {
      opacityPreview = null;
    });
  });
  document.querySelectorAll<HTMLButtonElement>("[data-group]").forEach(
    (b) =>
      (b.onclick = () => {
        // Switching groups swaps the list wholesale: start it at the top
        // instead of keeping the previous group's scroll place.
        scrollResetPending = true;
        void mutate("set_active_group", { index: Number(b.dataset.group) });
      }),
  );
  document.querySelectorAll<HTMLButtonElement>("[data-pos-market]").forEach(
    (b) =>
      (b.onclick = () => {
        // Same wholesale swap as a group switch: remember the market and
        // start its table at the top.
        positionsMarket = b.dataset.posMarket as "TWD" | "USD" | "monthly";
        scrollResetPending = true;
        render();
      }),
  );
  document.querySelectorAll<HTMLButtonElement>("[data-pos-sort]").forEach(
    (b) =>
      (b.onclick = () => {
        // Reordering the same table in place: keep the scroll place, just
        // re-render with the next sort in the click cycle.
        positionsSort = nextPositionSort(
          positionsSort,
          b.dataset.posSort as PositionSortKey,
        );
        render();
      }),
  );
  document
    .querySelector("[data-group-add]")
    ?.addEventListener("click", () => {
      scrollResetPending = true;
      void mutate("add_watch_group", {});
    });
  if (
    tab === "watch" &&
    state.settings.rowDisplay === "minimal" &&
    !content.querySelector("[data-disguise]")
  ) {
    bindMinimal(content, state, openStock);
  }
  document.querySelectorAll<HTMLButtonElement>("[data-quote]").forEach(
    (b) =>
      (b.onclick = () => {
        openStock(b.dataset.quote!);
      }),
  );
  document.querySelector<HTMLButtonElement>("#manage")!.onclick = () => {
    tab = "manage";
    manage();
  };
  document.querySelectorAll<HTMLButtonElement>("[data-look]").forEach(
    (b) =>
      (b.onclick = () =>
        void mutate("set_settings", {
          appearance: b.dataset.look as Appearance,
        })),
  );
  document
    .querySelector("#pause")
    ?.addEventListener(
      "click",
      () => void mutate("set_settings", { paused: !state.settings.paused }),
    );
  document.querySelector("#save-refresh")?.addEventListener(
    "click",
    () =>
      void mutate("set_settings", {
        refreshSeconds: Number(
          document.querySelector<HTMLInputElement>("#refresh")!.value,
        ),
      }),
  );
  document.querySelector("#cancel-edit")?.addEventListener("click", () => {
    editing = undefined;
    scrollResetPending = true;
    render();
  });
  document.querySelector("#rule-form")?.addEventListener("submit", (e) => {
    e.preventDefault();
    const data = Object.fromEntries(
      new FormData(e.target as HTMLFormElement).entries(),
    );
    void mutate("upsert_rule", {
      ...data,
      threshold: Number(data.threshold),
      enabled: editing?.enabled ?? true,
      ...(editing ? { id: editing.id } : {}),
    });
    editing = undefined;
  });
  document.querySelectorAll<HTMLButtonElement>("[data-edit]").forEach(
    (b) =>
      (b.onclick = () => {
        editing = state.rules.find((r) => r.id === b.dataset.edit);
        // The populated form sits at the top of the pane: go to it.
        scrollResetPending = true;
        render();
      }),
  );
  document
    .querySelectorAll<HTMLButtonElement>("[data-delete]")
    .forEach(
      (b) =>
        (b.onclick = () =>
          void mutate("delete_rule", { id: b.dataset.delete })),
    );
  document.querySelectorAll<HTMLButtonElement>("[data-toggle]").forEach(
    (b) =>
      (b.onclick = () => {
        const r = state.rules.find((r) => r.id === b.dataset.toggle)!;
        void mutate("upsert_rule", { ...r, enabled: !r.enabled });
      }),
  );
  document.querySelector("#lot-form")?.addEventListener("submit", (e) => {
    e.preventDefault();
    const data = Object.fromEntries(
      new FormData(e.target as HTMLFormElement).entries(),
    );
    void mutate("add_lot", {
      code: String(data.code ?? ""),
      date: String(data.date ?? ""),
      price: Number(data.price),
      shares: Number(data.shares),
    });
  });
  document.querySelector("#import-btn")?.addEventListener("click", () => {
    const text =
      document.querySelector<HTMLTextAreaElement>("#import-text")!.value;
    void (async () => {
      try {
        // Not mutate(): the import tally rides this one response and is kept
        // in a module variable so later polls do not wipe the outcome line.
        state = await request("import_positions", { text });
        const r = state.positionsImport;
        positionsMessage = r
          ? `匯入完成：成功 ${r.added} 筆${r.failedLines.length ? `；失敗 ${r.failedLines.length} 筆（第 ${r.failedLines.join("、")} 行）` : ""}`
          : "";
        apply();
        render();
      } catch (e) {
        showError(e);
      }
    })();
  });
  document.querySelectorAll<HTMLElement>("[data-expand]").forEach((el) =>
    el.addEventListener("click", () => {
      const code = el.dataset.expand!;
      expandedPosition = expandedPosition === code ? null : code;
      render();
    }),
  );
  document.querySelectorAll<HTMLButtonElement>("[data-del-lot]").forEach(
    (b) =>
      (b.onclick = () => {
        const [code, idx] = b.dataset.delLot!.split(":");
        void mutate("delete_lot", { code, index: Number(idx) });
      }),
  );
  document.querySelectorAll<HTMLButtonElement>("[data-add-lot]").forEach(
    (b) =>
      (b.onclick = () => {
        const code = document.querySelector<HTMLInputElement>("#lot-code");
        if (!code) return;
        code.value = displaySymbol(b.dataset.addLot!);
        document
          .querySelector<HTMLInputElement>('#lot-form [name="price"]')
          ?.focus();
      }),
  );
  document
    .querySelectorAll<HTMLButtonElement>("[data-del-position]")
    .forEach(
      (b) =>
        (b.onclick = () => {
          const code = b.dataset.delPosition!;
          if (!confirm(`刪除 ${displaySymbol(code)} 的全部持股紀錄？`)) return;
          if (expandedPosition === code) expandedPosition = null;
          void mutate("delete_position", { code });
        }),
    );
  document.querySelectorAll<HTMLButtonElement>("[data-del-month]").forEach(
    (b) =>
      (b.onclick = () => {
        const month = b.dataset.delMonth!;
        if (!confirm(`刪除 ${month} 的月度紀錄？`)) return;
        void mutate("delete_monthly_snapshot", { month });
      }),
  );
}
document.querySelectorAll<HTMLButtonElement>("[data-tab]").forEach(
  (b) =>
    (b.onclick = () => {
      tab = b.dataset.tab!;
      editing = undefined;
      render();
    }),
);
document.querySelector("#row-display")!.addEventListener(
  "click",
  () =>
    void mutate("set_settings", {
      rowDisplay:
        state.settings.rowDisplay === "minimal"
          ? "numbers"
          : state.settings.rowDisplay === "trend"
            ? "minimal"
            : "trend",
    }),
);
document.querySelector("#hide")!.addEventListener("click", () => {
  hideDetail();
  if (desktop) void getCurrentWindow().minimize();
  else showError("桌面版可縮到工作列，監看會持續。");
});
document.querySelector("#quit")!.addEventListener("click", () => {
  hideDetail();
  if (desktop) void invoke("quit_app").catch(showError);
  else showError("桌面版才能結束程式。");
});
const togglePin = () =>
  void mutate("set_settings", {
    alwaysOnTop: state.settings.alwaysOnTop === false,
  });
document.querySelector("#pin")!.addEventListener("click", togglePin);
const stealth = () => {
  hideDetail();
  if (desktop) void invoke("privacy_hide").catch(showError);
  else showError("桌面版可隱藏所有視窗，監看仍會持續。");
};
document.querySelector("#stealth")!.addEventListener("click", stealth);
document.addEventListener("click", (e) => {
  const target = e.target as Element;
  if (target.closest("#minimal-pin")) togglePin();
  if (target.closest("#minimal-stealth")) stealth();
});
document.addEventListener("keydown", (e) => {
  if (privacyPressed(e)) {
    e.preventDefault();
    hideDetail();
    if (desktop) void invoke("privacy_hide").catch(showError);
    else showError("桌面版可隱藏所有視窗，監看仍會持續。");
    return;
  }
  if (
    tab === "watch" &&
    state?.settings.rowDisplay === "minimal" &&
    document.body.dataset.skin === "small" &&
    primaryPressed(e) &&
    e.key === ","
  ) {
    e.preventDefault();
    tab = "settings";
    render();
  }
});
async function poll() {
  if (busy) return;
  busy = true;
  try {
    const next = await request("get_status");
    const previous = state;
    state = next;
    apply();
    if (tab === "daily" && dailyData?.pending) void loadDaily();
    if (!previous) render();
    else if (tab === "watch") {
      if (["terminal", "txt"].includes(state.settings.appearance)) {
        if (
          content
            .querySelector("[data-disguise]")
            ?.getAttribute("data-disguise") !== state.settings.appearance ||
          content
            .querySelector("[data-disguise]")
            ?.getAttribute("data-row-mode") !==
            (state.settings.rowDisplay ?? "numbers")
        )
          render();
        else updateDisguise(content, state);
      } else {
        content.onkeydown = null;
        if (
          state.settings.rowDisplay === "minimal" &&
          content.querySelector(".minimal-list")
        ) {
          updateMinimal(content, state);
        } else {
          if (content.querySelector(".minimal-list")) hideDetail();
          content.innerHTML = watch();
          bind();
          if (state.settings.rowDisplay === "minimal") observeMinimal();
        }
      }
      if (
        previous.settings.appearance !== state.settings.appearance ||
        previous.watchlist.length !== state.watchlist.length ||
        Boolean(previous.events.length) !== Boolean(state.events.length)
      )
        void resize();
    } else if (tab === "positions") {
      // The portfolio page follows live quotes: re-render when the valued
      // prices move (or the state revision does), but never while the user is
      // typing in the lot form or the import box. render() itself keeps the
      // scroll place for same-view re-renders.
      const el = document.activeElement as HTMLElement | null;
      const typing =
        !!el &&
        content.contains(el) &&
        ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName);
      if (
        !typing &&
        (previous.revision !== state.revision ||
          positionsSignature() !== positionsSig)
      )
        render();
    } else if (
      previous.revision !== state.revision &&
      !content.contains(document.activeElement)
    )
      render();
  } catch (e) {
    showError(e);
  } finally {
    busy = false;
  }
}
void poll();
setInterval(() => void poll(), 1000);
if (desktop)
  void invoke<string>("mcp_exe")
    .then((e) => (mcpExe = e))
    .catch(() => {});
if (desktop)
  void shioajiKeysStatus()
    .then((s) => (shioajiKeys = s))
    .catch(() => {});
