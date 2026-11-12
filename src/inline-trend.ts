import { esc } from "./model.ts";
import type { IntradayPoint, Snapshot } from "./model.ts";
export function line(
  points: IntradayPoint[],
  label: string,
  domain?: [number, number],
  gap = Infinity,
): string {
  const valid = points.filter(
    (p) => p.price != null && Number.isFinite(p.price),
  );
  if (valid.length < 2) return '<span class="trend-empty">待資料</span>';
  const prices = valid.map((p) => p.price!);
  const low = Math.min(...prices),
    high = Math.max(...prices),
    span = high - low;
  const w = 104,
    h = 16,
    pad = 1.5;
  let d = "",
    previous: number | null = null,
    last: [number, number] = [0, 0];
  points.forEach((p, i) => {
    if (p.price == null || !Number.isFinite(p.price)) {
      previous = null;
      return;
    }
    const fraction = domain
      ? (p.at - domain[0]) / (domain[1] - domain[0])
      : i / (points.length - 1);
    const x = pad + Math.max(0, Math.min(1, fraction)) * (w - 2 * pad),
      y = span ? h - pad - ((p.price - low) / span) * (h - 2 * pad) : h / 2;
    d += `${previous == null || p.at - previous > gap ? "M" : "L"}${x.toFixed(2)},${y.toFixed(2)} `;
    previous = p.at;
    last = [x, y];
  });
  return `<svg class="row-trend" viewBox="0 0 ${w} ${h}" role="img" aria-label="${esc(label)}"><path d="${d}"/><circle cx="${last[0]}" cy="${last[1]}" r="1.3"/></svg>`;
}
export function trendCell(
  s: Snapshot,
  code: string,
): { html: string; title: string } {
  if (!s.marketOpen && !s.demo) {
    const v = s.dailySummaries[code];
    const days = v?.days ?? [];
    const title = `最近 ${days.length} 個交易日收盤走勢（未還原價格）${days.length ? ` · ${days[0]!.date} 至 ${days.at(-1)!.date}` : ""}`;
    return {
      html: line(
        days.map((d, i) => ({ at: i, price: d.close })),
        title,
      ),
      title,
    };
  }
  const v = s.intraday?.[code],
    series = v?.series;
  const lastValueAt = series?.points.findLast((p) => p.price != null)?.at;
  const title = series
    ? `${series.source} · ${series.partial ? "局部觀測，並非全天" : "當天分鐘線，可能延遲"} · ${new Date(lastValueAt ?? series.fetchedAt).toLocaleTimeString("zh-TW", { hour12: false, timeZone: "Asia/Taipei" })}${v?.error ? " · 更新失敗" : ""}`
    : (v?.error ?? "正在取得當天分鐘線");
  if (!series)
    return { html: '<span class="trend-empty">待資料</span>', title };
  const start = series.points[0]?.at ?? s.now;
  const dayStart =
    Math.floor((start + 28800000) / 86400000) * 86400000 - 28800000;
  const domain: [number, number] = [
    dayStart + 9 * 3600000,
    dayStart + 13.5 * 3600000,
  ];
  const old = !s.demo && s.now - (lastValueAt ?? 0) > 120000;
  return {
    html: `<span class="line-slot${series.partial ? " partial" : ""}${old ? " delayed" : ""}">${line(series.points, title, domain, 90_000)}${series.partial ? '<span class="line-flag">局部</span>' : ""}</span>`,
    title,
  };
}
/**
 * The portfolio day chart: one aggregate TWD value line over the 09:00–13:30
 * session with a soft area fill and the previous-close total as a dashed
 * baseline. Pure geometry like `line` — the caller (the positions page)
 * supplies the aggregated points from model.portfolioIntradaySeries plus the
 * direction class, and renders all labels as HTML around the SVG so nothing
 * is distorted by the stretched viewBox.
 */
export function portfolioChart(
  pts: { at: number; value: number }[],
  baseline: number | null,
  signClass: "up" | "down" | "flat",
  label: string,
): string {
  if (pts.length < 2) return "";
  const w = 320,
    h = 148,
    padX = 2,
    padTop = 8,
    padBottom = 6;
  const first = pts[0]!.at;
  const dayStart =
    Math.floor((first + 28800000) / 86400000) * 86400000 - 28800000;
  const x0 = dayStart + 9 * 3600000,
    x1 = dayStart + 13.5 * 3600000;
  const values = pts.map((p) => p.value);
  let low = Math.min(...values),
    high = Math.max(...values);
  if (baseline != null) {
    low = Math.min(low, baseline);
    high = Math.max(high, baseline);
  }
  const span = high - low || Math.max(1, high * 0.001);
  low -= span * 0.08;
  high += span * 0.08;
  const X = (at: number) =>
    padX + Math.max(0, Math.min(1, (at - x0) / (x1 - x0))) * (w - 2 * padX);
  const Y = (v: number) =>
    padTop + (1 - (v - low) / (high - low)) * (h - padTop - padBottom);
  const coords = pts.map(
    (p) => [X(p.at), Y(p.value)] as [number, number],
  );
  const path = coords
    .map(([x, y], i) => `${i ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`)
    .join(" ");
  const area = `${path} L${coords.at(-1)![0].toFixed(1)},${h - padBottom} L${coords[0]![0].toFixed(1)},${h - padBottom} Z`;
  const baseLine =
    baseline != null
      ? `<line class="pf-base" x1="${padX}" x2="${w - padX}" y1="${Y(baseline).toFixed(1)}" y2="${Y(baseline).toFixed(1)}"/>`
      : "";
  const last = coords.at(-1)!;
  return `<svg class="pf-chart ${signClass}" viewBox="0 0 ${w} ${h}" preserveAspectRatio="none" role="img" aria-label="${esc(label)}"><title>${esc(label)}</title>${baseLine}<path class="pf-area" d="${area}"/><path class="pf-line" d="${path}"/><circle class="pf-dot" cx="${last[0].toFixed(1)}" cy="${last[1].toFixed(1)}" r="2.2"/></svg>`;
}
