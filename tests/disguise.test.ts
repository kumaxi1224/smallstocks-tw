import { it, expect } from "vitest";
import { notepadText, notepadMarkup, disguiseMarkup } from "../src/disguise.ts";
import { terminalRows } from "../src/model.ts";
import type { Snapshot } from "../src/model.ts";
it("the Notepad report does not substitute an old live price for an unpublished close", () => {
  const s = {
    marketOpen: false,
    demo: false,
    watchlist: ["2330"],
    dailySummaries: {},
    quotes: { "2330": { name: "台積電", price: 999999 } },
    settings: { paused: false },
    events: [],
    lastFetch: 0,
  } as unknown as Snapshot;
  expect(notepadText(s)).toContain("等待收盤資料");
  expect(notepadText(s)).not.toContain("999,999");
  expect(notepadText(s)).toContain("—");
  s.dailySummaries = {
    "2330": {
      days: [{ date: "2026-11-06", close: 2135, changePercent: -4.69 }],
    },
  } as unknown as Snapshot["dailySummaries"];
  expect(notepadText(s)).toContain("2026/11/06 收盤");
  expect(notepadText(s)).toContain("2,135");
  s.dailySummaries["2330"]!.days[0]!.date = "2026-11-05";
  s.watchlist.push("0050");
  s.dailySummaries["0050"] = {
    days: [{ date: "2026-11-06", close: 92.5, changePercent: -4.79 }],
  } as Snapshot["dailySummaries"][string];
  expect(notepadText(s)).toContain("(11/05)");
});
it("plain-text skins mark an order-book price with ~ and never present it as a trade", () => {
  const at = Date.parse("2026-11-09T10:00:00+08:00");
  const s = {
    marketOpen: true,
    demo: false,
    now: at,
    watchlist: ["2330", "0050"],
    dailySummaries: {},
    quotes: {
      "2330": {
        name: "台積電",
        price: null,
        mid: 2237.5,
        midChangePercent: 4.8,
        changePercent: null,
        sourceAt: at,
      },
      "0050": {
        name: "元大台灣50",
        price: 92.5,
        changePercent: 1,
        sourceAt: at,
      },
    },
    settings: { paused: false },
    events: [],
    lastFetch: at,
  } as unknown as Snapshot;
  const text = notepadText(s);
  expect(text).toContain("~2,237.5");
  expect(text).not.toContain("~92.5");
  expect(terminalRows(s)).toContain("BOOK");
  expect(terminalRows(s)).toContain("OK");
});
it("the Notepad markup puts every row's value and change in fixed-width fields", () => {
  const at = Date.parse("2026-11-09T10:00:00+08:00");
  const s = {
    marketOpen: true,
    demo: false,
    now: at,
    watchlist: ["00928", "2330"],
    dailySummaries: {},
    quotes: {
      // A CJK + ASCII mixed name: the case that drifted under padding.
      "00928": {
        name: "中信上櫃ESG30",
        price: 23.1,
        changePercent: -5.44,
        sourceAt: at,
      },
      "2330": {
        name: "台積電",
        price: 2135,
        changePercent: 0.8,
        sourceAt: at,
      },
    },
    settings: { paused: false },
    events: [],
    lastFetch: at,
  } as unknown as Snapshot;
  const html = notepadMarkup(s);
  // Header and both rows carry the same field structure, so the columns
  // line up by construction regardless of the name's glyph widths.
  expect(html.match(/note-field value/g)).toHaveLength(3);
  expect(html.match(/note-field change/g)).toHaveLength(3);
  expect(html).toContain("中信上櫃ESG30");
  expect(html).toContain("23.1");
  expect(html).toContain("-5.44%");
  expect(html).toContain("2,135");
  // The plain-text twin still renders the same values.
  expect(notepadText(s)).toContain("中信上櫃ESG30");
});
it("the Windows Notepad skin lists the watch groups in a 分組 menu after 檢視", () => {
  const at = Date.parse("2026-11-09T10:00:00+08:00");
  const s = {
    marketOpen: true,
    demo: false,
    now: at,
    watchlist: ["2330"],
    dailySummaries: {},
    quotes: {
      "2330": { name: "台積電", price: 2135, changePercent: 0.8, sourceAt: at },
    },
    settings: { appearance: "txt", rowDisplay: "numbers", paused: false },
    groups: [
      { name: "ETF", codes: ["0050"] },
      { name: "個股", codes: ["2330"] },
    ],
    activeGroup: 1,
    events: [],
    lastFetch: at,
  } as unknown as Snapshot;
  const html = disguiseMarkup(s);
  expect(html.indexOf("檢視")).toBeLessThan(html.indexOf("分組"));
  expect(html).toContain('data-note-group="0"');
  expect(html).toContain('data-note-group="1"');
  // Only the active group carries the check mark.
  expect(html).toContain(
    '<button data-note-group="1" role="menuitem"><span>個股</span><small>✓</small></button>',
  );
  expect(html).toContain(
    '<button data-note-group="0" role="menuitem"><span>ETF</span><small></small></button>',
  );
  // The body is the field-span document, not a textarea.
  expect(html).toContain('class="notepad-editor notepad-fields"');
  expect(html).not.toContain("<textarea");
});
