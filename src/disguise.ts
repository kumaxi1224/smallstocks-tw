import {
  esc,
  number,
  change,
  time,
  latestCloseDate,
  stale,
  livePrice,
  closeDay,
  oldestDataAt,
  displaySymbol,
  isYahooSymbol,
} from "./model.ts";
import type { Snapshot } from "./model.ts";
import { minimalMarkup, bindMinimal, updateMinimal } from "./minimal.ts";
import { stockDetail } from "./stock-view.ts";
import { isMac, primaryPressed, shellPrompt } from "./platform.ts";
export type SkinAction =
  | "watch"
  | "rules"
  | "history"
  | "settings"
  | "manage"
  | "small"
  | "hide"
  | "minimize"
  | "maximize"
  | "pause"
  | "resume";
export type SkinHandler = (action: SkinAction) => void;
let lastState: Snapshot,
  handler: SkinHandler,
  zoom = 100,
  showStatus = true;
let commandLog: string[] = [];
let relayout: () => void = () => {};
const pad = (text: string, cells: number) => {
  let out = "",
    used = 0;
  for (const c of text) {
    const n = c.charCodeAt(0) > 255 ? 2 : 1;
    if (used + n > cells) break;
    out += c;
    used += n;
  }
  return out + " ".repeat(cells - used);
};
function values(s: Snapshot, c: string) {
  const q = s.quotes[c],
    // Yahoo symbols never take the Taiwan close-day view.
    closed = !s.marketOpen && !s.demo && !isYahooSymbol(c),
    d = closed ? closeDay(s, c) : undefined,
    live = livePrice(q);
  const fullName =
    q?.name || (c === "t00" ? "加權指數" : c === "o00" ? "櫃買指數" : "");
  return {
    code: displaySymbol(c),
    name: s.settings.hideNames ? displaySymbol(c) : fullName,
    price: closed ? d?.close : live.price,
    book: !closed && live.basis === "book",
    date: d?.date,
    pct: closed ? d?.changePercent : live.pct,
    status: closed
      ? d
        ? d.date !== latestCloseDate(s)
          ? d.date.slice(5).replace("-", "/")
          : d.kind === "final"
            ? "CLOSED(MIS)"
            : "CLOSED"
        : "WAITING"
      : stale(q, s.now)
        ? "STALE"
        : live.basis === "book"
          ? "BOOK"
          : live.basis === "none"
            ? "NO PRICE"
            : "OK",
  };
}
function dateLabel(s: Snapshot) {
  const d = latestCloseDate(s);
  return !s.marketOpen && !s.demo
    ? d
      ? d.replaceAll("-", "/") + " 收盤"
      : "等待收盤資料"
    : oldestDataAt(s)
      ? `資料 ${time(oldestDataAt(s)!)}（取得 ${time(s.lastFetch)}）`
      : time(s.lastFetch);
}
export function notepadText(s: Snapshot): string {
  const lines = [
    "資料紀錄",
    `${!s.marketOpen && !s.demo ? "資料日期" : "更新"}：${dateLabel(s)}`,
    ...(s.demo ? ["示範資料（非真實行情）"] : []),
    "",
    (s.settings.showCodes ? pad("代號", 7) : "") +
      pad("名稱", 14) +
      pad("數值", 12) +
      pad("變化", 10),
    ...s.watchlist.map((c) => {
      const v = values(s, c);
      return (
        (s.settings.showCodes ? pad(v.code, 7) : "") +
        pad(v.name || v.code, 14) +
        ((v.book ? "~" : "") + number(v.price)).padStart(10) +
        "  " +
        change(v.pct).padStart(8) +
        (v.date && v.date !== latestCloseDate(s)
          ? `  (${v.date.slice(5).replace("-", "/")})`
          : "")
      );
    }),
    "",
    s.settings.paused
      ? "狀態：已暫停"
      : !s.marketOpen && !s.demo
        ? "狀態：已收盤"
        : "狀態：追蹤中",
  ];
  if (s.events.length)
    lines.push(
      "",
      "最近紀錄",
      ...s.events.slice(0, 4).map((e) => `${time(e.sourceAt)}  ${e.text}`),
    );
  return lines.join("\n");
}
function terminalText(s: Snapshot): string {
  const lines = [
    `${shellPrompt} npm run sync -- --watch`,
    "",
    `> sync-job  monitor`,
    `> refresh=${s.settings.refreshSeconds}s  streams=${s.watchlist.length}  rules=${s.rules.filter((r) => r.enabled).length}`,
    ...(s.demo ? ["[demo] synthetic data — not market quotes"] : []),
    "",
    `[${time(s.lastFetch)}] ${s.settings.paused ? "PAUSED" : !s.marketOpen && !s.demo ? "CLOSED" : "RUNNING"}${!s.marketOpen && !s.demo ? "  " + dateLabel(s) : ""}`,
    "",
    (s.settings.showCodes ? pad("ID", 7) : "") +
      pad("NAME", 14) +
      pad("LAST", 12) +
      pad("CHANGE", 11) +
      "STATE",
    ...s.watchlist.map((c) => {
      const v = values(s, c);
      return (
        (s.settings.showCodes ? pad(v.code, 7) : "") +
        pad(v.name || v.code, 14) +
        ((v.book ? "~" : "") + number(v.price)).padStart(10) +
        "  " +
        change(v.pct).padStart(9) +
        "  " +
        v.status
      );
    }),
  ];
  if (s.events.length)
    lines.push(
      "",
      ...s.events.slice(0, 3).map((e) => `[${time(e.sourceAt)}] ${e.text}`),
    );
  if (s.error)
    lines.push("", "[warning] feed connection unavailable; retry scheduled");
  if (commandLog.length) lines.push("", ...commandLog);
  return lines.join("\n");
}
/**
 * The notepad body as fixed-width field spans (the terminalMarkup
 * approach): each column is an inline-block box of a set width, so the
 * value and change columns start at the same x on every row no matter
 * what the name contains. Padding the text alone cannot promise that —
 * in WebView2 the CJK glyphs of the fallback font are not exactly two
 * half-width cells, so padded rows drifted apart. The plain-text twin
 * (notepadText) stays the canonical text for the character count and
 * copy-all; both render the same values() rows.
 */
