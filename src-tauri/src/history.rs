//! Portfolio history: Yahoo daily closes per held Taiwan code, backcast over
//! the CURRENT holdings into one aggregate value series per range. Display
//! only — like intraday.rs, nothing here reaches ingest, rules or events,
//! and these are Yahoo's unadjusted daily bars, a separate path from the
//! official TWSE daily adapter in daily.rs (which feeds the close-day view
//! and keeps its own source discipline). One 10-year fetch per code feeds
//! every range; the core caches it for 30 minutes.

use crate::core::{local, now, symbol};
use chrono::{Datelike, FixedOffset, Months, NaiveDate};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct DayClose {
    /// Taipei trading date, "YYYY-MM-DD".
    pub date: String,
    pub close: f64,
}
#[derive(Clone, Debug)]
pub struct Series {
    pub fetched_at: i64,
    pub days: Vec<DayClose>,
}
/// The multi-day ranges of the portfolio chart, in tab order. 1D is the
/// intraday path and never comes through here.
pub const RANGES: [&str; 7] = ["5D", "1M", "6M", "YTD", "1Y", "5Y", "MAX"];

pub fn parse(v: &Value, yahoo_code: &str) -> Result<Vec<DayClose>, String> {
    let r = &v["chart"]["result"][0];
    if r["meta"]["symbol"].as_str() != Some(yahoo_code)
        || r["meta"]["dataGranularity"].as_str() != Some("1d")
    {
        return Err("歷史日線標的或取樣格式不符".into());
    }
    let stamps = r["timestamp"].as_array().ok_or("歷史日線沒有時間")?;
    let closes = r["indicators"]["quote"][0]["close"]
        .as_array()
        .ok_or("歷史日線沒有價格")?;
    if stamps.len() != closes.len() {
        return Err("歷史日線時間與價格長度不符".into());
    }
    // Yahoo pads non-trading rows with null closes; those days simply do not
    // exist for the series. A duplicated date keeps its last close.
    let mut by_date: BTreeMap<String, f64> = BTreeMap::new();
    for (stamp, close) in stamps.iter().zip(closes) {
        let Some(t) = stamp.as_i64().and_then(|t| t.checked_mul(1000)) else {
            continue;
        };
        let Some(close) = close.as_f64().filter(|n| n.is_finite() && *n > 0.) else {
            continue;
        };
        if t < 0 || t > now().saturating_add(86_400_000) {
            continue;
        }
        by_date.insert(local(t).format("%Y-%m-%d").to_string(), close);
    }
    let days: Vec<DayClose> = by_date
        .into_iter()
        .map(|(date, close)| DayClose { date, close })
        .collect();
    if days.len() < 2 {
        return Err("歷史日線不足".into());
    }
    Ok(days)
}
pub fn fetch(client: &reqwest::blocking::Client, code: &str) -> Result<Series, String> {
    if symbol(code).is_none() {
        return Err("無效的台股代號".into());
    }
    // Every Taiwan code — TWSE or TPEx — resolves as {code}.TW on Yahoo
    // (verified live: 5225.TW answers, 5225.TWO does not). The index keeps
    // the ^TWII spelling the intraday fetch uses.
    let yahoo_code = if code == "t00" {
        "^TWII".to_string()
    } else {
        format!("{code}.TW")
    };
    let mut url =
        reqwest::Url::parse("https://query2.finance.yahoo.com/v8/finance/chart/").unwrap();
    url.path_segments_mut()
        .unwrap()
        .pop_if_empty()
        .push(&yahoo_code.replace('^', "%5E"));
    url.query_pairs_mut()
        .append_pair("range", "10y")
        .append_pair("interval", "1d");
    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("歷史日線連線失敗：{e}"))?;
    let v: Value = response
        .error_for_status()
        .and_then(|r| r.json())
        .map_err(|e| format!("歷史日線來源錯誤：{e}"))?;
    Ok(Series {
        fetched_at: now(),
        days: parse(&v, &yahoo_code)?,
    })
}
pub fn demo_series(code: &str) -> Series {
    let at = now();
    let mut seed: u32 = 2166136261;
    for b in code.bytes() {
        seed = (seed ^ b as u32).wrapping_mul(16777619);
    }
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed % 10000) as f64 / 10000.
    };
    let mut dates = Vec::new();
    let mut d = local(at).date_naive();
    while dates.len() < 2400 {
        if d.weekday().number_from_monday() <= 5 {
            dates.push(d);
        }
        d -= chrono::Duration::days(1);
    }
    dates.reverse();
    let mut price = 20. + next() * 30.;
    let days = dates
        .iter()
        .map(|d| {
            price *= 1. + (next() - 0.47) * 0.012;
            DayClose {
                date: d.format("%Y-%m-%d").to_string(),
                close: price,
            }
        })
        .collect();
    Series {
        fetched_at: at,
        days,
    }
}
/// One holding's contribution to the backcast: its current total shares and
/// its daily closes, borrowed from the core cache. A leg whose history could
/// not be fetched at all carries None: it adds nothing to any point and is
/// counted uncovered, the same honesty as the day chart's missing samples.
pub struct Leg<'a> {
    pub shares: f64,
    pub days: Option<&'a [DayClose]>,
}
pub struct Aggregate {
    /// (Taipei date, aggregate value) points, oldest first.
    pub points: Vec<(String, f64)>,
    /// The first point's value; multi-day ranges draw it as the baseline.
    pub baseline: Option<f64>,
    pub uncovered: usize,
}
#[derive(Clone, Copy, PartialEq)]
enum Granularity {
    Daily,
    Weekly,
    Monthly,
}
fn bucket_key(date: &str, gran: Granularity) -> String {
    match gran {
        Granularity::Daily => date.to_string(),
        Granularity::Monthly => date[..7].to_string(),
        Granularity::Weekly => {
            let d = NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap();
            let iso = d.iso_week();
            format!("{}-W{:02}", iso.year(), iso.week())
        }
    }
}
/// Window start + bucket size for a range, anchored on the last trading
/// date present in the union of the legs' calendars.
fn range_spec(range: &str, end: NaiveDate, grid: &[String]) -> (String, Granularity) {
    let minus = |months: u32| {
        end.checked_sub_months(Months::new(months))
            .unwrap_or(end)
            .format("%Y-%m-%d")
            .to_string()
    };
    match range {
        "5D" => (
            grid[grid.len().saturating_sub(5)..]
                .first()
                .cloned()
                .unwrap_or_default(),
            Granularity::Daily,
        ),
        "1M" => (minus(1), Granularity::Daily),
        "6M" => (minus(6), Granularity::Weekly),
        "YTD" => (format!("{}-01-01", end.year()), Granularity::Weekly),
        "1Y" => (minus(12), Granularity::Weekly),
        "5Y" => (minus(60), Granularity::Monthly),
        _ => (
            grid.first().cloned().unwrap_or_default(),
            Granularity::Monthly,
        ),
    }
}
/// Backcasts the current holdings over daily closes: at each bucket's last
/// trading day, every leg counts shares × its own latest close at or before
/// that day (forward-filled across its gaps), and a leg contributes 0 before
/// its first available day — a holding that listed later simply was not
/// there yet; that is the product's chosen semantics, not missing data.
/// Leading all-zero points (no leg had data yet) are dropped so the line
/// starts when the backcast portfolio first exists.
pub fn aggregate(legs: &[Leg], range: &str) -> Aggregate {
    let uncovered = legs
        .iter()
        .filter(|l| l.days.is_none_or(|d| d.is_empty()))
        .count();
    let live: Vec<&Leg> = legs
        .iter()
        .filter(|l| l.days.is_some_and(|d| !d.is_empty()))
        .collect();
    if live.is_empty() {
        return Aggregate {
            points: Vec::new(),
            baseline: None,
            uncovered,
        };
    }
    let mut grid: Vec<String> = Vec::new();
    for leg in &live {
        for d in leg.days.unwrap() {
            grid.push(d.date.clone());
        }
    }
    grid.sort();
    grid.dedup();
    let end = NaiveDate::parse_from_str(grid.last().unwrap(), "%Y-%m-%d").unwrap();
    let (start, gran) = range_spec(range, end, &grid);
    // One point per bucket, at the bucket's last union trading day.
    let mut reps: Vec<&String> = Vec::new();
    for d in grid.iter().filter(|d| d.as_str() >= start.as_str()) {
        match reps.last_mut() {
            Some(last) if bucket_key(last, gran) == bucket_key(d, gran) => {
                *last = d;
            }
            _ => reps.push(d),
        }
    }
    let value_at = |date: &str| -> f64 {
        live.iter()
            .map(|leg| {
                let days = leg.days.unwrap();
                let idx = days.partition_point(|d| d.date.as_str() <= date);
                if idx == 0 {
                    0.
                } else {
                    leg.shares * days[idx - 1].close
                }
            })
            .sum()
    };
    let mut points: Vec<(String, f64)> = reps
        .iter()
        .map(|d| (d.to_string(), value_at(d)))
        .collect();
    let first_live = points
        .iter()
        .position(|(_, v)| *v > 0.)
        .unwrap_or(points.len());
    points.drain(..first_live);
    let baseline = points.first().map(|(_, v)| *v);
    Aggregate {
        points,
        baseline,
        uncovered,
    }
}
/// Milliseconds of a Taipei date's midnight, the `at` the frontend plots.
pub fn date_ms(date: &str) -> i64 {
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .unwrap_or_else(|_| local(now()).date_naive());
    d.and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(FixedOffset::east_opt(8 * 3600).unwrap())
        .unwrap()
        .timestamp_millis()
}
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    fn days(rows: &[(&str, f64)]) -> Vec<DayClose> {
        rows.iter()
            .map(|(date, close)| DayClose {
                date: date.to_string(),
                close: *close,
            })
            .collect()
    }
    fn leg(shares: f64, rows: &[(&str, f64)]) -> Leg<'static> {
        // Leak is fine in tests: Leg borrows, and each test builds its own.
        let days = Box::leak(Box::new(days(rows)));
        Leg {
            shares,
            days: Some(days.as_slice()),
        }
    }
    #[test]
    fn sums_shares_times_close_and_forward_fills() {
        let a = leg(
            100.,
            &[("2026-11-09", 10.), ("2026-11-11", 12.), ("2026-11-13", 13.)],
        );
        let b = leg(50., &[("2026-11-09", 20.), ("2026-11-13", 22.)]);
        let agg = aggregate(&[a, b], "1M");
        // Union grid: 09, 11, 13. B forward-fills 20 across the 11th.
        assert_eq!(
            agg.points,
            vec![
                ("2026-11-09".to_string(), 100. * 10. + 50. * 20.),
                ("2026-11-11".to_string(), 100. * 12. + 50. * 20.),
                ("2026-11-13".to_string(), 100. * 13. + 50. * 22.),
            ]
        );
        assert_eq!(agg.baseline, Some(2000.));
        assert_eq!(agg.uncovered, 0);
    }
    #[test]
    fn unlisted_segment_contributes_zero_then_joins() {
        let old = leg(
            10.,
            &[("2026-11-02", 5.), ("2026-11-09", 6.), ("2026-11-13", 7.)],
        );
        let new = leg(10., &[("2026-11-09", 100.), ("2026-11-13", 110.)]);
        let agg = aggregate(&[old, new], "1M");
        assert_eq!(agg.points[0], ("2026-11-02".to_string(), 50.));
        assert_eq!(agg.points[1], ("2026-11-09".to_string(), 60. + 1000.));
        assert_eq!(agg.points[2], ("2026-11-13".to_string(), 70. + 1100.));
    }
    #[test]
    fn series_starts_at_the_first_available_date() {
        // The union grid comes from the legs themselves, so a book whose
        // only holding listed on the 10th has no points before the 10th and
        // its baseline is that first day's value.
        let only = leg(10., &[("2026-11-10", 3.), ("2026-11-13", 4.)]);
        let agg = aggregate(&[only], "1M");
        assert_eq!(agg.points.len(), 2);
        assert_eq!(agg.points[0].0, "2026-11-10");
        assert_eq!(agg.baseline, Some(30.));
    }
    #[test]
    fn missing_history_counts_uncovered_and_is_excluded() {
        let a = leg(100., &[("2026-11-12", 10.), ("2026-11-13", 11.)]);
        let gone = Leg {
            shares: 500.,
            days: None,
        };
        let empty = Leg {
            shares: 5.,
            days: Some(&[]),
        };
        let agg = aggregate(&[a, gone, empty], "1M");
        assert_eq!(agg.uncovered, 2);
        assert_eq!(agg.points.len(), 2);
        assert_eq!(agg.points[1].1, 1100.);
    }
    #[test]
    fn weekly_and_monthly_buckets_take_the_last_trading_day() {
        let rows: Vec<(&str, f64)> = vec![
            ("2026-11-02", 1.),
            ("2026-11-04", 2.),
            ("2026-11-06", 3.),
            ("2026-11-09", 4.),
            ("2026-11-13", 5.),
        ];
        let weekly = aggregate(&[leg(10., &rows)], "6M");
        assert_eq!(
            weekly.points,
            vec![
                ("2026-11-06".to_string(), 30.),
                ("2026-11-13".to_string(), 50.),
            ]
        );
        let monthly = aggregate(&[leg(10., &rows)], "5Y");
        assert_eq!(monthly.points, vec![("2026-11-13".to_string(), 50.)]);
        let two_months: Vec<(&str, f64)> =
            vec![("2026-10-29", 1.), ("2026-10-30", 2.), ("2026-11-02", 3.)];
        let monthly = aggregate(&[leg(10., &two_months)], "5Y");
        assert_eq!(
            monthly.points,
            vec![
                ("2026-10-30".to_string(), 20.),
                ("2026-11-02".to_string(), 30.),
            ]
        );
    }
    #[test]
    fn range_windows_slice_by_calendar() {
        let rows: Vec<(&str, f64)> = (0..40)
            .map(|i| {
                let d = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
                    + chrono::Duration::days(i);
                (Box::leak(d.format("%Y-%m-%d").to_string().into_boxed_str()) as &str, 10.)
            })
            .collect();
        let five = aggregate(&[leg(1., &rows)], "5D");
        assert_eq!(five.points.len(), 5);
        // 1M back from 2026-11-09 starts 2026-10-09: Oct 1–8 fall out.
        let rows2: Vec<(&str, f64)> = vec![
            ("2026-10-05", 1.),
            ("2026-10-08", 1.),
            ("2026-10-09", 1.),
            ("2026-11-09", 1.),
        ];
        let one_m = aggregate(&[leg(1., &rows2)], "1M");
        assert_eq!(one_m.points.len(), 2);
        assert_eq!(one_m.points[0].0, "2026-10-09");
    }
    #[test]
    fn parse_skips_nulls_and_checks_symbol() {
        let v = serde_json::json!({"chart":{"result":[{"meta":{"symbol":"0050.TW","dataGranularity":"1d"},"timestamp":[1794432000,1794518400,1794604800],"indicators":{"quote":[{"close":[100.0,null,102.0]}]}}]}});
        let days = parse(&v, "0050.TW").unwrap();
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].close, 100.);
        assert_eq!(days[1].close, 102.);
        assert!(parse(&v, "0060.TW").is_err());
    }
    #[test]
    fn date_ms_is_taipei_midnight() {
        let expected = FixedOffset::east_opt(28800)
            .unwrap()
            .with_ymd_and_hms(2026, 11, 13, 0, 0, 0)
            .unwrap()
            .timestamp_millis();
        assert_eq!(date_ms("2026-11-13"), expected);
    }
}
