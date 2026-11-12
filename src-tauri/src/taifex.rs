//! TAIFEX quote adapter for the exchange's own futures — the one Taiwan
//! instrument neither TWSE MIS (cash market only) nor Yahoo (no Taiwan
//! futures at all) can serve. One POST to the futures MIS quote list with the
//! family code ("TXF") returns the spot reference row plus the main contract
//! of each listed month, nearest first; the adapter keeps the nearest traded
//! main contract and maps it onto core::Quote with the family code as the
//! symbol, so the stored watch code stays "TXF" across contract rolls. The
//! after-hours session (15:00–05:00) is a first-class citizen here: the
//! MarketType follows the Taipei clock, and the source time (CDate+CTime)
//! and the receive time are kept apart exactly as in the other adapters.

use crate::core::{index_name, local, now, Quote};
use chrono::{FixedOffset, NaiveDateTime, TimeZone, Timelike};
use serde_json::Value;

const UA: &str = "Mozilla/5.0 SmallStocks-TW";
const API: &str = "https://mis.taifex.com.tw/futures/api/getQuoteList";

/// Which MIS market a Taipei timestamp belongs to: "0" is the regular
/// session and the post-close lull after it (08:45–15:00 reads the day
/// session's final values), "1" is the after-hours session (15:00–05:00)
/// and the pre-open hours after it (05:00–08:45 reads the night session's
/// final values). Stale picks are harmless: the quote carries its own
/// CDate+CTime, and the freshness rule judges it by that.
pub fn market_type_at(at: i64) -> &'static str {
    let t = local(at);
    let m = t.hour() * 60 + t.minute();
    if (525..900).contains(&m) {
        "0"
    } else {
        "1"
    }
}
/// A string field that must parse to a finite number (the MIS spells every
/// number as a string, "" when absent).
fn str_num(v: &Value) -> Option<f64> {
    let n = v.as_str()?.trim().parse::<f64>().ok()?;
    n.is_finite().then_some(n)
}
/// A price field: a finite number above zero ("0" and "" both mean none).
fn pos(v: &Value) -> Option<f64> {
    str_num(v).filter(|n| *n > 0.)
}
/// A main-series contract of the family: TXF + month code (A=Jan..L=Dec) +
/// year digit + "-M" — "TXFK6-M". The spot row ("TXF-P") and the individual
/// non-main series do not match, by shape alone.
fn is_main_contract(family: &str, symbol_id: &str) -> bool {
    let Some(rest) = symbol_id.strip_prefix(family) else {
        return false;
    };
    let b = rest.as_bytes();
    b.len() == 4
        && (b'A'..=b'L').contains(&b[0])
        && b[1].is_ascii_digit()
        && &rest[2..] == "-M"
}
/// CDate (YYYYMMDD) + CTime (HHMMSS) are Taipei wall time; turn them into an
/// epoch-millis source stamp, the way parse_mis stamps its rows.
fn stamp(date: &str, time: &str) -> Option<i64> {
    if date.len() != 8 || !date.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let digits: String = time.chars().filter(|c| *c != ':').collect();
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    NaiveDateTime::parse_from_str(&format!("{date}{digits}"), "%Y%m%d%H%M%S")
        .ok()
        .and_then(|t| {
            FixedOffset::east_opt(28800)?
                .from_local_datetime(&t)
                .single()
        })
        .map(|t| t.timestamp_millis())
}
/// Parses one getQuoteList response for a family code into the near-month
/// quote. The list is ordered nearest month first, so the first main
/// contract that actually traded (a non-empty last price) is the front
/// month; an untraded near month falls through to the next one rather than
/// producing a price-less quote. A bad envelope or a list with nothing
/// traded parses to None, never to an invented price.
pub fn parse(v: &Value, code: &str, received: i64) -> Option<Quote> {
    if v["RtCode"].as_str() != Some("0") {
        return None;
    }
    let rows = v["RtData"]["QuoteList"].as_array()?;
    for r in rows {
        let Some(sid) = r["SymbolID"].as_str() else {
            continue;
        };
        if !is_main_contract(code, sid) {
            continue;
        }
        let Some(price) = pos(&r["CLastPrice"]) else {
            continue;
        };
        let date = r["CDate"].as_str().unwrap_or("");
        let Some(source_at) = stamp(date, r["CTime"].as_str().unwrap_or("")) else {
            continue;
        };
        let previous = pos(&r["CRefPrice"]);
        let bid = pos(&r["CBidPrice1"]);
        let ask = pos(&r["CAskPrice1"]);
        // Only a sane two-sided book gives a mid, recomputed here like the
        // other adapters compute it — an order price, never the trade price.
        let mid = bid
            .zip(ask)
            .filter(|(b, a)| b <= a)
            .map(|(b, a)| (b + a) / 2.);
        return Some(Quote {
            symbol: code.into(),
            name: index_name(code).map(str::to_owned).unwrap_or_default(),
            price: Some(price),
            bid,
            ask,
            previous,
            // The exchange's own rate, verified against its own diff
            // (CLastPrice − CRefPrice = CDiff); recomputed only if absent.
            change_percent: str_num(&r["CDiffRate"])
                .or_else(|| previous.map(|y| (price / y - 1.) * 100.)),
            mid,
            mid_change_percent: mid.zip(previous).map(|(m, y)| (m / y - 1.) * 100.),
            // CTime is the matching time of the last price, so the trade
            // time and the source time are the same stamp for this source.
            trade_at: Some(source_at),
            day_high: pos(&r["CHighPrice"]),
            day_low: pos(&r["CLowPrice"]),
            source_at,
            received_at: received,
            date: date.into(),
            source: "TAIFEX".into(),
        });
    }
    None
}
/// One TAIFEX fetch round: a POST per family code with the MarketType the
/// Taipei clock calls for. A code that fails everywhere is simply absent
/// when another produced a quote; only a round that produced nothing at
/// all is an error, the same contract as the Yahoo adapter.
pub fn fetch(
    client: &reqwest::blocking::Client,
    codes: &[String],
) -> Result<Vec<Quote>, String> {
    if codes.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for code in codes {
        let body = serde_json::json!({
            "MarketType": market_type_at(now()),
            "SymbolID": [code],
        });
        let round = client
            .post(API)
            .header("User-Agent", UA)
            .json(&body)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json::<Value>());
        match round {
            Ok(v) => {
                if let Some(q) = parse(&v, code, now()) {
                    out.push(q);
                }
            }
            Err(e) => errors.push(e.to_string()),
        }
    }
    if out.is_empty() {
        return Err(match errors.len() {
            0 => "期交所行情未回傳任何標的".into(),
            _ => format!("期交所行情連線失敗：{}", errors.join("；")),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{index_name, is_taifex_symbol, is_tw_symbol, symbol};
    use serde_json::json;

    /// The measured night-session shape (2026-11-13 03:22 Taipei): the spot
    /// reference row, then the main contracts nearest first — November
    /// (K6) traded at 43,655, December (L6) behind it.
    fn live_shape() -> Value {
        json!({"RtCode":"0","RtMsg":"","RtData":{"QuoteCount":"4140","QuoteList":[
            {"SymbolID":"TXF-P","DispCName":"臺指現貨","DispEName":"TXF",
             "CLastPrice":"","CRefPrice":"43091.87","CDiff":"0.00","CDiffRate":"0.00",
             "CDate":"20261112","CTime":""},
            {"SymbolID":"TXFK6-M","DispCName":"臺指期116","DispEName":"TX116",
             "CBidPrice1":"43657.00","CAskPrice1":"43664.00","CTotalVolume":"78727",
             "COpenPrice":"43180.00","CHighPrice":"44034.00","CLowPrice":"42371.00",
             "CLastPrice":"43655.00","CRefPrice":"43128.00",
             "CDiff":"527.00","CDiffRate":"1.22","CDate":"20261112","CTime":"032202"},
            {"SymbolID":"TXFL6-M","DispCName":"臺指期126","DispEName":"TX126",
             "CLastPrice":"43712.00","CRefPrice":"43160.00",
             "CDiff":"552.00","CDiffRate":"1.28","CDate":"20261112","CTime":"032143"}
        ]}})
    }
    #[test]
    fn parse_picks_the_near_month_and_maps_its_fields() {
        let q = parse(&live_shape(), "TXF", 1794425000000).unwrap();
        assert_eq!(q.symbol, "TXF", "the family code, not the contract code");
        assert_eq!(q.name, "臺指期貨");
        assert_eq!(q.price, Some(43655.), "November (K6), not December");
        assert_eq!(q.previous, Some(43128.), "the previous settlement");
        assert_eq!(q.change_percent, Some(1.22));
        assert_eq!(q.bid, Some(43657.));
        assert_eq!(q.ask, Some(43664.));
        assert_eq!(q.mid, Some(43660.5), "the book mid, recomputed");
        assert_eq!(q.day_high, Some(44034.));
        assert_eq!(q.day_low, Some(42371.));
        assert_eq!(q.date, "20261112", "the session's own date");
        // 2026-11-12 03:22:02 Taipei, from CDate+CTime.
        assert_eq!(q.source_at, 1794424922000);
        assert_eq!(q.trade_at, Some(1794424922000));
        assert_eq!(q.received_at, 1794425000000);
        assert_eq!(q.source, "TAIFEX");
    }
    #[test]
    fn parse_skips_the_spot_row_and_untraded_contracts() {
        let mut v = live_shape();
        // The near month has not traded: the next traded main contract wins.
        v["RtData"]["QuoteList"][1]["CLastPrice"] = json!("");
        let q = parse(&v, "TXF", 0).unwrap();
        assert_eq!(q.price, Some(43712.), "December steps up");
        assert_eq!(q.previous, Some(43160.));
        // Nothing traded at all: no quote, never the spot reference price.
        v["RtData"]["QuoteList"][2]["CLastPrice"] = json!("");
        assert!(parse(&v, "TXF", 0).is_none());
        let spot_only = json!({"RtCode":"0","RtData":{"QuoteList":[
            {"SymbolID":"TXF-P","CLastPrice":"43091.87","CRefPrice":"43091.87",
             "CDate":"20261112","CTime":"032202"}
        ]}});
        assert!(parse(&spot_only, "TXF", 0).is_none(), "TXF-P is not a contract");
    }
    #[test]
    fn parse_rejects_a_bad_envelope_without_panicking() {
        let mut v = live_shape();
        v["RtCode"] = json!("999");
        assert!(parse(&v, "TXF", 0).is_none());
        assert!(parse(&json!({}), "TXF", 0).is_none());
        assert!(parse(&json!({"RtCode":"0"}), "TXF", 0).is_none());
        assert!(parse(&json!({"RtCode":"0","RtData":{"QuoteList":null}}), "TXF", 0).is_none());
        assert!(parse(&json!("garbage"), "TXF", 0).is_none());
    }
    #[test]
    fn market_type_follows_the_taipei_session_boundaries() {
        // All on Thursday 2026-11-12, Taipei epoch seconds x1000.
        let cases: [(&str, i64, &str); 10] = [
            ("08:44 pre-open still reads the night session", 1794444240000, "1"),
            ("08:45 regular session opens", 1794444300000, "0"),
            ("13:45 regular close reads the day session", 1794462300000, "0"),
            ("14:59 post-close lull reads the day session", 1794466740000, "0"),
            ("15:00 after-hours opens", 1794466800000, "1"),
            ("00:00 inside the night session", 1794412800000, "1"),
            ("05:00 night close still reads the night session", 1794430800000, "1"),
            ("08:00 pre-open reads the night session", 1794441600000, "1"),
            ("10:00 mid regular session", 1794448800000, "0"),
            ("16:00 inside the night session", 1794470400000, "1"),
        ];
        for (label, at, want) in cases {
            assert_eq!(market_type_at(at), want, "{label}");
        }
    }
    #[test]
    fn txf_is_a_taifex_code_not_a_taiwan_or_yahoo_one() {
        assert_eq!(symbol("TXF").as_deref(), Some("TXF"));
        assert_eq!(symbol("txf").as_deref(), Some("TXF"), "uppercased first");
        assert!(is_taifex_symbol("TXF"));
        assert!(!is_taifex_symbol("MTX"), "MTX is not served yet");
        assert!(!is_tw_symbol("TXF"), "no digit: never the TWSE route");
        assert_eq!(index_name("TXF"), Some("臺指期貨"));
    }
}