export function notepadMarkup(s: Snapshot): string {
  const lines = notepadText(s).split("\n");
  const header =
    (s.settings.showCodes ? pad("代號", 7) : "") +
    pad("名稱", 14) +
    pad("數值", 12) +
    pad("變化", 10);
  const index = lines.indexOf(header);
  if (index < 0) return esc(lines.join("\n"));
  const cell = (value: string, kind: string) =>
    `<span class="note-field ${kind}">${esc(value)}</span>`;
  const head =
    (s.settings.showCodes ? cell("代號", "code") : "") +
    cell("名稱", "name") +
    cell("數值  ", "value") +
    cell("變化", "change");
  const rows = s.watchlist.map((c) => {
    const v = values(s, c);
    return (
      (s.settings.showCodes ? cell(pad(v.code, 7), "code") : "") +
      cell(pad(v.name || v.code, 14), "name") +
      cell((v.book ? "~" : "") + number(v.price) + "  ", "value") +
      cell(change(v.pct), "change") +
      (v.date && v.date !== latestCloseDate(s)
        ? esc(`  (${v.date.slice(5).replace("-", "/")})`)
        : "")
    );
  });
  return (
    esc(lines.slice(0, index).join("\n") + "\n") +
    head +
    "\n" +
    rows.join("\n") +
    esc("\n" + lines.slice(index + 1 + s.watchlist.length).join("\n"))
  );
}
function terminalMarkup(s: Snapshot): string {
  const all = terminalText(s).split("\n");
  const index = all.findIndex((l) =>
    l.startsWith((s.settings.showCodes ? pad("ID", 7) : "") + pad("NAME", 14)),
  );
  const cell = (value: string, kind: string) =>
    `<span class="console-field ${kind}">${esc(value)}</span>`;
  const head =
    (s.settings.showCodes ? cell("ID", "id") : "") +
    cell("NAME", "name") +
    cell("LAST  ", "last") +
    cell("CHANGE  ", "change") +
    cell("STATE", "state");
  const rows = s.watchlist.map((c) => {
    const v = values(s, c);
    return (
      (s.settings.showCodes ? cell(pad(v.code, 7), "id") : "") +
      cell(pad(v.name || v.code, 14), "name") +
      cell((v.book ? "~" : "") + number(v.price) + "  ", "last") +
      cell(change(v.pct) + "  ", "change") +
      cell(v.status, "state")
    );
  });
  return (
    esc(all.slice(0, index).join("\n") + "\n") +
    head +
    "\n" +
    rows.join("\n") +
    esc("\n" + all.slice(index + 1 + s.watchlist.length).join("\n"))
  );
}
const controls = () =>
  `<div class="native-controls"><button data-skin-action="minimize" aria-label="最小化" title="最小化"><i class="cap-min"></i></button><button data-skin-action="maximize" aria-label="最大化或還原" title="最大化或還原"><i class="cap-max"></i></button><button data-skin-action="hide" class="cap-close" aria-label="關閉視窗" title="關閉視窗，繼續監看"><i></i></button></div>`;
