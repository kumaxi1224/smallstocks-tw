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

const UA: &str = "Mozilla/5.0 SmallStocks-TW";

fn num(v: &Value) -> Option<f64> {
    let n = v.as_f64().or_else(|| v.as_str()?.parse().ok())?;
    (n.is_finite() && n > 0.).then_some(n)
}
/// Yahoo spells class shares with a dash (BRK-B); the app stores the dotted
/// form the user pastes (BRK.B), so translate on the way out.
fn yahoo_code(code: &str) -> String {
    code.replace('.', "-")
}
/// Batch quote URL for the spark endpoint: every symbol in one request, the ^
/// of an index percent-encoded, class-share dashes already translated.
pub fn spark_url(codes: &[String]) -> String {
    let symbols = codes
        .iter()
        .map(|c| yahoo_code(c))
        .collect::<Vec<_>>()
        .join(",")
        .replace('^', "%5E");
    format!(
        "https://query2.finance.yahoo.com/v7/finance/spark?symbols={symbols}&range=1d&interval=1m"
    )
}
/// Single-symbol chart URL, the batch fallback. Built by hand like spark_url:
/// reqwest's path-segment encoder leaves `^` raw, and Yahoo wants %5E.
pub fn chart_url(code: &str) -> String {
    let symbol = yahoo_code(code).replace('^', "%5E");
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
    fn urls_encode_index_carets_and_class_share_dashes() {
        let u = spark_url(&["^IXIC".into(), "BRK.B".into(), "VOO".into()]);
        assert!(u.contains("symbols=%5EIXIC,BRK-B,VOO"), "{u}");
        let c = chart_url("^GSPC");
        assert!(c.contains("/chart/%5EGSPC?"), "{c}");
        assert!(chart_url("BRK.B").contains("/chart/BRK-B?"));
    }
    #[test]
    fn an_error_envelope_or_empty_batch_parses_to_no_quotes() {
        let codes: Vec<String> = vec!["BRK.B".into()];
        let v = json!({"spark":{"result":null,"error":{"code":"Not Found","description":"No data found for spark symbols"}}});
        assert!(parse_spark(&v, &codes, 0).is_empty());
        assert!(parse_spark(&json!({}), &codes, 0).is_empty());
    }
}
