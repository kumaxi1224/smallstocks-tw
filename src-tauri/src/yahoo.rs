//! Yahoo Finance quote adapter for the symbols TWSE MIS and the Shioaji
//! sidecar cannot serve: US tickers and the ^ indices. One spark batch covers
//! every Yahoo-routed symbol; a per-symbol chart call is the fallback for
//! whatever the batch did not return. Unlike intraday.rs — whose Yahoo minutes
//! are display-only — these quotes are real monitoring input: they flow into
//! core's ingest like any other source, with the source time
//! (regularMarketTime) and the receive time kept apart exactly as parse_mis
//! keeps them. No login and no crumb: only the public spark/chart endpoints.

use crate::core::{index_name, local, now, Quote};
use serde_json::Value;
use std::collections::HashMap;

const UA: &str = "Mozilla/5.0 MaxiStocks";

fn num(v: &Value) -> Option<f64> {
    let n = v.as_f64().or_else(|| v.as_str()?.parse().ok())?;
    (n.is_finite() && n > 0.).then_some(n)
}
/// Yahoo spells class shares with a dash (BRK-B); the app stores the dotted
/// form the user pastes (BRK.B), so translate on the way out.
fn yahoo_code(code: &str) -> String {
    code.replace('.', "-")
}
/// Percent-encodes the two characters Yahoo's own spelling carries that a
/// query value cannot be trusted with raw: ^ (%5E) and the '=' of the =F/=X
/// suffixes (%3D). Yahoo currently tolerates a raw '=' inside the symbols
/// value, but %3D is the form that parses identically everywhere, so the
/// URLs are pinned to it.
fn encode_symbol(yahoo: String) -> String {
    yahoo.replace('^', "%5E").replace('=', "%3D")
}
/// Batch quote URL for the spark endpoint: every symbol in one request, each
/// percent-encoded by encode_symbol, class-share dashes already translated.
pub fn spark_url(codes: &[String]) -> String {
    let symbols = codes
        .iter()
        .map(|c| encode_symbol(yahoo_code(c)))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "https://query2.finance.yahoo.com/v7/finance/spark?symbols={symbols}&range=1d&interval=1m"
    )
}
/// Single-symbol chart URL, the batch fallback. Built by hand like spark_url:
/// reqwest's path-segment encoder leaves `^` raw, and Yahoo wants %5E.
pub fn chart_url(code: &str) -> String {
    let symbol = encode_symbol(yahoo_code(code));
    format!(
        "https://query2.finance.yahoo.com/v8/finance/chart/{symbol}?interval=1m&range=1d"
    )
}
/// One Quote from a spark/chart `meta` block. The change percent is recomputed
/// from price and previous close rather than trusted from the wire, the same
/// discipline as the other adapters; the previous close comes from
/// previousClose / chartPreviousClose, and only when both are absent is it
/// derived back out of Yahoo's own change percent.
fn quote_from_meta(code: &str, meta: &Value, received: i64) -> Option<Quote> {
    let price = num(&meta["regularMarketPrice"])?;
    let source_at = meta["regularMarketTime"]
        .as_i64()
        .filter(|t| *t > 0)?
        .checked_mul(1000)?;
    let previous = num(&meta["previousClose"])
        .or_else(|| num(&meta["chartPreviousClose"]))
        .or_else(|| {
            let pct = meta["regularMarketChangePercent"]
                .as_f64()
                .filter(|p| p.is_finite() && *p > -100.)?;
            Some(price / (1. + pct / 100.))
        });
    let name = index_name(code)
        .map(str::to_owned)
        .or_else(|| {
            meta["longName"]
                .as_str()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_owned)
        })
        .or_else(|| {
            meta["shortName"]
                .as_str()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_default();
    Some(Quote {
        symbol: code.into(),
        name,
        price: Some(price),
        bid: None,
        ask: None,
        previous,
        change_percent: previous.map(|y| (price / y - 1.) * 100.),
        mid: None,
        mid_change_percent: None,
        // regularMarketTime is the quote's own timestamp (source_at); Yahoo
        // does not separate a trade time from it, so no trade time is claimed.
        trade_at: None,
        day_high: num(&meta["regularMarketDayHigh"]),
        day_low: num(&meta["regularMarketDayLow"]),
        source_at,
        received_at: received,
        // Core judges freshness and the daily latch in the app's own (Taipei)
        // calendar for every source — MIS dates are Taipei trading dates — so
        // this date is the Taipei date of the source timestamp. An exchange-
        // local date would silently disable rules after Taipei midnight for
        // the second half of every US session.
        date: local(source_at).format("%Y%m%d").to_string(),
        source: "Yahoo".into(),
    })
}
/// Parses a spark batch response. Entries are matched back to the requested
/// codes by symbol (the batch answers in its own order and simply omits a
/// symbol it has no data for); a row without a usable price or source time is
/// skipped without failing the rest.
pub fn parse_spark(v: &Value, codes: &[String], received: i64) -> Vec<Quote> {
    let wanted: HashMap<String, String> = codes
        .iter()
        .map(|c| (yahoo_code(c), c.clone()))
        .collect();
    let rows = match v["spark"]["result"].as_array() {
        Some(rows) => rows,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for r in rows {
        let meta = &r["response"][0]["meta"];
        let Some(ysym) = meta["symbol"].as_str().or_else(|| r["symbol"].as_str()) else {
            continue;
        };
        let Some(code) = wanted.get(ysym) else {
            continue;
        };
        if let Some(q) = quote_from_meta(code, meta, received) {
            out.push(q);
        }
    }
    out
}
/// Parses one chart response for the batch fallback. The returned symbol must
/// be the one asked for; anything else is an error, not a quote.
pub fn parse_chart(v: &Value, code: &str, received: i64) -> Result<Quote, String> {
    let r = &v["chart"]["result"][0];
    let meta = &r["meta"];
    if meta["symbol"].as_str() != Some(yahoo_code(code).as_str()) {
        return Err("Yahoo 回傳的標的不符".into());
    }
    quote_from_meta(code, meta, received).ok_or_else(|| "Yahoo 報價不完整".into())
}
/// One Yahoo fetch round: the spark batch first, then a chart call for each
/// symbol the batch missed or could not serve. A symbol that fails everywhere
/// is simply absent (the caller keeps its previous quote); only a round that
/// produced nothing at all is an error.
pub fn fetch(
    client: &reqwest::blocking::Client,
    codes: &[String],
) -> Result<Vec<Quote>, String> {
    if codes.is_empty() {
        return Ok(Vec::new());
    }
    let batch = client
        .get(spark_url(codes))
        .header("User-Agent", UA)
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json::<Value>());
    let mut out = match batch {
        Ok(v) => parse_spark(&v, codes, now()),
        Err(_) => Vec::new(),
    };
    let missing: Vec<&String> = codes
        .iter()
        .filter(|c| !out.iter().any(|q| &q.symbol == *c))
        .collect();
    for (i, code) in missing.iter().enumerate() {
        if i > 0 {
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        let single = client
            .get(chart_url(code))
            .header("User-Agent", UA)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json::<Value>());
        if let Ok(v) = single {
            if let Ok(q) = parse_chart(&v, code, now()) {
                out.push(q);
            }
        }
    }
    if out.is_empty() {
        return Err("Yahoo 行情未回傳任何標的".into());
    }
    Ok(out)
}

/// Chart URL for completed daily bars: six months of 1d OHLC, the daily
/// input of the core's add-signal crossings for Yahoo-routed symbols (the
/// TWSE adapter in daily.rs is the official source for Taiwan codes and
/// stays the only daily path for them). Intraday minutes never enter here.
pub fn daily_bars_url(code: &str) -> String {
    let symbol = encode_symbol(yahoo_code(code));
    format!(
        "https://query2.finance.yahoo.com/v8/finance/chart/{symbol}?interval=1d&range=6mo"
    )
}
/// Parses one daily-bars chart response into daily::Day rows on the
/// exchange's own (US Eastern) trading dates. A bar is complete only once
/// its session has closed: today's Eastern bar is dropped before the
/// 16:00 ET close, and future bars never count — the same completeness
/// discipline daily::parse applies to official Taiwan data with its
/// 15:00 cutoff. Rows with missing or inconsistent OHLC are skipped, a
/// duplicated date keeps its last bar, and change is recomputed from the
/// closes rather than trusted from the wire.
pub fn parse_daily_bars(v: &Value, code: &str, at: i64) -> Result<Vec<crate::daily::Day>, String> {
    let r = &v["chart"]["result"][0];
    let meta = &r["meta"];
    if meta["symbol"].as_str() != Some(yahoo_code(code).as_str())
        || meta["dataGranularity"].as_str() != Some("1d")
    {
        return Err("Yahoo 日線標的或取樣格式不符".into());
    }
    let stamps = r["timestamp"].as_array().ok_or("Yahoo 日線沒有時間")?;
    let quote = &r["indicators"]["quote"][0];
    let column = |name: &str| quote[name].as_array().cloned().unwrap_or_default();
    let (opens, highs, lows, closes, volumes) = (
        column("open"),
        column("high"),
        column("low"),
        column("close"),
        column("volume"),
    );
    if [opens.len(), highs.len(), lows.len(), closes.len()]
        .iter()
        .any(|n| *n != stamps.len())
    {
        return Err("Yahoo 日線欄位長度不符".into());
    }
    let et_now = crate::core::eastern(at);
    let today = et_now.date_naive();
    let session_closed = et_now.time() >= chrono::NaiveTime::from_hms_opt(16, 0, 0).unwrap();
    let mut by_date: std::collections::BTreeMap<String, crate::daily::Day> =
        std::collections::BTreeMap::new();
    for i in 0..stamps.len() {
        let Some(t) = stamps[i].as_i64().and_then(|t| t.checked_mul(1000)) else {
            continue;
        };
        let date = crate::core::eastern(t).date_naive();
        if date > today || date == today && !session_closed {
            continue;
        }
        let pick = |col: &Vec<Value>| {
            col[i].as_f64().filter(|n| n.is_finite() && *n > 0.)
        };
        let (Some(open), Some(high), Some(low), Some(close)) = (
            pick(&opens),
            pick(&highs),
            pick(&lows),
            pick(&closes),
        ) else {
            continue;
        };
        if low > high || close < low || close > high || open < low || open > high {
            continue;
        }
        by_date.insert(
            date.format("%Y-%m-%d").to_string(),
            crate::daily::Day {
                date: date.format("%Y-%m-%d").to_string(),
                open,
                high,
                low,
                close,
                change: None,
                change_percent: None,
                volume: volumes.get(i).and_then(|n| n.as_f64()).filter(|n| *n >= 0.),
                volume_unit: String::new(),
            },
        );
    }
    let mut days: Vec<crate::daily::Day> = by_date.into_values().collect();
    for i in 1..days.len() {
        let previous = days[i - 1].close;
        let change = days[i].close - previous;
        days[i].change = Some(change);
        days[i].change_percent = Some(change / previous * 100.);
    }
    if days.len() < 2 {
        return Err("Yahoo 日線不足".into());
    }
    Ok(days)
}
/// One daily-bars fetch for the core's signal queue, wrapped as a
/// daily::Series so the core can hold it beside the official Taiwan
/// series without confusing the two sources.
pub fn fetch_daily_bars(
    client: &reqwest::blocking::Client,
    code: &str,
) -> Result<crate::daily::Series, String> {
    if crate::core::symbol(code).is_none() {
        return Err("無效的標的".into());
    }
    let at = now();
    let v: Value = client
        .get(daily_bars_url(code))
        .header("User-Agent", UA)
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json())
        .map_err(|e| format!("Yahoo 日線無法讀取：{e}"))?;
    Ok(crate::daily::Series {
        symbol: code.into(),
        month: String::new(),
        source: "Yahoo 日線".into(),
        fetched_at: at,
        days: parse_daily_bars(&v, code, at)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meta(symbol: &str, price: f64, at_s: i64) -> Value {
        json!({
            "symbol": symbol,
            "regularMarketPrice": price,
            "regularMarketTime": at_s,
            "regularMarketChangePercent": 1.5,
            "regularMarketDayHigh": price * 1.01,
            "regularMarketDayLow": price * 0.99,
            "longName": "Some Long Name",
            "previousClose": 100.,
            "chartPreviousClose": 100.,
        })
    }
    #[test]
    fn spark_rows_map_back_to_stored_codes_and_recompute_change() {
        // 2026-11-12 15:00 UTC = 10:00 EST = 23:00 Taipei.
        let at_s = 1794495600;
        let codes: Vec<String> = vec!["VOO".into(), "^IXIC".into(), "BRK.B".into()];
        let v = json!({"spark":{"result":[
            {"symbol":"BRK-B","response":[{"meta":meta("BRK-B", 588.24, at_s)}]},
            {"symbol":"^IXIC","response":[{"meta":meta("^IXIC", 25466.19, at_s)}]},
            {"symbol":"ZZZZ","response":[{"meta":meta("ZZZZ", 1., at_s)}]}
        ]}});
        let out = parse_spark(&v, &codes, at_s * 1000 + 500);
        assert_eq!(out.len(), 2, "an unrequested row is ignored, a missing one absent");
        let brk = out.iter().find(|q| q.symbol == "BRK.B").unwrap();
        assert_eq!(brk.price, Some(588.24));
        assert_eq!(brk.source, "Yahoo");
        assert_eq!(brk.received_at, at_s * 1000 + 500);
        let ix = out.iter().find(|q| q.symbol == "^IXIC").unwrap();
        // The built-in table wins over the wire's longName for a known index.
        assert_eq!(ix.name, "Nasdaq Composite");
        // previousClose 100 with price 25466.19 would be nonsense in production;
        // here it pins the recompute: change comes from price/previous, not 1.5.
        assert_eq!(ix.previous, Some(100.));
        assert!((ix.change_percent.unwrap() - (25466.19 / 100. - 1.) * 100.).abs() < 1e-9);
        assert_eq!(ix.source_at, at_s * 1000);
        assert_eq!(ix.date, "20261112", "the Taipei date of the source time");
        assert_eq!(ix.trade_at, None);
        assert!(ix.day_high.is_some() && ix.day_low.is_some());
    }
    #[test]
    fn previous_close_falls_back_to_chart_previous_then_to_the_percent() {
        let at_s = 1794495600;
        let mut m = meta("VOO", 110., at_s);
        m.as_object_mut().unwrap().remove("previousClose");
        let q = quote_from_meta("VOO", &m, 0).unwrap();
        assert_eq!(q.previous, Some(100.), "chartPreviousClose is next");
        m.as_object_mut().unwrap().remove("chartPreviousClose");
        let q = quote_from_meta("VOO", &m, 0).unwrap();
        // Only regularMarketChangePercent (1.5) remains: 110 / 1.015.
        assert!((q.previous.unwrap() - 110. / 1.015).abs() < 1e-9);
        assert!((q.change_percent.unwrap() - 1.5).abs() < 1e-9);
        // No price or no source time: no quote at all.
        let mut bad = meta("VOO", 110., at_s);
        bad.as_object_mut().unwrap().remove("regularMarketPrice");
        assert!(quote_from_meta("VOO", &bad, 0).is_none());
        let bad2 = meta("VOO", 110., 0);
        assert!(quote_from_meta("VOO", &bad2, 0).is_none());
    }
    #[test]
    fn chart_fallback_parses_and_rejects_a_mismatched_symbol() {
        let at_s = 1794495600;
        let v = json!({"chart":{"result":[{"meta":meta("^GSPC", 7619.38, at_s)}]}});
        let q = parse_chart(&v, "^GSPC", at_s * 1000).unwrap();
        assert_eq!(q.symbol, "^GSPC");
        assert_eq!(q.name, "S&P 500");
        assert_eq!(q.price, Some(7619.38));
        assert!(parse_chart(&v, "^IXIC", at_s * 1000).is_err());
        let empty = json!({"chart":{"result":null,"error":{"code":"Not Found"}}});
        assert!(parse_chart(&empty, "VOO", 0).is_err());
    }
    #[test]
    fn spark_rows_parse_futures_and_fx_codes_with_builtin_names() {
        // Meta blocks in the exact shape Yahoo answered with for BZ=F and
        // TWD=X (live check 2026-11-12): an encoded batch request returns
        // futures and FX rows alongside everything else, previousClose and
        // chartPreviousClose both present.
        let at_s = 1794494823;
        let codes: Vec<String> = vec!["BZ=F".into(), "TWD=X".into()];
        let bz = json!({
            "symbol": "BZ=F", "currency": "USD", "instrumentType": "FUTURE",
            "regularMarketPrice": 93.55, "regularMarketTime": at_s,
            "regularMarketChangePercent": 0.852,
            "regularMarketDayHigh": 94.36, "regularMarketDayLow": 91.19,
            "previousClose": 92.76, "chartPreviousClose": 92.76,
            "longName": "Brent Crude Oil Last Day Financial Futures",
        });
        let twd = json!({
            "symbol": "TWD=X", "currency": "TWD", "instrumentType": "CURRENCY",
            "regularMarketPrice": 32.967, "regularMarketTime": at_s,
            "regularMarketChangePercent": 0.3195,
            "previousClose": 32.862, "chartPreviousClose": 32.862,
            "longName": "USD/TWD",
        });
        let v = json!({"spark":{"result":[
            {"symbol":"TWD=X","response":[{"meta":twd}]},
            {"symbol":"BZ=F","response":[{"meta":bz}]}
        ]}});
        let out = parse_spark(&v, &codes, at_s * 1000);
        assert_eq!(out.len(), 2);
        let bz = out.iter().find(|q| q.symbol == "BZ=F").unwrap();
        assert_eq!(bz.name, "布蘭特原油", "the built-in table wins over the wire name");
        assert_eq!(bz.price, Some(93.55));
        assert_eq!(bz.previous, Some(92.76));
        assert!((bz.change_percent.unwrap() - (93.55 / 92.76 - 1.) * 100.).abs() < 1e-9);
        assert_eq!(bz.day_high, Some(94.36));
        assert_eq!(bz.source, "Yahoo");
        let twd = out.iter().find(|q| q.symbol == "TWD=X").unwrap();
        assert_eq!(twd.name, "美元／台幣");
        assert_eq!(twd.price, Some(32.967));
        assert_eq!(twd.previous, Some(32.862));
    }
    #[test]
    fn urls_encode_carets_equals_and_class_share_dashes() {
        let u = spark_url(&["^IXIC".into(), "BRK.B".into(), "VOO".into()]);
        assert!(u.contains("symbols=%5EIXIC,BRK-B,VOO"), "{u}");
        // The '=' of a futures/FX suffix must reach Yahoo as %3D, in the
        // batch and in the chart fallback alike.
        let f = spark_url(&["BZ=F".into(), "TWD=X".into()]);
        assert!(f.contains("symbols=BZ%3DF,TWD%3DX"), "{f}");
        assert!(chart_url("BZ=F").contains("/chart/BZ%3DF?"));
        assert!(chart_url("TWD=X").contains("/chart/TWD%3DX?"));
        let c = chart_url("^GSPC");
        assert!(c.contains("/chart/%5EGSPC?"), "{c}");
        assert!(chart_url("BRK.B").contains("/chart/BRK-B?"));
    }
    fn bars_chart(symbol: &str, rows: &[(i64, f64, f64, f64, f64)]) -> Value {
        // rows: (unix seconds at the bar's Eastern midnight, open, high, low, close)
        json!({"chart":{"result":[{"meta":{"symbol":symbol,"dataGranularity":"1d"},
            "timestamp": rows.iter().map(|r| r.0).collect::<Vec<_>>(),
            "indicators":{"quote":[{
                "open": rows.iter().map(|r| r.1).collect::<Vec<_>>(),
                "high": rows.iter().map(|r| r.2).collect::<Vec<_>>(),
                "low": rows.iter().map(|r| r.3).collect::<Vec<_>>(),
                "close": rows.iter().map(|r| r.4).collect::<Vec<_>>(),
                "volume": rows.iter().map(|_| 1000).collect::<Vec<_>>(),
            }]}}]}})
    }
    #[test]
    fn daily_bars_parse_on_eastern_dates_with_recomputed_change() {
        // 2026-11-10/11/12 00:00 EST = 05:00 UTC.
        let rows = [
            (1794286800, 100., 103., 99., 102.),
            (1794373200, 102., 104., 100., 101.),
            (1794459600, 101., 105., 100., 104.),
        ];
        let v = bars_chart("VOO", &rows);
        // Evaluated 2026-11-13 17:00 UTC: every bar is a completed session.
        let days = parse_daily_bars(&v, "VOO", 1794589200000).unwrap();
        assert_eq!(days.len(), 3);
        assert_eq!(days[0].date, "2026-11-10");
        assert_eq!(days[2].date, "2026-11-12");
        assert_eq!(days[0].low, 99.);
        assert_eq!(days[1].change, Some(-1.));
        assert!(parse_daily_bars(&v, "QQQ", 1794589200000).is_err());
    }
    #[test]
    fn daily_bars_drop_the_in_progress_eastern_bar_until_the_close() {
        let rows = [
            (1794200400, 99., 101., 98., 100.),
            (1794286800, 100., 103., 99., 102.),
            (1794373200, 102., 104., 100., 101.),
        ];
        let v = bars_chart("VOO", &rows);
        // 2026-11-11 16:30 UTC = 11:30 ET: the 11th's bar is still forming.
        let days = parse_daily_bars(&v, "VOO", 1794414600000).unwrap();
        assert_eq!(days.len(), 2);
        assert_eq!(days[1].date, "2026-11-10");
        // 2026-11-11 21:30 UTC = 16:30 ET: the session has closed.
        let days = parse_daily_bars(&v, "VOO", 1794432600000).unwrap();
        assert_eq!(days.len(), 3);
        assert_eq!(days[2].date, "2026-11-11");
    }
    #[test]
    fn an_error_envelope_or_empty_batch_parses_to_no_quotes() {
        let codes: Vec<String> = vec!["BRK.B".into()];
        let v = json!({"spark":{"result":null,"error":{"code":"Not Found","description":"No data found for spark symbols"}}});
        assert!(parse_spark(&v, &codes, 0).is_empty());
        assert!(parse_spark(&json!({}), &codes, 0).is_empty());
    }
}