const item = (label: string, action: SkinAction, shortcut = "") =>
  `<button data-skin-action="${action}" role="menuitem"><span>${label}</span><small>${shortcut}</small></button>`;
const menu = (label: string, body: string) =>
  `<details class="native-menu"><summary>${label}</summary><div class="native-popup" role="menu">${body}</div></details>`;
function macMarkup(
  s: Snapshot,
  mini: string,
  minimal: boolean,
  terminal: boolean,
) {
  const trafficLights =
    '<div class="mac-controls"><button class="mac-close" data-skin-action="hide" aria-label="關閉視窗" title="關閉視窗，繼續監看"></button><button class="mac-minimize" data-skin-action="minimize" aria-label="最小化" title="最小化"></button><button class="mac-maximize" data-skin-action="maximize" aria-label="最大化或還原" title="最大化或還原"></button></div>';
  const actions =
    item("自選清單…", "manage") +
    item("提醒設定…", "rules") +
    item("事件紀錄", "history") +
    "<hr>" +
    item("設定", "settings", "⌘,") +
    item("返回 small", "small") +
    item("隱藏視窗", "hide");
  const editing =
    '<hr><button data-note-edit="select-all" role="menuitem"><span>全選</span><small>⌘A</small></button><button data-note-edit="copy" role="menuitem"><span>複製</span><small>⌘C</small></button><hr><button data-zoom="in" role="menuitem"><span>放大</span><small>⌘+</small></button><button data-zoom="out" role="menuitem"><span>縮小</span><small>⌘−</small></button><button data-zoom="reset" role="menuitem"><span>還原預設縮放</span><small>⌘0</small></button>';
  const title = terminal ? "work — sync — zsh" : "工作紀錄.txt";
  const titlebar = `<div class="native-title mac-titlebar" data-tauri-drag-region>${trafficLights}<span class="mac-document-title" data-tauri-drag-region>${title}</span><div class="native-drag" data-tauri-drag-region></div><div class="mac-view-menu">${menu("⋯", actions + (terminal ? "" : editing))}</div></div>`;
  if (terminal)
    return `<section class="native-frame terminal-view mac-terminal" data-disguise="terminal" data-row-mode="${s.settings.rowDisplay ?? "numbers"}">${titlebar}<div class="terminal-screen">${minimal ? mini : ""}<pre class="console-output${minimal ? " console-minimal-log" : ""}"></pre><form class="console-command" autocomplete="off"><span>${shellPrompt}</span><input aria-label="終端機模式指令" spellcheck="false" autocomplete="off" autocapitalize="off"></form></div></section>`;
  return `<section class="native-frame text-view mac-textedit" data-disguise="txt" data-row-mode="${s.settings.rowDisplay ?? "numbers"}">${titlebar}${minimal ? mini : '<div class="notepad-editor notepad-fields" tabindex="0" aria-label="工作紀錄文字內容"></div>'}<div class="notepad-status" hidden><span class="caret-status"></span><span class="character-status"></span><span class="zoom-status">100%</span></div></section>`;
}
export function disguiseMarkup(s: Snapshot): string {
  const terminal = s.settings.appearance === "terminal";
  const minimal = s.settings.rowDisplay === "minimal";
  const mini = `<div class="minimal-native"><div class="minimal-native-demo" ${!s.demo && !s.settings.paused ? "hidden" : ""}>${s.demo ? "示範資料（非真實行情）" : ""}${s.settings.paused ? " 已暫停監看" : ""}</div>${minimalMarkup(s)}</div>`;
  if (isMac) return macMarkup(s, mini, minimal, terminal);
  if (terminal)
    return `<section class="native-frame terminal-view windows-terminal" data-disguise="terminal" data-row-mode="${s.settings.rowDisplay ?? "numbers"}"><div class="native-title terminal-titlebar" data-tauri-drag-region><div class="terminal-tab"><span class="console-icon" aria-hidden="true">&gt;_</span><span>PowerShell</span><button data-skin-action="hide" aria-label="關閉分頁">×</button></div><button class="native-add" data-skin-action="manage" aria-label="新增監看項目" title="新增監看項目">＋</button>${menu("⌄", item("自選清單…", "manage") + item("提醒設定…", "rules") + item("事件紀錄", "history") + "<hr>" + item("設定", "settings", "Ctrl+,") + item("返回 small", "small") + item("隱藏視窗", "hide"))}<div class="native-drag" data-tauri-drag-region></div>${controls()}</div><div class="terminal-screen">${minimal ? mini : ""}<pre class="console-output${minimal ? " console-minimal-log" : ""}"></pre><form class="console-command" autocomplete="off"><span>PS C:\Work&gt;</span><input aria-label="終端機模式指令" spellcheck="false" autocomplete="off" autocapitalize="off"></form></div></section>`;
  const groups = s.groups?.length
    ? s.groups
    : [{ name: "自選1", codes: s.watchlist }];
  const activeGroup = Math.max(
    0,
    Math.min(s.activeGroup ?? 0, groups.length - 1),
  );
  const groupMenu = menu(
    "分組",
    groups
      .map(
        (g, i) =>
          `<button data-note-group="${i}" role="menuitem"><span>${esc(g.name)}</span><small>${i === activeGroup ? "✓" : ""}</small></button>`,
      )
      .join(""),
  );
  return `<section class="native-frame text-view windows-notepad" data-disguise="txt" data-row-mode="${s.settings.rowDisplay ?? "numbers"}"><div class="native-title notepad-titlebar" data-tauri-drag-region><span class="notepad-icon" aria-hidden="true"></span><div class="notepad-tab"><span>工作紀錄.txt</span><button data-skin-action="hide" aria-label="關閉分頁">×</button></div><button class="native-add" data-skin-action="manage" aria-label="新增監看項目" title="新增監看項目">＋</button><div class="native-drag" data-tauri-drag-region></div>${controls()}</div><div class="notepad-menubar">${menu("檔案", item("自選清單…", "manage") + item("提醒設定…", "rules") + "<hr>" + item("返回 small", "small") + item("關閉視窗", "hide"))}${menu("編輯", '<button data-note-edit="select-all" role="menuitem"><span>全選</span><small>Ctrl+A</small></button><button data-note-edit="copy" role="menuitem"><span>複製</span><small>Ctrl+C</small></button>')}${menu("檢視", '<button data-zoom="in" role="menuitem"><span>放大</span><small>Ctrl++</small></button><button data-zoom="out" role="menuitem"><span>縮小</span><small>Ctrl+-</small></button><button data-zoom="reset" role="menuitem"><span>還原預設縮放</span><small>Ctrl+0</small></button><hr><button data-note-edit="status" role="menuitemcheckbox" aria-checked="true"><span>狀態列</span></button>' + item("事件紀錄", "history"))}${groupMenu}<div class="notepad-menu-space"></div><button class="native-settings" data-skin-action="settings" aria-label="設定" title="設定">⚙︎</button></div>${minimal ? mini : '<div class="notepad-editor notepad-fields" tabindex="0" aria-label="工作紀錄文字內容"></div>'}<div class="notepad-status"><span class="caret-status">第 1 行，第 1 欄</span><span class="character-status"></span><span class="zoom-status">100%</span><span>Windows (CRLF)</span><span>UTF-8</span></div></section>`;
}
function closeMenus(root: HTMLElement) {
  root
    .querySelectorAll<HTMLDetailsElement>("details[open]")
    .forEach((d) => (d.open = false));
}
function caret(root: HTMLElement) {
  const status = root.querySelector(".caret-status");
  if (!status) return;
  const editor = root.querySelector<HTMLElement>(".notepad-editor");
  const copy = root.querySelector<HTMLButtonElement>('[data-note-edit="copy"]');
  let before = "",
    collapsed = true;
  const selection = document.getSelection();
  if (
    editor &&
    selection &&
    selection.rangeCount > 0 &&
    selection.anchorNode &&
    editor.contains(selection.anchorNode)
  ) {
    collapsed = selection.isCollapsed;
    const range = document.createRange();
    range.selectNodeContents(editor);
    range.setEnd(selection.anchorNode, selection.anchorOffset);
    before = range.toString();
  }
  if (copy) copy.disabled = collapsed;
  status.textContent =
    `第 ${before.split("\n").length} 行，第 ${before.length - (before.lastIndexOf("\n") + 1) + 1} 欄`;
}
function setZoom(root: HTMLElement) {
  const editor = root.querySelector<HTMLElement>(".notepad-editor");
  if (editor) editor.style.fontSize = `${(13 * zoom) / 100}px`;
  const minimal = root.querySelector<HTMLElement>(".text-view .minimal-native");
  if (minimal) minimal.style.fontSize = `${(13 * zoom) / 100}px`;
  const status = root.querySelector(".zoom-status");
  if (status) status.textContent = `${zoom}%`;
}
export function bindDisguise(
  root: HTMLElement,
  s: Snapshot,
  onAction: SkinHandler,
  onStock?: (code: string) => void,
  onLayout?: () => void,
  onGroup?: (index: number) => void,
) {
  handler = onAction;
  lastState = s;
  relayout = onLayout ?? (() => {});
  if (s.settings.rowDisplay === "minimal")
    bindMinimal(root, s, (c) => onStock?.(c));
  root.querySelectorAll<HTMLButtonElement>("[data-note-group]").forEach(
    (b) =>
      (b.onclick = () => {
        closeMenus(root);
        onGroup?.(Number(b.dataset.noteGroup));
      }),
  );
  root.querySelectorAll<HTMLButtonElement>("[data-skin-action]").forEach(
    (b) =>
      (b.onclick = () => {
        closeMenus(root);
        onAction(b.dataset.skinAction as SkinAction);
      }),
  );
  root.querySelectorAll<HTMLDetailsElement>("details").forEach((d) =>
    d.addEventListener("toggle", () => {
      if (d.open)
        root
          .querySelectorAll<HTMLDetailsElement>("details")
          .forEach((other) => {
            if (other !== d) other.open = false;
          });
      if (lastState.settings.rowDisplay === "minimal") relayout();
    }),
  );
  root.querySelectorAll<HTMLButtonElement>("[data-note-edit]").forEach(
    (b) =>
      (b.onclick = async () => {
        closeMenus(root);
        const editor = root.querySelector<HTMLElement>(".notepad-editor");
        if (b.dataset.noteEdit === "status") {
          showStatus = !showStatus;
          root.querySelector<HTMLElement>(".notepad-status")!.hidden =
            !showStatus;
          b.setAttribute("aria-checked", String(showStatus));
          relayout();
          return;
        }
        if (!editor) {
          const list = root.querySelector(".minimal-list");
          if (!list) return;
          if (b.dataset.noteEdit === "select-all") {
            const range = document.createRange();
            range.selectNodeContents(list);
            const selection = document.getSelection();
            selection?.removeAllRanges();
            selection?.addRange(range);
          } else {
            const text =
              document.getSelection()?.toString() || list.textContent || "";
            try {
              await navigator.clipboard.writeText(text);
            } catch {
              document.execCommand("copy");
            }
          }
          return;
        }
        editor.focus();
        const selectAll = () => {
          const range = document.createRange();
          range.selectNodeContents(editor);
          const selection = document.getSelection();
          selection?.removeAllRanges();
          selection?.addRange(range);
        };
        if (b.dataset.noteEdit === "select-all") selectAll();
        else {
          const text =
            document.getSelection()?.toString() || notepadText(lastState);
          try {
            await navigator.clipboard.writeText(text);
          } catch {
            if (!document.getSelection()?.toString()) selectAll();
            document.execCommand("copy");
          }
        }
        caret(root);
      }),
  );
  root.querySelectorAll<HTMLButtonElement>("[data-zoom]").forEach(
    (b) =>
      (b.onclick = () => {
        zoom =
          b.dataset.zoom === "reset"
            ? 100
            : Math.max(
                50,
                Math.min(200, zoom + (b.dataset.zoom === "in" ? 10 : -10)),
              );
        closeMenus(root);
        setZoom(root);
        relayout();
      }),
  );
  const editor = root.querySelector<HTMLElement>(".notepad-editor");
  editor?.addEventListener("click", () => caret(root));
  editor?.addEventListener("keyup", () => caret(root));
  editor?.addEventListener("mouseup", () => caret(root));
  root
    .querySelector<HTMLFormElement>(".console-command")
    ?.addEventListener("submit", (event) => {
      event.preventDefault();
      const input = root.querySelector<HTMLInputElement>(
        ".console-command input",
      )!;
      const command = input.value.trim();
      input.value = "";
      if (!command) return;
      const actions: Record<string, SkinAction> = {
        watch: "watch",
        rules: "rules",
        history: "history",
        settings: "settings",
        small: "small",
        hide: "hide",
        pause: "pause",
        resume: "resume",
        list: "manage",
      };
      if (command === "clear" || command === "cls") commandLog = [];
      else if (command === "help")
        commandLog.push(
          `${shellPrompt} help`,
          "watch | list | rules | history | settings | small | pause | hide | clear",
        );
      else if (actions[command]) {
        onAction(actions[command]);
        return;
      } else
        commandLog.push(
          `${shellPrompt} ${command}`,
          "Unknown view command. Type help.",
        );
      commandLog = commandLog.slice(-6);
      updateDisguise(root, lastState);
    });
  root.onkeydown = (event) => {
    if (primaryPressed(event) && event.key === ",") {
      event.preventDefault();
      handler("settings");
    }
    if (event.key === "Escape") closeMenus(root);
    if (
      s.settings.appearance === "txt" &&
      primaryPressed(event) &&
      ["+", "=", "-", "0"].includes(event.key)
    ) {
      event.preventDefault();
      zoom =
        event.key === "0"
          ? 100
          : Math.max(50, Math.min(200, zoom + (event.key === "-" ? -10 : 10)));
      setZoom(root);
      relayout();
    }
  };
  updateDisguise(root, s);
}
export function updateDisguise(root: HTMLElement, s: Snapshot) {
  lastState = s;
  if (s.settings.rowDisplay === "minimal") {
    const info = root.querySelector<HTMLElement>(".minimal-native-demo");
    if (info) {
      info.hidden = !s.demo && !s.settings.paused;
      info.textContent =
        (s.demo ? "示範資料（非真實行情）" : "") +
        (s.settings.paused ? " 已暫停監看" : "");
    }
    updateMinimal(root, s);
    const output = root.querySelector<HTMLElement>(".console-output");
    if (output) output.textContent = commandLog.join("\n");
    const counter = root.querySelector(".character-status");
    if (counter)
      counter.textContent = `${root.querySelector(".minimal-list")?.textContent?.length ?? 0} 個字元`;
    const status = root.querySelector<HTMLElement>(".notepad-status");
    if (status) status.hidden = !showStatus;
    setZoom(root);
    relayout();
    return;
  }
  const output = root.querySelector<HTMLElement>(".console-output");
  if (output) {
    const selection = document.getSelection();
    if (
      selection &&
      !selection.isCollapsed &&
      selection.containsNode(output, true)
    )
      return;
    const text = terminalText(s);
    if (output.dataset.snapshot !== text) {
      output.innerHTML = terminalMarkup(s);
      output.dataset.snapshot = text;
    }
    return;
  }
  const editor = root.querySelector<HTMLElement>(".notepad-editor");
  if (!editor) return;
  const next = notepadText(s);
  const selection = document.getSelection();
  const selecting =
    !!selection &&
    !selection.isCollapsed &&
    !!selection.anchorNode &&
    editor.contains(selection.anchorNode);
  if (editor.dataset.snapshot !== next && !selecting) {
    const top = editor.scrollTop,
      left = editor.scrollLeft;
    editor.innerHTML = notepadMarkup(s);
    editor.dataset.snapshot = next;
    editor.scrollTop = top;
    editor.scrollLeft = left;
  }
  root.querySelector(".character-status")!.textContent =
    `${next.length} 個字元`;
  root.querySelector<HTMLElement>(".notepad-status")!.hidden = !showStatus;
  caret(root);
  setZoom(root);
}
