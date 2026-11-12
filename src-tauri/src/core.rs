use crate::calendar;
use crate::daily;
use crate::intraday;
use chrono::{Datelike, FixedOffset, NaiveDateTime, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf};

pub fn now() -> i64 {
    Utc::now().timestamp_millis()
}
pub fn local(ms: i64) -> chrono::DateTime<FixedOffset> {
    FixedOffset::east_opt(8 * 3600)
        .unwrap()
        .timestamp_millis_opt(ms)
        .single()
        .unwrap()
}
pub fn trading(ms: i64) -> bool {
    let t = local(ms);
    let m = t.hour() * 60 + t.minute();
    t.weekday().number_from_monday() <= 5 && (540..810).contains(&m)
}
/// The nth Sunday of a month, for the US daylight-saving boundaries below.
fn nth_sunday(year: i32, month: u32, n: u32) -> chrono::NaiveDate {
    let first = chrono::NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    let to_sunday = (7 - first.weekday().num_days_from_sunday()) % 7;
    first + chrono::Duration::days(to_sunday as i64 + 7 * (n as i64 - 1))
}
/// US Eastern UTC offset in seconds at an instant: EDT (UTC−4) from the second
/// Sunday of March 07:00 UTC until the first Sunday of November 06:00 UTC,
/// EST (UTC−5) otherwise (the post-2007 US rule).
fn eastern_offset_seconds(at: i64) -> i64 {
    let year = Utc.timestamp_millis_opt(at).unwrap().year();
    let edt_start = nth_sunday(year, 3, 2)
        .and_hms_opt(7, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();
    let edt_end = nth_sunday(year, 11, 1)
        .and_hms_opt(6, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();
    if (edt_start..edt_end).contains(&at) {
        -4 * 3600
    } else {
        -5 * 3600
    }
}
pub fn eastern(at: i64) -> chrono::DateTime<FixedOffset> {
    FixedOffset::east_opt(eastern_offset_seconds(at) as i32)
        .unwrap()
        .timestamp_millis_opt(at)
        .single()
        .unwrap()
}
/// Whether Yahoo's regularMarketTime can be advancing: a US weekday between the
/// 04:00 pre-market open and the 20:00 after-hours close, Eastern. There is no
/// US holiday calendar here; on a holiday the source timestamps simply go stale
/// and the freshness rule keeps alerts silent, the same failure shape as a
/// Taiwan closure the official calendar has not published yet.
pub fn us_market_open(at: i64) -> bool {
    let t = eastern(at);
    let m = t.hour() * 60 + t.minute();
    t.weekday().number_from_monday() <= 5 && (240..1200).contains(&m)
}
fn num(v: &Value) -> Option<f64> {
    let n = v.as_f64().or_else(|| v.as_str()?.parse().ok())?;
    (n.is_finite() && n > 0.).then_some(n)
}
/// Spreadsheet shorthands for the indices of the daily report, folded to the
/// Yahoo symbols the quote router understands. Bare spellings (IXIC, GSPC, …)
/// are included so a code copied back out of the app — which displays indices
/// without the ^ — round-trips to the same canonical symbol.
fn index_alias(s: &str) -> Option<&'static str> {
    Some(match s {
        "IX0001" | "TWII" | "^TWII" => "^TWII",
        "NI225" | "N225" | "^N225" => "^N225",
        "KOSPI" | "KS11" | "^KS11" => "^KS11",
        ".IXIC" | "IXIC" | "^IXIC" => "^IXIC",
        ".INX" | "INX" | "GSPC" | "^GSPC" => "^GSPC",
        ".DJI" | "DJI" | "^DJI" => "^DJI",
        "SOX" | "^SOX" => "^SOX",
        "RUT" | "^RUT" => "^RUT",
        _ => return None,
    })
}
/// Built-in display names for those same indices, plus the Yahoo futures and
/// FX codes users paste from the same sheet. Yahoo's own longName for the
/// indices is truncated or padded ("TSEC CAPITALIZATION WEIGHTED ST…"), so
/// this table wins wherever a Yahoo quote is parsed for one of them.
pub fn index_name(code: &str) -> Option<&'static str> {
    Some(match code {
        "^TWII" => "加權指數",
        "^N225" => "Nikkei 225",
        "^KS11" => "KOSPI",
        "^IXIC" => "Nasdaq Composite",
        "^SOX" => "PHLX Semiconductor Sector",
        "^DJI" => "Dow Jones Industrial Average",
        "^GSPC" => "S&P 500",
        "^RUT" => "Russell 2000 Index",
        "TXF" => "臺指期貨",
        "BZ=F" => "布蘭特原油",
        "CL=F" => "西德州原油",
        "GC=F" => "黃金",
        "TWD=X" => "美元／台幣",
        _ => return None,
    })
}
/// The quote route of a canonical symbol: Taiwan codes (they contain a digit
/// and carry no ^) come from TWSE MIS / the Shioaji sidecar; everything else
/// (US tickers, ^ indices, =F futures, =X exchange rates) comes from Yahoo.
/// t00/o00 are Taiwan index codes.
pub fn is_tw_symbol(code: &str) -> bool {
    !code.contains('=')
        && !code.starts_with('^')
        && code.bytes().any(|b| b.is_ascii_digit())
}
/// TAIFEX futures family codes, served by the exchange's own MIS (taifex.rs)
/// rather than TWSE MIS or Yahoo — Yahoo carries no Taiwan futures at all.
/// Letters only, so is_tw_symbol never claims them; the poll routes them to
/// the TAIFEX adapter before the Yahoo bucket. Only TXF is served for now:
/// the mini contract's MIS symbol resolution is unverified, so MTX stays a
/// plain (Yahoo-bound, unserved) ticker until it is.
pub fn is_taifex_symbol(code: &str) -> bool {
    matches!(code, "TXF")
}
/// Whether a TAIFEX session is running at `at` (Taipei): the regular session
/// 08:45–13:45 and the after-hours session 15:00–05:00, Monday to Friday,
/// the night session running into Saturday 05:00. Like the US calendar above
/// there is no holiday table here; on a holiday the source timestamps simply
/// go stale and the freshness rule keeps alerts silent.
pub fn taifex_market_open(at: i64) -> bool {
    let t = local(at);
    let m = t.hour() * 60 + t.minute();
    let weekday = t.weekday().number_from_monday();
    let regular = weekday <= 5 && (525..=825).contains(&m);
    let evening = weekday <= 5 && m >= 900;
    let early = (2..=6).contains(&weekday) && m < 300;
    regular || evening || early
}
/// How a stored symbol is written for the user: indices lose their ^ prefix.
pub fn display_symbol(code: &str) -> &str {
    code.strip_prefix('^').unwrap_or(code)
}
/// Canonical form of a user-typed code, shared by the UI and MCP through
/// execute. Taiwan codes keep their historic shapes (2330, 00685L, t00, o00);
/// index shorthands fold to Yahoo symbols (.IXIC → ^IXIC, IX0001 → ^TWII, …);
/// TAIFEX family codes keep their spelling (TXF); US tickers pass through
/// uppercased, class-share dot included (VOO, BRK.B);
/// Yahoo futures and FX codes keep their =F / =X suffix (BZ=F, TWD=X).
pub fn symbol(s: &str) -> Option<String> {
    let s = s.trim().to_uppercase();
    if matches!(s.as_str(), "大盤" | "加權" | "加權指數" | "T00") {
        return Some("t00".into());
    }
    if matches!(s.as_str(), "櫃買" | "櫃買指數" | "O00") {
        return Some("o00".into());
    }
    // A TAIFEX family code (TXF) is its own route, not a US ticker, even
    // though its letters-only shape would pass the ticker check below.
    if is_taifex_symbol(&s) {
        return Some(s);
    }
    if let Some(index) = index_alias(&s) {
        return Some(index.into());
    }
    // An unknown ^ symbol the user pasted in Yahoo's own spelling passes
    // through, so an index this table does not know still works.
    if let Some(body) = s.strip_prefix('^') {
        let valid = (1..=12).contains(&body.len())
            && body
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
        return valid.then(|| s.clone());
    }
    let tw = s.trim_end_matches(".TWO").trim_end_matches(".TW");
    let digits = tw.chars().take_while(|c| c.is_ascii_digit()).count();
    if (4..=6).contains(&digits)
        && (tw.len() == digits
            || tw.len() == digits + 1 && tw.as_bytes()[digits].is_ascii_uppercase())
    {
        return Some(tw.to_owned());
    }
    // A Yahoo future or FX code: an alphanumeric body carrying exactly one
    // =F (futures) or =X (exchange rate) suffix — BZ=F, CL=F, TWD=X, JPY=X.
    // Any other shape with an '=' in it falls through and is rejected below.
    if let Some(body) = s.strip_suffix("=F").or_else(|| s.strip_suffix("=X")) {
        let valid = (1..=6).contains(&body.len())
            && body
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
        return valid.then(|| s.clone());
    }
    // A US ticker: letters only, with at most one class-share suffix (BRK.B).
    let (head, tail) = match s.split_once('.') {
        Some((h, t)) => (h, Some(t)),
        None => (s.as_str(), None),
    };
    let valid = (1..=6).contains(&head.len())
        && head.bytes().all(|b| b.is_ascii_uppercase())
        && tail.is_none_or(|t| t.len() == 1 && t.bytes().all(|b| b.is_ascii_uppercase()));
    valid.then(|| s.clone())
}
pub fn mis_url(codes: &[String]) -> String {
    let channels: Vec<String> = codes
        .iter()
        .flat_map(|c| match c.as_str() {
            "t00" => vec!["tse_t00.tw".into()],
            "o00" => vec!["otc_o00.tw".into()],
            _ => vec![format!("tse_{c}.tw"), format!("otc_{c}.tw")],
        })
        .collect();
    let mut url =
        reqwest::Url::parse("https://mis.twse.com.tw/stock/api/getStockInfo.jsp").unwrap();
    url.query_pairs_mut()
        .append_pair("ex_ch", &channels.join("|"))
        .append_pair("json", "1")
        .append_pair("delay", "0")
        .append_pair("_", &now().to_string());
    url.into()
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Quote {
    pub symbol: String,
    pub name: String,
    pub price: Option<f64>,
    pub bid: Option<f64>,
    pub ask: Option<f64>,
    pub previous: Option<f64>,
    pub change_percent: Option<f64>,
    /// Average of the best bid and ask of the same snapshot. An order-book value, not a trade.
    pub mid: Option<f64>,
    pub mid_change_percent: Option<f64>,
    /// When the last trade happened (the source's own `trade` block), if it said.
    pub trade_at: Option<i64>,
    /// The day's highest and lowest trade price so far, from the same snapshot.
    pub day_high: Option<f64>,
    pub day_low: Option<f64>,
    pub source_at: i64,
    pub received_at: i64,
    pub date: String,
    pub source: String,
}
/// Stock snapshots from the source arrive about every one to two minutes, so a quote stays usable
/// for three minutes; older than that it is stale and cannot start or complete a crossing.
pub const FRESH_MS: i64 = 180_000;
pub fn parse_mis(v: &Value, received: i64) -> Result<Vec<Quote>, String> {
    if v["rtcode"].as_str() != Some("0000") {
        return Err("行情來源未回傳成功狀態".into());
    }
    let rows = v["msgArray"].as_array().ok_or("行情格式不符")?;
    let mut out = Vec::new();
    for r in rows {
        let Some(code) = r["c"].as_str().and_then(symbol) else {
            continue;
        };
        let date = r["d"].as_str().unwrap_or("");
        let time = r["t"].as_str().unwrap_or("");
        let stamp = NaiveDateTime::parse_from_str(&format!("{date} {time}"), "%Y%m%d %H:%M:%S")
            .ok()
            .and_then(|t| {
                FixedOffset::east_opt(28800)?
                    .from_local_datetime(&t)
                    .single()
            })
            .map(|t| t.timestamp_millis());
        let Some(stamp) = stamp else { continue };
        // Missing trade stays missing: order book and previous close never become a trade.
        // z is the trade price of this snapshot only and is "-" between trades. The official MIS
        // site then reads the last trade from the response's `trade` block; so do we. Only a real
        // trade ever fills `price`: the order book never does.
        let trade = &r["trade"];
        let price = num(&r["z"]).or_else(|| num(&trade["z"]));
        let trade_at = trade["t"]
            .as_str()
            .filter(|_| price.is_some())
            .and_then(|t| {
                NaiveDateTime::parse_from_str(&format!("{date} {t}"), "%Y%m%d %H:%M:%S").ok()
            })
            .and_then(|t| {
                FixedOffset::east_opt(28800)?
                    .from_local_datetime(&t)
                    .single()
            })
            .map(|t| t.timestamp_millis());
        let previous = num(&r["y"]);
        let level = |k: &str| {
            r[k].as_str()
                .and_then(|s| s.split('_').next())
                .and_then(|s| s.parse::<f64>().ok())
                .filter(|n| n.is_finite() && *n > 0.)
        };
        let bid = level("b");
        let ask = level("a");
        // Only a sane two-sided book gives a mid; a locked or empty side (limit up or down) gives none.
        let mid = bid
            .zip(ask)
            .filter(|(b, a)| b <= a)
            .map(|(b, a)| (b + a) / 2.);
        out.push(Quote {
            symbol: code,
            name: r["n"].as_str().unwrap_or("").into(),
            price,
            bid,
            ask,
            previous,
            change_percent: price.zip(previous).map(|(p, y)| (p / y - 1.) * 100.),
            mid,
            mid_change_percent: mid.zip(previous).map(|(m, y)| (m / y - 1.) * 100.),
            trade_at,
            day_high: num(&r["h"]),
            day_low: num(&r["l"]),
            source_at: stamp,
            received_at: received,
            date: date.into(),
            source: "TWSE MIS".into(),
        });
    }
    Ok(out)
}

/// Loopback URL of the local Shioaji sidecar for the given codes.
pub fn sidecar_url(port: u16, codes: &[String]) -> String {
    format!(
        "http://127.0.0.1:{port}/quotes?codes={}",
        codes.join(",")
    )
}
/// The sidecar writes its auth token next to the app's config as sidecar-token.txt
/// (the app's data dir). Returns None when the sidecar has never run there.
pub fn sidecar_token(dir: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("sidecar-token.txt")).ok()?;
    let token = text.trim().to_string();
    (!token.is_empty()).then_some(token)
}
/// Parses the sidecar's JSON into Quotes. Same discipline as parse_mis: the trade
/// price comes only from real ticks, the mid is recomputed here from a sane
/// two-sided book (never trusted from the wire), change percents are recomputed
/// from price/mid and previous close, and received_at is this app's own clock.
pub fn parse_sidecar(v: &Value, received: i64) -> Result<Vec<Quote>, String> {
    let rows = v["quotes"].as_array().ok_or("sidecar 行情格式不符")?;
    let mut out = Vec::new();
    for r in rows {
        let Some(code) = r["symbol"].as_str().and_then(symbol) else {
            continue;
        };
        // Without a source timestamp the quote can never be judged fresh; drop it.
        let Some(source_at) = r["sourceAt"].as_i64().filter(|t| *t > 0) else {
            continue;
        };
        let date = r["date"].as_str().unwrap_or("");
        if date.len() != 8 || !date.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let price = num(&r["price"]);
        let previous = num(&r["previous"]);
        let bid = num(&r["bid"]);
        let ask = num(&r["ask"]);
        let mid = bid
            .zip(ask)
            .filter(|(b, a)| b <= a)
            .map(|(b, a)| (b + a) / 2.);
        out.push(Quote {
            symbol: code,
            name: r["name"].as_str().unwrap_or("").into(),
            price,
            bid,
            ask,
            previous,
            change_percent: price.zip(previous).map(|(p, y)| (p / y - 1.) * 100.),
            mid,
            mid_change_percent: mid.zip(previous).map(|(m, y)| (m / y - 1.) * 100.),
            trade_at: r["tradeAt"].as_i64().filter(|_| price.is_some()),
            day_high: num(&r["dayHigh"]),
            day_low: num(&r["dayLow"]),
            source_at,
            received_at: received,
            date: date.into(),
            source: "Shioaji".into(),
        });
    }
    Ok(out)
}
/// Primary quotes win; fallback quotes only fill symbols the primary lacks.
pub fn fill_missing(primary: Vec<Quote>, fallback: Vec<Quote>) -> Vec<Quote> {
    let have: std::collections::HashSet<String> =
        primary.iter().map(|q| q.symbol.clone()).collect();
    let mut out = primary;
    out.extend(
        fallback
            .into_iter()
            .filter(|q| !have.contains(&q.symbol)),
    );
    out
}

/// How many state backups are kept in the backup folder; older ones are
/// pruned after each write.
pub const BACKUP_KEEP: usize = 14;
/// Backup files in `dir` (state-backup-*.json), oldest first — the
/// timestamped names sort chronologically.
fn backup_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("state-backup-") && n.ends_with(".json"))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}
/// The Bot API endpoint for a token. The result embeds the token: hand it
/// only to the HTTP client — never log it, store it in an error, or show
/// it in the UI.
fn telegram_endpoint(token: &str) -> String {
    format!("https://api.telegram.org/bot{token}/sendMessage")
}
/// Guard + payload for one Telegram send, shared by the settings-page test
/// command and the alert path in lib.rs. Errors name only what is missing —
/// never the token.
fn telegram_request(settings: &Settings, text: &str) -> Result<(String, Value), String> {
    let token = settings.telegram_bot_token.trim();
    if token.is_empty() {
        return Err("尚未設定 bot token".into());
    }
    let chat_id = settings.telegram_chat_id.trim();
    if chat_id.is_empty() {
        return Err("尚未設定 chat id".into());
    }
    Ok((
        telegram_endpoint(token),
        json!({"chat_id": chat_id, "text": text}),
    ))
}
/// POSTs one Telegram message with a blocking client (10s timeout, no
/// retry). Failure is reported by status class only: the request URL
/// carries the token, and reqwest's own error text can embed it, so the
/// underlying error is never formatted into the result.
pub fn telegram_send_blocking(settings: &Settings, text: &str) -> Result<(), String> {
    let (url, body) = telegram_request(settings, text)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|_| "無法建立連線".to_string())?;
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .map_err(|_| "Telegram 連線失敗".to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("Telegram 回應 HTTP {}", resp.status().as_u16()))
    }
}
/// The Telegram sends mirroring a batch of rule events: one (endpoint,
/// payload) per strip/card event, when the push is enabled and configured.
/// The endpoint embeds the token — callers must treat it as a secret and
/// stay silent on failure.
pub fn telegram_alerts(settings: &Settings, events: &[Event]) -> Vec<(String, Value)> {
    if !settings.telegram_enabled {
        return Vec::new();
    }
    events
        .iter()
        .filter(|e| matches!(e.channel.as_str(), "strip" | "card"))
        .filter_map(|e| telegram_request(settings, &format!("🔔 {}", e.text)).ok())
        .collect()
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default = "default_always_on_top")]
    pub always_on_top: bool,
    #[serde(default)]
    pub show_codes: bool,
    /// Hide stock names in list rows; only the code is shown.
    #[serde(default)]
    pub hide_names: bool,
    /// In numbers mode, show the point change next to the price as well as the percent.
    #[serde(default = "default_show_change_points")]
    pub show_change_points: bool,
    #[serde(default = "default_row_display")]
    pub row_display: String,
    pub appearance: String,
    pub paused: bool,
    pub refresh_seconds: u64,
    /// Live-quote source: "mis" (TWSE MIS snapshots) or "shioaji" (local Shioaji
    /// sidecar first, MIS fills whatever the sidecar did not return).
    #[serde(default = "default_quote_source")]
    pub quote_source: String,
    #[serde(default = "default_sidecar_port")]
    pub sidecar_port: u16,
    /// Opacity of the main window's shell background, 0.4–1.0 (1.0 = opaque).
    /// Only the background fades; text and numbers stay fully opaque.
    #[serde(default = "default_window_opacity")]
    pub window_opacity: f64,
    /// Telegram alert push: when a rule event would pop a local notice, also
    /// send it to the configured chat through the configured bot. The token
    /// is a secret: it lives only in state.json, is set only from the
    /// settings page (never through MCP), and never enters a snapshot —
    /// snapshots carry `telegramTokenSet` instead (see `settings_view`).
    #[serde(default)]
    pub telegram_enabled: bool,
    #[serde(default)]
    pub telegram_chat_id: String,
    #[serde(default)]
    pub telegram_bot_token: String,
    /// Automatic state.json backups into a user-chosen folder (typically a
    /// cloud-synced one); see `maybe_backup`.
    #[serde(default)]
    pub backup_enabled: bool,
    #[serde(default)]
    pub backup_dir: String,
    /// Show the auxiliary PTT chat window (a plain webview on the public
    /// t.me/s preview feed, no app capabilities). lib.rs creates/destroys
    /// the window to match; closing the window by hand writes this back
    /// to false, so the setting always says whether the window is up.
    #[serde(default)]
    pub chat_window: bool,
}
fn default_row_display() -> String {
    "numbers".into()
}
fn default_quote_source() -> String {
    "mis".into()
}
pub const DEFAULT_SIDECAR_PORT: u16 = 18765;
fn default_sidecar_port() -> u16 {
    DEFAULT_SIDECAR_PORT
}
fn default_always_on_top() -> bool {
    true
}
fn default_show_change_points() -> bool {
    true
}
pub const MIN_WINDOW_OPACITY: f64 = 0.4;
fn default_window_opacity() -> f64 {
    1.0
}
/// Clamp a window-opacity value into the supported range; non-finite input
/// falls back to fully opaque.
pub fn clamp_window_opacity(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(MIN_WINDOW_OPACITY, 1.0)
    } else {
        1.0
    }
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            always_on_top: true,
            show_codes: false,
            hide_names: false,
            show_change_points: true,
            row_display: default_row_display(),
            appearance: "small".into(),
            paused: false,
            refresh_seconds: 15,
            quote_source: default_quote_source(),
            sidecar_port: DEFAULT_SIDECAR_PORT,
            window_opacity: 1.0,
            telegram_enabled: false,
            telegram_chat_id: String::new(),
            telegram_bot_token: String::new(),
            backup_enabled: false,
            backup_dir: String::new(),
            chat_window: false,
        }
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: String,
    pub symbol: String,
    pub metric: String,
    pub direction: String,
    pub threshold: f64,
    pub channel: String,
    pub repeat: String,
    pub enabled: bool,
    #[serde(default)]
    pub last_value: Option<f64>,
    #[serde(default)]
    pub last_at: Option<i64>,
    #[serde(default)]
    pub last_day: Option<String>,
    /// The day high and low (in the rule's unit) at the last update, to notice a touch between two
    /// updates.
    #[serde(default)]
    pub last_high: Option<f64>,
    #[serde(default)]
    pub last_low: Option<f64>,
    #[serde(default)]
    pub fired_day: Option<String>,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: String,
    pub rule_id: String,
    pub symbol: String,
    pub text: String,
    pub source_at: i64,
    pub detected_at: i64,
    pub channel: String,
    pub value: f64,
    pub threshold: f64,
    /// Quote source that produced this event ("TWSE MIS" / "Shioaji"), so a
    /// notification never credits the wrong source. Empty on pre-existing events.
    #[serde(default)]
    pub source: String,
}
/// One named watchlist group （自選1、自選2…）. Monitoring covers the union of all groups;
/// the flat `watchlist` on Saved stays as a mirror of the active group's codes.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WatchGroup {
    pub name: String,
    #[serde(default)]
    pub codes: Vec<String>,
}
pub const MAX_WATCH_GROUPS: usize = 10;
pub const MAX_GROUP_SYMBOLS: usize = 30;
/// One purchase lot of a held position: when it was bought (YYYY-MM-DD, may be
/// empty when the user did not record it), the per-share price paid, and how
/// many shares. Shares are fractional-capable (US fractional shares, e.g. 9.43).
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Lot {
    #[serde(default)]
    pub date: String,
    pub price: f64,
    pub shares: f64,
}
/// A held position: one symbol and its purchase lots, in entry order. Position
/// codes join the monitoring union (see Saved::all_codes) so a holding that is
/// not in any watch group still gets quotes for its valuation.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub code: String,
    #[serde(default)]
    pub lots: Vec<Lot>,
}
/// One month's portfolio snapshot for the positions page's monthly tab:
/// the combined total (TWD equivalent) the app computed the first time it
/// ran that month with complete quotes, with the TWD/USD legs and the
/// conversion rate kept beside it so the figure stays auditable.
/// First write wins — a month is never overwritten once recorded.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MonthlySnapshot {
    /// "YYYY-MM".
    pub month: String,
    pub total_twd: f64,
    pub twd_value: f64,
    #[serde(default)]
    pub usd_value: Option<f64>,
    #[serde(default)]
    pub rate: Option<f64>,
    /// The Taipei date the total was actually captured, "YYYY-MM-DD"
    /// (the 1st itself may be a weekend or a day the app never ran).
    pub recorded_on: String,
}
#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    #[serde(default)]
    pub intraday_cache: HashMap<String, intraday::Series>,
    #[serde(default)]
    pub local_intraday: HashMap<String, intraday::Series>,
    #[serde(default)]
    pub daily_cache: HashMap<String, daily::Series>,
    /// Official TWSE open/closed calendar by year, kept so a restart on a holiday knows it.
    #[serde(default)]
    pub calendars: HashMap<String, calendar::Calendar>,
    #[serde(default)]
    pub watchlist: Vec<String>,
    /// Named groups; a pre-groups save has none and is migrated on load (see normalize_groups).
    #[serde(default)]
    pub groups: Vec<WatchGroup>,
    #[serde(default)]
    pub active_group: usize,
    /// Held positions with their purchase lots; empty on pre-positions saves.
    #[serde(default)]
    pub positions: Vec<Position>,
    /// Monthly portfolio snapshots (see MonthlySnapshot); empty on
    /// pre-monthly saves.
    #[serde(default)]
    pub monthly_snapshots: Vec<MonthlySnapshot>,
    /// Display names learned from live quotes (any source), keyed by canonical
    /// code. Quotes live only in memory, so without this cache a restart — or
    /// the close-day view, whose daily series carry no name — loses every name
    /// the intraday feed once provided. Empty on pre-cache saves.
    #[serde(default)]
    pub names: HashMap<String, String>,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub events: Vec<Event>,
}
impl Saved {
    /// Folds a pre-groups save (one flat watchlist) into a single group named 自選1, and keeps
    /// `watchlist` mirroring the active group's codes for every reader that predates groups.
    fn normalize_groups(&mut self) {
        if self.groups.is_empty() {
            let mut codes: Vec<String> = Vec::new();
            for c in self.watchlist.drain(..) {
                if !codes.contains(&c) {
                    codes.push(c);
                }
            }
            self.groups.push(WatchGroup {
                name: "自選1".into(),
                codes,
            });
        }
        if self.active_group >= self.groups.len() {
            self.active_group = self.groups.len() - 1;
        }
        self.sync_watchlist();
    }
    fn sync_watchlist(&mut self) {
        self.watchlist = self
            .groups
            .get(self.active_group)
            .map(|g| g.codes.clone())
            .unwrap_or_default();
    }
    /// Every watched code across all groups plus every held position, in group
    /// order then position order, duplicates removed: the set polling fetches
    /// and ingest judges rules against. A holding outside every group is still
    /// monitored so the portfolio page can value it.
    pub fn all_codes(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for g in &self.groups {
            for c in &g.codes {
                if !out.contains(c) {
                    out.push(c.clone());
                }
            }
        }
        for p in &self.positions {
            if !out.contains(&p.code) {
                out.push(p.code.clone());
            }
        }
        out
    }
    pub fn watches(&self, code: &str) -> bool {
        self.groups.iter().any(|g| g.codes.iter().any(|c| c == code))
            || self.positions.iter().any(|p| p.code == code)
    }
    fn default_group_name(&self) -> String {
        (1..)
            .map(|n| format!("自選{n}"))
            .find(|n| self.groups.iter().all(|g| &g.name != n))
            .unwrap()
    }
}
/// The `index` argument of the group commands, bounds-checked against the group count.
fn group_index(args: &Value, count: usize) -> Result<usize, String> {
    let i = args["index"].as_u64().ok_or("需要分組編號")? as usize;
    if i >= count {
        return Err("找不到分組".into());
    }
    Ok(i)
}
/// A user-supplied group name: trimmed, non-empty, at most 20 characters.
fn group_name(args: &Value) -> Result<String, String> {
    let name = args["name"].as_str().ok_or("需要分組名稱")?.trim();
    if name.is_empty() {
        return Err("分組名稱不可空白".into());
    }
    if name.chars().count() > 20 {
        return Err("分組名稱最多 20 字".into());
    }
    Ok(name.to_string())
}
/// A lot date the user typed: strict YYYY-MM-DD shape and a real calendar day.
/// A "YYYY-MM" month key in shape only — unlike daily::valid_month this
/// applies no range check, because a snapshot's month is whatever month
/// the caller says the total belongs to (the import path takes files at
/// their word the same way). The month must still be a real calendar
/// month, verified by parsing the 1st.
fn valid_month_key(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 7
        && b[4] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || c.is_ascii_digit())
        && chrono::NaiveDate::parse_from_str(&format!("{s}-01"), "%Y-%m-%d").is_ok()
}
fn valid_lot_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
        && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}
/// The optional `date` argument of add_lot: absent or blank means unrecorded.
fn lot_date(args: &Value) -> Result<String, String> {
    let Some(v) = args.get("date") else {
        return Ok(String::new());
    };
    let s = v.as_str().ok_or("日期格式錯誤")?.trim();
    if s.is_empty() {
        return Ok(String::new());
    }
    if !valid_lot_date(s) {
        return Err("日期需為 YYYY-MM-DD".into());
    }
    Ok(s.to_string())
}
/// One line of a pasted positions import: `CODE, DATE, PRICE, SHARES`, with the
/// date optional (three fields read as CODE, PRICE, SHARES). Fields may be
/// separated by commas, tabs or spaces. None when the line does not parse.
fn parse_import_line(line: &str) -> Option<(String, Lot)> {
    let fields: Vec<&str> = line
        .split(|c| c == ',' || c == '\t' || c == '，')
        .flat_map(|part| part.split_whitespace())
        .collect();
    let (raw_code, date, raw_price, raw_shares) = match fields.as_slice() {
        [code, date, price, shares] => (*code, *date, *price, *shares),
        [code, price, shares] => (*code, "", *price, *shares),
        _ => return None,
    };
    if !date.is_empty() && !valid_lot_date(date) {
        return None;
    }
    let code = symbol(raw_code)?;
    let price: f64 = raw_price.parse().ok()?;
    let shares: f64 = raw_shares.parse().ok()?;
    if !price.is_finite() || price <= 0. || !shares.is_finite() || shares <= 0. {
        return None;
    }
    Some((
        code,
        Lot {
            date: date.to_string(),
            price,
            shares,
        },
    ))
}
#[derive(Clone)]
pub struct Core {
    pub intraday_queue: std::collections::VecDeque<String>,
    pub intraday_pending: std::collections::HashSet<String>,
    pub intraday_errors: HashMap<String, (String, i64)>,
    pub last_intraday_save: i64,
    pub daily_queue: std::collections::VecDeque<(String, String)>,
    pub daily_pending: std::collections::HashSet<String>,
    pub daily_errors: HashMap<String, (String, i64)>,
    pub calendar_errors: HashMap<i32, (String, i64)>,
    pub saved: Saved,
    pub quotes: HashMap<String, Quote>,
    pub path: PathBuf,
    pub demo: bool,
    pub error: Option<String>,
    pub last_fetch: Option<i64>,
    pub failures: u32,
    pub next_poll: i64,
    pub revision: u64,
    pub config_revision: u64,
    pub notices_hidden: bool,
    pub privacy_shortcut_error: Option<String>,
    /// Non-fatal note about the live-quote source (e.g. sidecar fell back to MIS),
    /// shown in the UI footer; None when the configured source served everything.
    pub source_note: Option<String>,
    /// Outcome of the most recent state backup, for the settings page:
    /// (Taipei timestamp, full path written) on success.
    pub last_backup_at: Option<(String, String)>,
    /// Why the most recent backup attempt failed, when it did. Backup paths
    /// are user-chosen folders, never secrets.
    pub last_backup_error: Option<String>,
}
impl Core {
    pub fn load(path: PathBuf, demo: bool) -> Result<Self, String> {
        let mut saved: Saved = if path.exists() {
            serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("設定檔無法讀取，已保留原檔：{e}"))?
        } else {
            Saved {
                watchlist: vec!["t00".into(), "2330".into(), "0050".into()],
                ..Default::default()
            }
        };
        saved.normalize_groups();
        for r in &mut saved.rules {
            r.last_value = None;
            r.last_at = None;
            r.last_day = None;
            r.last_high = None;
            r.last_low = None;
        }
        Ok(Self {
            intraday_queue: Default::default(),
            intraday_pending: Default::default(),
            intraday_errors: Default::default(),
            last_intraday_save: 0,
            daily_queue: Default::default(),
            daily_pending: Default::default(),
            daily_errors: Default::default(),
            calendar_errors: Default::default(),
            saved,
            quotes: HashMap::new(),
            path,
            demo,
            error: None,
            last_fetch: None,
            failures: 0,
            next_poll: 0,
            revision: 0,
            config_revision: 0,
            notices_hidden: false,
            privacy_shortcut_error: None,
            source_note: None,
            last_backup_at: None,
            last_backup_error: None,
        })
    }
    pub fn persist(&mut self) -> Result<(), String> {
        let temp = self.path.with_extension("tmp");
        std::fs::write(
            &temp,
            serde_json::to_vec_pretty(&self.saved).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(temp, &self.path).map_err(|e| e.to_string())?;
        // A successful save is the automatic-backup trigger point; the
        // backup rides along silently and never fails the save itself.
        self.maybe_backup(false);
        Ok(())
    }
    /// Automatic state backup, called after every successful persist and
    /// once at startup. Acts only when backups are enabled, a folder is
    /// configured and exists, and state.json is on disk. Unless `force`,
    /// the write is skipped when the newest backup in the folder is already
    /// from today (Asia/Taipei) — the timestamped file names make that a
    /// name check, so the per-save trigger costs one directory scan.
    /// Failures are recorded in `last_backup_error` for the settings page
    /// and otherwise swallowed.
    pub fn maybe_backup(&mut self, force: bool) {
        let settings = &self.saved.settings;
        if !settings.backup_enabled || settings.backup_dir.trim().is_empty() {
            return;
        }
        let dir = PathBuf::from(settings.backup_dir.trim());
        if !dir.is_dir() || !self.path.exists() {
            return;
        }
        if !force {
            let today = format!("state-backup-{}", local(now()).format("%Y%m%d"));
            if backup_files(&dir)
                .last()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&today))
            {
                return;
            }
        }
        match self.write_backup_file(&dir) {
            Ok(path) => {
                self.last_backup_at = Some((
                    local(now()).format("%Y-%m-%d %H:%M:%S").to_string(),
                    path,
                ));
                self.last_backup_error = None;
            }
            Err(e) => self.last_backup_error = Some(e),
        }
    }
    /// Copies the current on-disk state.json into `dir` as
    /// state-backup-YYYYMMDD-HHmmss.json (Taipei) and prunes the folder's
    /// backups to the newest BACKUP_KEEP. Returns the full path written.
    fn write_backup_file(&self, dir: &std::path::Path) -> Result<String, String> {
        let content =
            std::fs::read(&self.path).map_err(|e| format!("讀取 state.json 失敗：{e}"))?;
        let name = format!(
            "state-backup-{}.json",
            local(now()).format("%Y%m%d-%H%M%S")
        );
        let dest = dir.join(name);
        std::fs::write(&dest, &content).map_err(|e| format!("寫入備份失敗：{e}"))?;
        let files = backup_files(dir);
        if files.len() > BACKUP_KEEP {
            for old in &files[..files.len() - BACKUP_KEEP] {
                let _ = std::fs::remove_file(old);
            }
        }
        Ok(dest.to_string_lossy().into_owned())
    }
    /// The export_data payload as a JSON string: the watch groups (with the
    /// active group index), all positions and the monthly snapshots.
    /// Settings and rules stay out.
    fn export_text(&self) -> Result<String, String> {
        let payload = json!({
            "app": "smallstocks-tw",
            "version": 1,
            "groups": self.saved.groups,
            "activeGroup": self.saved.active_group,
            "positions": self.saved.positions,
            "monthlySnapshots": self.saved.monthly_snapshots,
        });
        serde_json::to_string_pretty(&payload).map_err(|e| format!("匯出序列化失敗：{e}"))
    }
    /// Where export_data_file writes: the user's Downloads folder when it
    /// exists (Windows %USERPROFILE%\Downloads, elsewhere $HOME/Downloads),
    /// else the folder holding state.json.
    fn export_dir(&self) -> PathBuf {
        let home = std::env::var_os("USERPROFILE")
            .filter(|v| !v.is_empty())
            .or_else(|| std::env::var_os("HOME").filter(|v| !v.is_empty()));
        if let Some(h) = home {
            let d = PathBuf::from(h).join("Downloads");
            if d.is_dir() {
                return d;
            }
        }
        self.path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }
    /// Name of the calendar closure on this Taipei date, if the official calendar lists one.
    fn closure_on(&self, ms: i64) -> Option<&calendar::Closure> {
        let day = local(ms);
        self.saved
            .calendars
            .get(&day.year().to_string())?
            .closures
            .iter()
            .find(|c| c.date == day.format("%Y-%m-%d").to_string())
    }
    /// Session hours on a weekday that the official calendar does not mark as closed.
    pub fn market_open(&self, ms: i64) -> bool {
        trading(ms) && self.closure_on(ms).is_none()
    }
    /// When polling the source is worthwhile: a little before the open (08:55) until three minutes
    /// after the delayed-close limit (13:33), on a weekday the calendar does not close. The final
    /// 13:30 closing price and any postponed close are only published after the session ends.
    pub fn polling_window(&self, ms: i64) -> bool {
        let day = local(ms);
        let minutes = day.hour() * 60 + day.minute();
        day.weekday().number_from_monday() <= 5
            && (535..816).contains(&minutes)
            && self.closure_on(ms).is_none()
    }
    /// Why a weekday session is closed (holiday name), for the status line.
    pub fn closed_reason(&self, ms: i64) -> Option<String> {
        let day = local(ms);
        (day.weekday().number_from_monday() <= 5)
            .then(|| self.closure_on(ms).map(|c| c.name.clone()))
            .flatten()
    }
    fn calendar_view(&self) -> Value {
        let year = local(now()).year();
        json!({
            "years": self.saved.calendars.keys().collect::<Vec<_>>(),
            "currentYearLoaded": self.saved.calendars.contains_key(&year.to_string()),
            "error": self.calendar_errors.get(&year).map(|(e, _)| e),
        })
    }
    /// Years whose calendar should be fetched now: missing (retry after an hour), older than six
    /// hours, or refreshed once at 08:30-09:00 on a weekday so a same-day announcement is seen.
    pub fn calendar_due(&self, at: i64) -> Vec<i32> {
        let day = local(at);
        let this_year = day.year();
        let mut years = vec![this_year];
        if day.month() >= 11 {
            years.push(this_year + 1);
        }
        let minutes = day.hour() * 60 + day.minute();
        let pre_open = day.weekday().number_from_monday() <= 5 && (510..540).contains(&minutes);
        let today_pre_open = at - (minutes as i64 - 510) * 60_000 - day.second() as i64 * 1000;
        years
            .into_iter()
            .filter(|y| {
                if self
                    .calendar_errors
                    .get(y)
                    .is_some_and(|(_, t)| at - t < 3_600_000)
                {
                    return false;
                }
                match self.saved.calendars.get(&y.to_string()) {
                    None => true,
                    Some(c) => {
                        at - c.fetched_at >= 6 * 3_600_000
                            || (*y == this_year && pre_open && c.fetched_at < today_pre_open)
                    }
                }
            })
            .collect()
    }
    /// The settings as the UI and MCP may see them: identical to the stored
    /// settings except the Telegram bot token, which is replaced by a
    /// `telegramTokenSet` flag. The token itself never leaves the core.
    fn settings_view(&self) -> Value {
        let mut v = serde_json::to_value(&self.saved.settings).unwrap_or_else(|_| json!({}));
        let token_set = !self.saved.settings.telegram_bot_token.trim().is_empty();
        if let Some(obj) = v.as_object_mut() {
            obj.remove("telegramBotToken");
            obj.insert("telegramTokenSet".into(), json!(token_set));
        }
        v
    }
    pub fn snapshot(&self) -> Value {
        let months = daily::months(now());
        // Daily summaries cover the visible watchlist plus Taiwan holdings, so
        // the portfolio page can show official closes for held codes that are
        // in no watch group.
        let mut summary_codes = self.saved.watchlist.clone();
        for p in &self.saved.positions {
            if is_tw_symbol(&p.code) && !summary_codes.contains(&p.code) {
                summary_codes.push(p.code.clone());
            }
        }
        let summaries: HashMap<String, Value> = summary_codes
            .iter()
            .map(|c| (c.clone(), self.daily_view(c, &months)))
            .collect();
        // Intraday views cover the same codes as the daily summaries: the
        // watchlist plus held Taiwan codes, so the portfolio page can chart
        // positions that sit in no watch group. Display only — queueing a
        // fetch stays with get_status / get_intraday_history.
        let intraday: HashMap<String, Value> = summary_codes
            .iter()
            .map(|c| (c.clone(), self.intraday_view(c)))
            .collect();
        json!({"watchlist":self.saved.watchlist,"groups":self.saved.groups,"activeGroup":self.saved.active_group,"positions":self.saved.positions,"monthlySnapshots":self.saved.monthly_snapshots,"names":self.saved.names,"settings":self.settings_view(),"privacyShortcutError":self.privacy_shortcut_error,"rules":self.saved.rules,"events":self.saved.events,"quotes":self.quotes,"demo":self.demo,"error":self.error,"sourceNote":self.source_note,"lastBackupAt":self.last_backup_at.as_ref().map(|(at, path)| json!({"at":at,"path":path})),"lastBackupError":self.last_backup_error,"lastFetch":self.last_fetch,"nextPoll":self.next_poll,"marketOpen":self.market_open(now()),"marketClosedReason":self.closed_reason(now()),"calendar":self.calendar_view(),"revision":self.revision,"now":now(),"dailySummaries":summaries,"intraday":intraday})
    }
    pub fn queue_intraday(&mut self, code: &str) {
        let at = now();
        let fresh = self
            .saved
            .intraday_cache
            .get(code)
            .is_some_and(|s| s.date == intraday::day(at) && at - s.fetched_at < 60_000);
        let cooldown = self
            .intraday_errors
            .get(code)
            .is_some_and(|(_, t)| at - t < 300_000);
        if !fresh
            && !cooldown
            && !self.intraday_pending.contains(code)
            && self.intraday_queue.len() < 32
        {
            self.intraday_pending.insert(code.into());
            self.intraday_queue.push_back(code.into());
        }
    }
    pub fn intraday_view(&self, code: &str) -> Value {
        let date = intraday::day(now());
        let external = self
            .saved
            .intraday_cache
            .get(code)
            .filter(|s| s.date == date);
        let series = external.or_else(|| {
            self.saved
                .local_intraday
                .get(code)
                .filter(|s| s.date == date)
        });
        json!({"series":series,"pending":self.intraday_pending.contains(code),"error":self.intraday_errors.get(code).map(|(e,_)|e),"demo":self.demo})
    }
    fn record_intraday(&mut self, q: &Quote, at: i64) {
        if !self.demo
            && (!intraday::session(q.source_at)
                || q.date != intraday::day(at)
                || at - q.source_at > 60_000
                || q.source_at > at + 5000)
        {
            return;
        }
        let Some(price) = q.price else { return };
        let series = self
            .saved
            .local_intraday
            .entry(q.symbol.clone())
            .or_insert_with(|| intraday::Series {
                symbol: q.symbol.clone(),
                date: q.date.clone(),
                source: "本機 MIS 觀測（局部）".into(),
                fetched_at: at,
                partial: true,
                points: Vec::new(),
            });
        if series.date != q.date {
            series.date = q.date.clone();
            series.points.clear();
        }
        let point = intraday::Point {
            at: q.source_at,
            price: Some(price),
        };
        match series.points.last_mut() {
            Some(last) if last.at > point.at => return,
            Some(last) if last.at / 60000 == point.at / 60000 => *last = point,
            _ => series.points.push(point),
        }
        series.fetched_at = at;
        series.points.truncate(300);
    }
    pub fn queue_daily(&mut self, code: &str, month: &str) {
        let key = format!("{code}:{month}");
        let at = now();
        let ttl = if month == daily::months(at)[0] {
            3_600_000
        } else {
            86_400_000
        };
        let fresh = self.saved.daily_cache.get(&key).is_some_and(|s| {
            at - s.fetched_at < ttl
                && !(local(at).hour() >= 15
                    && local(s.fetched_at).date_naive() == local(at).date_naive()
                    && local(s.fetched_at).hour() < 15)
        });
        let cooldown = self
            .daily_errors
            .get(&key)
            .is_some_and(|(_, t)| at - t < 300_000);
        if !fresh && !cooldown && !self.daily_pending.contains(&key) && self.daily_queue.len() < 64
        {
            self.daily_pending.insert(key);
            self.daily_queue.push_back((code.into(), month.into()));
        }
    }
    pub fn daily_view(&self, code: &str, months: &[String]) -> Value {
        let mut days = Vec::new();
        let mut sources = Vec::new();
        let mut fetched_at = 0;
        let mut pending = false;
        let mut errors = Vec::new();
        for m in months {
            let key = format!("{code}:{m}");
            pending |= self.daily_pending.contains(&key);
            if let Some(s) = self.saved.daily_cache.get(&key) {
                days.extend(s.days.clone());
                fetched_at = fetched_at.max(s.fetched_at);
                if !sources.contains(&s.source) {
                    sources.push(s.source.clone())
                }
            }
            if let Some((e, _)) = self.daily_errors.get(&key) {
                errors.push(e.clone())
            }
        }
        days.sort_by(|a, b| a.date.cmp(&b.date));
        days.dedup_by(|a, b| a.date == b.date);
        // Cached current-day data must not leak into a new day's pre-close view after clock changes.
        let today = local(now());
        days.retain(|d| {
            d.date < today.format("%Y-%m-%d").to_string()
                || d.date == today.format("%Y-%m-%d").to_string() && today.hour() >= 15
                || self.demo
        });
        if months.len() > 1 && days.len() > 20 {
            days.drain(..days.len() - 20);
        }
        json!({"symbol":code,"days":days,"sources":sources,"fetchedAt":fetched_at,"pending":pending,"error":errors.first(),"demo":self.demo})
    }
    pub fn execute(&mut self, tool: &str, args: Value) -> Result<Value, String> {
        if tool == "get_status" {
            // The intraday/daily prefetch queues feed Taiwan-only modules; Yahoo
            // symbols never enter them (their minutes and closes are not TWSE data).
            if self.saved.settings.row_display == "trend" && (self.market_open(now()) || self.demo)
            {
                for c in self.saved.watchlist.clone() {
                    if is_tw_symbol(&c) {
                        self.queue_intraday(&c)
                    }
                }
            }
            if !self.market_open(now()) || self.demo {
                let months = daily::months(now());
                // Held Taiwan codes need their official closes too, even when
                // they sit in no watch group.
                let mut daily_codes = self.saved.watchlist.clone();
                for p in &self.saved.positions {
                    if is_tw_symbol(&p.code) && !daily_codes.contains(&p.code) {
                        daily_codes.push(p.code.clone());
                    }
                }
                for c in daily_codes {
                    if is_tw_symbol(&c) {
                        for m in &months {
                            self.queue_daily(&c, m)
                        }
                    }
                }
            }
            return Ok(self.snapshot());
        }
        if tool == "get_intraday_history" {
            let code = args["symbol"]
                .as_str()
                .and_then(symbol)
                .ok_or("需要台股代號")?;
            if !is_tw_symbol(&code) {
                return Err("分鐘線僅支援台股代號".into());
            }
            if !self.saved.watches(&code) {
                return Err("請先將標的加入自選，再讀取分鐘線".into());
            }
            self.queue_intraday(&code);
            return Ok(self.intraday_view(&code));
        }
        if tool == "get_daily_history" {
            let code = args["symbol"]
                .as_str()
                .and_then(symbol)
                .ok_or("需要台股代號")?;
            if !is_tw_symbol(&code) {
                return Err("日資料僅支援台股代號".into());
            }
            let month = args["month"].as_str().ok_or("需要 YYYY-MM 月份")?;
            if !daily::valid_month(month) {
                return Err("月份需介於 2000-01 與本月之間".into());
            }
            self.queue_daily(&code, month);
            return Ok(self.daily_view(&code, &[month.into()]));
        }
        if tool == "list_events" {
            return Ok(json!(self.saved.events));
        }
        if tool == "list_rules" {
            return Ok(json!(self.saved.rules));
        }
        if tool == "export_data" {
            // One JSON string packing the watch groups and the positions —
            // settings and rules deliberately stay out (see import_data).
            return Ok(Value::String(self.export_text()?));
        }
        if tool == "export_data_file" {
            // The same payload, written to a file by the core itself: the
            // desktop shell's Blob-download path is silently ignored by
            // WebView2, so the settings page's export button calls this and
            // reports the path it gets back.
            let text = self.export_text()?;
            let file = format!("smallstocks-data-{}.json", local(now()).format("%Y%m%d"));
            let dest = self.export_dir().join(file);
            std::fs::write(&dest, text).map_err(|e| format!("匯出寫檔失敗：{e}"))?;
            return Ok(json!({"path": dest.to_string_lossy()}));
        }
        if tool == "telegram_test" {
            // UI-only on purpose: this command is not in the MCP tool list,
            // so an agent can neither spend the user's bot nor probe whether
            // a token is stored. Sends with the saved settings; the reply
            // never carries the token or the request URL.
            telegram_send_blocking(
                &self.saved.settings,
                "Maxi stocks 測試訊息：Telegram 推播已連通",
            )?;
            return Ok(json!({"sent": true}));
        }
        if tool == "backup_now" {
            // Manual backup: same write as the automatic one but without
            // the once-a-day throttle; the folder must be configured and
            // exist. Also reachable through MCP (no secrets involved).
            let dir_raw = self.saved.settings.backup_dir.trim().to_string();
            if dir_raw.is_empty() {
                return Err("尚未設定備份資料夾".into());
            }
            let dir = PathBuf::from(&dir_raw);
            if !dir.is_dir() {
                return Err("備份資料夾不存在，請先建立它或改指向現有資料夾".into());
            }
            return match self.write_backup_file(&dir) {
                Ok(path) => {
                    self.last_backup_at = Some((
                        local(now()).format("%Y-%m-%d %H:%M:%S").to_string(),
                        path.clone(),
                    ));
                    self.last_backup_error = None;
                    Ok(json!({"path": path}))
                }
                Err(e) => {
                    self.last_backup_error = Some(e.clone());
                    Err(format!("備份失敗：{e}"))
                }
            };
        }
        let mut next = self.clone();
        next.saved.normalize_groups();
        // Set by import_positions so its added/failed tally rides the returned
        // snapshot; every other command leaves it empty.
        let mut import_report: Option<Value> = None;
        // Same idea for import_data's groups/positions/lots tally.
        let mut data_report: Option<Value> = None;
        match tool {
            "set_watchlist" => {
                let raw = args["symbols"].as_array().ok_or("需要 symbols 陣列")?;
                if raw.len() > MAX_GROUP_SYMBOLS {
                    return Err("首版最多監看 30 檔".into());
                }
                let mut codes = Vec::new();
                for s in raw {
                    let c = s.as_str().and_then(symbol).ok_or("請使用有效代號")?;
                    if !codes.contains(&c) {
                        codes.push(c)
                    }
                }
                let active = next.saved.active_group;
                next.saved.groups[active].codes = codes;
                // A symbol may only leave the monitoring union when no enabled rule needs it;
                // staying in another group is enough.
                let watched = next.saved.all_codes();
                if next
                    .saved
                    .rules
                    .iter()
                    .any(|r| r.enabled && !watched.contains(&r.symbol))
                {
                    return Err("請先停用或刪除該股票的規則，再移除自選".into());
                }
                next.saved.sync_watchlist();
                next.saved
                    .intraday_cache
                    .retain(|c, _| watched.contains(c));
                next.saved
                    .local_intraday
                    .retain(|c, _| watched.contains(c));
                next.last_fetch = None;
                next.quotes.retain(|c, _| watched.contains(c));
                next.next_poll = 0;
            }
            "add_watch_group" => {
                if next.saved.groups.len() >= MAX_WATCH_GROUPS {
                    return Err("分組最多 10 個".into());
                }
                let name = if args.get("name").is_some() {
                    group_name(&args)?
                } else {
                    next.saved.default_group_name()
                };
                if next.saved.groups.iter().any(|g| g.name == name) {
                    return Err("分組名稱已存在".into());
                }
                next.saved.groups.push(WatchGroup {
                    name,
                    codes: Vec::new(),
                });
                next.saved.active_group = next.saved.groups.len() - 1;
                next.saved.sync_watchlist();
            }
            "rename_watch_group" => {
                let idx = group_index(&args, next.saved.groups.len())?;
                let name = group_name(&args)?;
                if next
                    .saved
                    .groups
                    .iter()
                    .enumerate()
                    .any(|(i, g)| i != idx && g.name == name)
                {
                    return Err("分組名稱已存在".into());
                }
                next.saved.groups[idx].name = name;
            }
            "delete_watch_group" => {
                let idx = group_index(&args, next.saved.groups.len())?;
                if next.saved.groups.len() <= 1 {
                    return Err("至少保留一個分組".into());
                }
                next.saved.groups.remove(idx);
                let watched = next.saved.all_codes();
                if next
                    .saved
                    .rules
                    .iter()
                    .any(|r| r.enabled && !watched.contains(&r.symbol))
                {
                    return Err("請先停用或刪除該股票的規則，再刪除分組".into());
                }
                if next.saved.active_group > idx {
                    next.saved.active_group -= 1;
                }
                if next.saved.active_group >= next.saved.groups.len() {
                    next.saved.active_group = next.saved.groups.len() - 1;
                }
                next.saved.sync_watchlist();
                next.saved
                    .intraday_cache
                    .retain(|c, _| watched.contains(c));
                next.saved
                    .local_intraday
                    .retain(|c, _| watched.contains(c));
                next.last_fetch = None;
                next.quotes.retain(|c, _| watched.contains(c));
                next.next_poll = 0;
            }
            "set_active_group" => {
                let idx = group_index(&args, next.saved.groups.len())?;
                next.saved.active_group = idx;
                next.saved.sync_watchlist();
            }
            "add_lot" => {
                let code = args["code"]
                    .as_str()
                    .and_then(symbol)
                    .ok_or("代號格式不正確")?;
                let date = lot_date(&args)?;
                let price = args["price"]
                    .as_f64()
                    .filter(|n| n.is_finite() && *n > 0.)
                    .ok_or("購入價必須大於零")?;
                let shares = args["shares"]
                    .as_f64()
                    .filter(|n| n.is_finite() && *n > 0.)
                    .ok_or("股數必須大於零")?;
                let lot = Lot {
                    date,
                    price,
                    shares,
                };
                match next.saved.positions.iter_mut().find(|p| p.code == code) {
                    Some(p) => p.lots.push(lot),
                    None => next.saved.positions.push(Position {
                        code,
                        lots: vec![lot],
                    }),
                }
                next.next_poll = 0;
            }
            "delete_lot" => {
                let code = args["code"]
                    .as_str()
                    .and_then(symbol)
                    .ok_or("代號格式不正確")?;
                let idx = args["index"].as_u64().ok_or("需要批次編號")? as usize;
                let Some(pi) = next.saved.positions.iter().position(|p| p.code == code)
                else {
                    return Err("找不到持股".into());
                };
                if idx >= next.saved.positions[pi].lots.len() {
                    return Err("找不到批次".into());
                }
                next.saved.positions[pi].lots.remove(idx);
                if next.saved.positions[pi].lots.is_empty() {
                    next.saved.positions.remove(pi);
                }
            }
            "delete_position" => {
                let code = args["code"]
                    .as_str()
                    .and_then(symbol)
                    .ok_or("代號格式不正確")?;
                if !next.saved.positions.iter().any(|p| p.code == code) {
                    return Err("找不到持股".into());
                }
                next.saved.positions.retain(|p| p.code != code);
            }
            "record_monthly_snapshot" => {
                // First write wins: the UI calls this whenever the header
                // total is computable and the month is not yet recorded,
                // so a repeat call for a recorded month is a plain no-op,
                // never an overwrite and never an error.
                let month = args["month"].as_str().ok_or("需要月份")?.trim();
                if !valid_month_key(month) {
                    return Err("月份需為 YYYY-MM".into());
                }
                let total_twd = args["totalTwd"]
                    .as_f64()
                    .filter(|n| n.is_finite() && *n > 0.)
                    .ok_or("總資產必須大於零")?;
                let twd_value = args["twdValue"]
                    .as_f64()
                    .filter(|n| n.is_finite() && *n >= 0.)
                    .ok_or("台幣市值格式錯誤")?;
                let usd_value = match args.get("usdValue") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(
                        v.as_f64()
                            .filter(|n| n.is_finite() && *n >= 0.)
                            .ok_or("美元市值格式錯誤")?,
                    ),
                };
                let rate = match args.get("rate") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(
                        v.as_f64()
                            .filter(|n| n.is_finite() && *n > 0.)
                            .ok_or("匯率格式錯誤")?,
                    ),
                };
                let recorded_on = args["recordedOn"].as_str().ok_or("需要記錄日期")?.trim();
                if !valid_lot_date(recorded_on) {
                    return Err("記錄日期需為 YYYY-MM-DD".into());
                }
                if !next
                    .saved
                    .monthly_snapshots
                    .iter()
                    .any(|s| s.month == month)
                {
                    next.saved.monthly_snapshots.push(MonthlySnapshot {
                        month: month.to_string(),
                        total_twd,
                        twd_value,
                        usd_value,
                        rate,
                        recorded_on: recorded_on.to_string(),
                    });
                }
            }
            "delete_monthly_snapshot" => {
                let month = args["month"].as_str().ok_or("需要月份")?.trim();
                if !valid_month_key(month) {
                    return Err("月份需為 YYYY-MM".into());
                }
                next.saved.monthly_snapshots.retain(|s| s.month != month);
            }
            "import_positions" => {
                let text = args["text"].as_str().ok_or("需要匯入文字")?;
                let mut added = 0usize;
                let mut failed_lines: Vec<usize> = Vec::new();
                for (i, line) in text.lines().enumerate() {
                    if line.trim().is_empty() {
                        continue;
                    }
                    match parse_import_line(line) {
                        Some((code, lot)) => {
                            match next.saved.positions.iter_mut().find(|p| p.code == code) {
                                Some(p) => p.lots.push(lot),
                                None => next.saved.positions.push(Position {
                                    code,
                                    lots: vec![lot],
                                }),
                            }
                            added += 1;
                        }
                        None => failed_lines.push(i + 1),
                    }
                }
                import_report = Some(json!({"added": added, "failedLines": failed_lines}));
                next.next_poll = 0;
            }
            "import_data" => {
                // Whole-file replace of the watch groups and positions from
                // an export_data file. Anything that would silently lose
                // data (bad envelope, over-cap groups, broken group names,
                // an enabled rule losing its symbol) rejects the whole file
                // and changes nothing; individual bad symbols or lots are
                // skipped and tallied, mirroring import_positions.
                let text = args["json"].as_str().ok_or("需要匯入 JSON")?;
                let v: Value = serde_json::from_str(text)
                    .map_err(|_| "匯入檔案不是有效的 JSON".to_string())?;
                if v["app"].as_str() != Some("smallstocks-tw") {
                    return Err("這不是 SmallStocks TW 的匯出檔案".into());
                }
                if v["version"].as_u64() != Some(1) {
                    return Err("不支援的匯出檔案版本".into());
                }
                let raw_groups = v["groups"].as_array().ok_or("匯出檔案缺少分組資料")?;
                if raw_groups.is_empty() {
                    return Err("匯出檔案沒有任何分組".into());
                }
                if raw_groups.len() > MAX_WATCH_GROUPS {
                    return Err("分組超過 10 個上限，檔案未匯入".into());
                }
                let mut skipped_symbols: Vec<String> = Vec::new();
                let mut new_groups: Vec<WatchGroup> = Vec::new();
                for g in raw_groups {
                    let name = g["name"].as_str().map(str::trim).unwrap_or("");
                    if name.is_empty() || name.chars().count() > 20 {
                        return Err("分組名稱空白或超過 20 字，檔案未匯入".into());
                    }
                    if new_groups.iter().any(|x| x.name == name) {
                        return Err(format!("分組名稱重複：{name}，檔案未匯入"));
                    }
                    let raw_codes = g["codes"].as_array().ok_or("分組缺少代號清單")?;
                    let mut codes: Vec<String> = Vec::new();
                    for s in raw_codes {
                        match s.as_str().and_then(symbol) {
                            Some(c) => {
                                if !codes.contains(&c) {
                                    codes.push(c)
                                }
                            }
                            None => skipped_symbols.push(
                                s.as_str().map(str::to_string).unwrap_or_else(|| s.to_string()),
                            ),
                        }
                    }
                    if codes.len() > MAX_GROUP_SYMBOLS {
                        return Err(format!("分組「{name}」超過 30 檔上限，檔案未匯入"));
                    }
                    new_groups.push(WatchGroup {
                        name: name.to_string(),
                        codes,
                    });
                }
                let raw_positions = v["positions"].as_array().ok_or("匯出檔案缺少持股資料")?;
                let mut new_positions: Vec<Position> = Vec::new();
                let mut skipped_lots = 0usize;
                for p in raw_positions {
                    let Some(code) = p["code"].as_str().and_then(symbol) else {
                        skipped_symbols.push(
                            p["code"]
                                .as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| p["code"].to_string()),
                        );
                        continue;
                    };
                    let Some(raw_lots) = p["lots"].as_array() else {
                        continue;
                    };
                    let mut lots: Vec<Lot> = Vec::new();
                    for l in raw_lots {
                        let date = l["date"].as_str().unwrap_or("").trim();
                        let valid = l["price"]
                            .as_f64()
                            .is_some_and(|n| n.is_finite() && n > 0.)
                            && l["shares"]
                                .as_f64()
                                .is_some_and(|n| n.is_finite() && n > 0.)
                            && (date.is_empty() || valid_lot_date(date));
                        if valid {
                            lots.push(Lot {
                                date: date.to_string(),
                                price: l["price"].as_f64().unwrap(),
                                shares: l["shares"].as_f64().unwrap(),
                            });
                        } else {
                            skipped_lots += 1;
                        }
                    }
                    if lots.is_empty() {
                        continue;
                    }
                    match new_positions.iter_mut().find(|x| x.code == code) {
                        Some(x) => x.lots.extend(lots),
                        None => new_positions.push(Position { code, lots }),
                    }
                }
                // Monthly snapshots ride the same whole-file replace. Old
                // exports have no such field and simply mean none; a bad
                // entry (month shape, total, recorded-on date) is skipped
                // and tallied like a bad lot, and a repeated month keeps
                // its first entry, mirroring first-write-wins recording.
                let mut new_snapshots: Vec<MonthlySnapshot> = Vec::new();
                let mut skipped_snapshots = 0usize;
                if let Some(raw_snapshots) = v.get("monthlySnapshots") {
                    let Some(raw_snapshots) = raw_snapshots.as_array() else {
                        return Err("匯出檔案的月度紀錄格式錯誤，檔案未匯入".into());
                    };
                    for s in raw_snapshots {
                        let month = s["month"].as_str().unwrap_or("").trim();
                        let recorded_on = s["recordedOn"].as_str().unwrap_or("").trim();
                        let parsed = (|| {
                            if !valid_month_key(month) || !valid_lot_date(recorded_on) {
                                return None;
                            }
                            let total_twd = s["totalTwd"]
                                .as_f64()
                                .filter(|n| n.is_finite() && *n > 0.)?;
                            let twd_value = s["twdValue"]
                                .as_f64()
                                .filter(|n| n.is_finite() && *n >= 0.)?;
                            let usd_value = match s.get("usdValue") {
                                None | Some(Value::Null) => None,
                                Some(v) => Some(
                                    v.as_f64().filter(|n| n.is_finite() && *n >= 0.)?,
                                ),
                            };
                            let rate = match s.get("rate") {
                                None | Some(Value::Null) => None,
                                Some(v) => {
                                    Some(v.as_f64().filter(|n| n.is_finite() && *n > 0.)?)
                                }
                            };
                            Some(MonthlySnapshot {
                                month: month.to_string(),
                                total_twd,
                                twd_value,
                                usd_value,
                                rate,
                                recorded_on: recorded_on.to_string(),
                            })
                        })();
                        match parsed {
                            Some(snap)
                                if !new_snapshots.iter().any(|x| x.month == snap.month) =>
                            {
                                new_snapshots.push(snap)
                            }
                            _ => skipped_snapshots += 1,
                        }
                    }
                }
                // An enabled rule must keep its symbol in the monitoring
                // union, exactly as set_watchlist guards it.
                let mut union: Vec<String> = Vec::new();
                for g in &new_groups {
                    for c in &g.codes {
                        if !union.contains(c) {
                            union.push(c.clone());
                        }
                    }
                }
                for p in &new_positions {
                    if !union.contains(&p.code) {
                        union.push(p.code.clone());
                    }
                }
                if next
                    .saved
                    .rules
                    .iter()
                    .any(|r| r.enabled && !union.contains(&r.symbol))
                {
                    return Err("匯入後將有啟用中的規則失去監看代號，檔案未匯入；請先停用或刪除相關規則".into());
                }
                let lot_total: usize = new_positions.iter().map(|p| p.lots.len()).sum();
                data_report = Some(json!({
                    "groups": new_groups.len(),
                    "positions": new_positions.len(),
                    "lots": lot_total,
                    "skippedSymbols": skipped_symbols,
                    "skippedLots": skipped_lots,
                    "skippedSnapshots": skipped_snapshots,
                }));
                next.saved.groups = new_groups;
                next.saved.active_group = (v["activeGroup"].as_u64().unwrap_or(0) as usize)
                    .min(next.saved.groups.len() - 1);
                next.saved.positions = new_positions;
                next.saved.monthly_snapshots = new_snapshots;
                next.saved.sync_watchlist();
                next.saved
                    .intraday_cache
                    .retain(|c, _| union.contains(c));
                next.saved.local_intraday.retain(|c, _| union.contains(c));
                next.last_fetch = None;
                next.quotes.retain(|c, _| union.contains(c));
                next.next_poll = 0;
            }
            "set_settings" => {
                if let Some(v) = args.get("alwaysOnTop") {
                    next.saved.settings.always_on_top = v.as_bool().ok_or("置頂格式錯誤")?;
                }
                if let Some(v) = args.get("rowDisplay") {
                    let v = v.as_str().ok_or("列顯示格式錯誤")?;
                    if !["numbers", "trend", "minimal"].contains(&v) {
                        return Err("列顯示需為 numbers、trend 或 minimal".into());
                    }
                    next.saved.settings.row_display = v.into();
                }
                if let Some(v) = args.get("showCodes") {
                    next.saved.settings.show_codes = v.as_bool().ok_or("代號顯示需為布林值")?;
                }
                if let Some(v) = args.get("hideNames") {
                    next.saved.settings.hide_names = v.as_bool().ok_or("隱藏名稱需為布林值")?;
                }
                if let Some(v) = args.get("showChangePoints") {
                    next.saved.settings.show_change_points =
                        v.as_bool().ok_or("漲跌點數顯示需為布林值")?;
                }
                if let Some(a) = args.get("appearance") {
                    let a = a.as_str().ok_or("外觀格式錯誤")?;
                    if !["small", "gray", "terminal", "txt"].contains(&a) {
                        return Err("未知外觀".into());
                    };
                    next.saved.settings.appearance = a.into();
                }
                if let Some(p) = args.get("paused") {
                    next.saved.settings.paused = p.as_bool().ok_or("暫停格式錯誤")?;
                    next.reset_baselines();
                }
                if let Some(s) = args.get("refreshSeconds") {
                    let s = s.as_u64().ok_or("更新秒數錯誤")?;
                    if !(5..=300).contains(&s) {
                        return Err("更新間隔為 5–300 秒".into());
                    };
                    next.saved.settings.refresh_seconds = s;
                }
                if let Some(v) = args.get("quoteSource") {
                    let v = v.as_str().ok_or("行情來源格式錯誤")?;
                    if !["mis", "shioaji"].contains(&v) {
                        return Err("行情來源需為 mis 或 shioaji".into());
                    }
                    next.saved.settings.quote_source = v.into();
                }
                if let Some(p) = args.get("sidecarPort") {
                    let p = p.as_u64().ok_or("sidecar 連接埠錯誤")?;
                    if !(1024..=65535).contains(&p) {
                        return Err("sidecar 連接埠需為 1024–65535".into());
                    };
                    next.saved.settings.sidecar_port = p as u16;
                }
                if let Some(v) = args.get("windowOpacity") {
                    let v = v.as_f64().ok_or("視窗透明度需為數字")?;
                    next.saved.settings.window_opacity = clamp_window_opacity(v);
                }
                if let Some(v) = args.get("telegramEnabled") {
                    next.saved.settings.telegram_enabled =
                        v.as_bool().ok_or("Telegram 推播需為布林值")?;
                }
                if let Some(v) = args.get("telegramChatId") {
                    next.saved.settings.telegram_chat_id =
                        v.as_str().ok_or("chat id 需為文字")?.trim().into();
                }
                if let Some(v) = args.get("telegramBotToken") {
                    // Write-only secret: accepted here from the settings
                    // page only (the MCP schema deliberately has no such
                    // field) and never echoed back — snapshots carry
                    // telegramTokenSet instead. An empty value means "keep
                    // the stored token"; clearing goes through
                    // telegramClearToken so a blank field cannot wipe it.
                    let v = v.as_str().ok_or("bot token 需為文字")?.trim();
                    if !v.is_empty() {
                        next.saved.settings.telegram_bot_token = v.into();
                    }
                }
                if args.get("telegramClearToken").and_then(Value::as_bool) == Some(true) {
                    next.saved.settings.telegram_bot_token = String::new();
                }
                if let Some(v) = args.get("backupEnabled") {
                    next.saved.settings.backup_enabled =
                        v.as_bool().ok_or("自動備份需為布林值")?;
                }
                if let Some(v) = args.get("backupDir") {
                    next.saved.settings.backup_dir =
                        v.as_str().ok_or("備份資料夾需為文字")?.trim().into();
                }
                if let Some(v) = args.get("chatWindow") {
                    next.saved.settings.chat_window =
                        v.as_bool().ok_or("聊天視窗需為布林值")?;
                }
                next.next_poll = 0;
            }
            "upsert_rule" => {
                let code = args["symbol"]
                    .as_str()
                    .and_then(symbol)
                    .ok_or("請指定股票代號")?;
                let metric = args["metric"].as_str().ok_or("需要 metric")?;
                let dir = args["direction"].as_str().ok_or("需要 direction")?;
                let threshold = args["threshold"]
                    .as_f64()
                    .filter(|n| n.is_finite())
                    .ok_or("門檻必須是有限數字")?;
                let channel = args["channel"].as_str().unwrap_or("strip");
                let repeat = args["repeat"].as_str().unwrap_or("daily");
                if !["price", "changePercent", "mid", "midChangePercent"].contains(&metric)
                    || !["above", "below"].contains(&dir)
                    || !["off", "log", "strip", "card"].contains(&channel)
                    || !["once", "daily"].contains(&repeat)
                {
                    return Err("不支援的規則欄位".into());
                }
                if matches!(metric, "price" | "mid") && threshold <= 0. {
                    return Err("價格門檻必須大於零".into());
                }
                if next.saved.rules.len() >= 100 && args.get("id").is_none() {
                    return Err("規則上限為 100".into());
                }
                let supplied = args
                    .get("id")
                    .map(|v| v.as_str().ok_or("規則 ID 格式錯誤"))
                    .transpose()?;
                if let Some(id) = supplied {
                    if !next.saved.rules.iter().any(|r| r.id == id) {
                        return Err("找不到規則".into());
                    }
                }
                if !next.saved.watches(&code) {
                    let active = next.saved.active_group;
                    if next.saved.groups[active].codes.len() >= MAX_GROUP_SYMBOLS {
                        return Err("自選清單已滿".into());
                    };
                    next.saved.groups[active].codes.push(code.clone());
                    next.saved.sync_watchlist();
                    next.last_fetch = None;
                }
                let id = supplied
                    .map(str::to_owned)
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                let enabled = args
                    .get("enabled")
                    .map(|v| v.as_bool().ok_or("enabled 格式錯誤"))
                    .transpose()?
                    .unwrap_or(true);
                // Editing preserves today's fire latch: no duplicate alert through edits/restarts.
                // A spent one-time rule that the user turns back on is a new one-time request.
                let previous = next.saved.rules.iter().find(|r| r.id == id);
                let rearmed = repeat == "once" && enabled && previous.is_some_and(|r| !r.enabled);
                let fired = previous
                    .filter(|_| !rearmed)
                    .and_then(|r| r.fired_day.clone());
                let rule = Rule {
                    id: id.clone(),
                    symbol: code,
                    metric: metric.into(),
                    direction: dir.into(),
                    threshold,
                    channel: channel.into(),
                    repeat: repeat.into(),
                    enabled,
                    last_value: None,
                    last_at: None,
                    last_day: None,
                    last_high: None,
                    last_low: None,
                    fired_day: fired,
                };
                next.saved.rules.retain(|r| r.id != id);
                next.saved.rules.push(rule);
                next.next_poll = 0;
            }
            "delete_rule" => {
                let id = args["id"].as_str().ok_or("需要規則 ID")?;
                if !next.saved.rules.iter().any(|r| r.id == id) {
                    return Err("找不到規則".into());
                };
                next.saved.rules.retain(|r| r.id != id);
            }
            _ => return Err("未知操作".into()),
        }
        next.revision += 1;
        next.config_revision += 1;
        next.persist()?;
        *self = next;
        let mut snap = self.snapshot();
        if let Some(report) = import_report {
            snap["positionsImport"] = report;
        }
        if let Some(report) = data_report {
            snap["dataImport"] = report;
        }
        Ok(snap)
    }
    pub fn reset_baselines(&mut self) {
        for r in &mut self.saved.rules {
            r.last_value = None;
            r.last_at = None;
            r.last_day = None;
            r.last_high = None;
            r.last_low = None;
        }
    }
    /// Applies a poll result. A failed save never discards live quotes or detected events:
    /// state stays in memory, the failure is surfaced in `error`, and saving is retried next poll.
    pub fn ingest(&mut self, quotes: Vec<Quote>, at: i64) -> Vec<Event> {
        let mut next = self.clone();
        let open = next.market_open(at);
        let mut emitted = Vec::new();
        let mut names_changed = false;
        for q in quotes {
            if !next.saved.watches(&q.symbol) {
                continue;
            }
            if next
                .quotes
                .get(&q.symbol)
                .is_some_and(|old| old.source_at > q.source_at)
            {
                continue;
            }
            // Local intraday samples are this machine's MIS observations only;
            // Yahoo points are never spliced into that series.
            if is_tw_symbol(&q.symbol) {
                next.record_intraday(&q, at);
            }
            // Each symbol is judged against its own market's session: Taiwan
            // symbols against the TWSE session, TAIFEX symbols against the
            // futures sessions (the night session included), Yahoo symbols
            // against US hours.
            let quote_open = if is_tw_symbol(&q.symbol) {
                open
            } else if is_taifex_symbol(&q.symbol) {
                taifex_market_open(at)
            } else {
                us_market_open(at)
            };
            for rule in next
                .saved
                .rules
                .iter_mut()
                .filter(|r| r.symbol == q.symbol && r.enabled)
            {
                if let Some(e) = evaluate(rule, &q, at, self.demo, quote_open) {
                    next.saved.events.insert(0, e.clone());
                    emitted.push(e)
                }
            }
            // Learn the name any source reports (MIS, Shioaji and Yahoo all
            // carry one): quotes are runtime-only, so this persisted cache is
            // what lets names survive a restart and the close-day view.
            if !q.name.is_empty()
                && next.saved.names.get(&q.symbol).map(String::as_str) != Some(q.name.as_str())
            {
                next.saved.names.insert(q.symbol.clone(), q.name.clone());
                names_changed = true;
            }
            next.quotes.insert(q.symbol.clone(), q);
        }
        next.saved.events.truncate(500);
        next.last_fetch = Some(at);
        next.error = None;
        next.failures = 0;
        if names_changed || !emitted.is_empty() || at - next.last_intraday_save >= 60_000 {
            next.revision += 1;
            match next.persist() {
                Ok(()) => next.last_intraday_save = at,
                Err(e) => next.error = Some(format!("事件儲存失敗，稍後重試：{e}")),
            }
        }
        *self = next;
        emitted
    }
}

/// `open` is whether the session is open now (hours and official calendar), decided by the caller.
pub fn evaluate(r: &mut Rule, q: &Quote, at: i64, demo: bool, open: bool) -> Option<Event> {
    let fresh = q.source_at <= at + 5000
        && at - q.source_at <= FRESH_MS
        && q.date == local(at).format("%Y%m%d").to_string();
    if !fresh || (!demo && !open) || r.channel == "off" {
        r.last_value = None;
        r.last_at = None;
        return None;
    }
    let Some(value) = (match r.metric.as_str() {
        "price" => q.price,
        "mid" => q.mid,
        "midChangePercent" => q.mid_change_percent,
        _ => q.change_percent,
    }) else {
        r.last_value = None;
        r.last_at = None;
        return None;
    };
    if r.last_at.is_some_and(|t| q.source_at <= t) {
        return None;
    }
    let continuous = r.last_day.as_deref() == Some(&q.date)
        && r.last_at.is_some_and(|t| q.source_at - t <= FRESH_MS);
    // The day's high and low in the rule's unit. Trade-price rules only: the book mid has no such
    // record. They catch a threshold touched between two updates, which the latest price alone
    // would miss.
    let (high, low) = match r.metric.as_str() {
        "price" => (q.day_high, q.day_low),
        "changePercent" => {
            let pct = |x: Option<f64>| x.zip(q.previous).map(|(v, y)| (v / y - 1.) * 100.);
            (pct(q.day_high), pct(q.day_low))
        }
        _ => (None, None),
    };
    let touched = continuous
        && if r.direction == "above" {
            high.is_some_and(|h| h >= r.threshold) && r.last_high.is_some_and(|p| p < r.threshold)
        } else {
            low.is_some_and(|l| l <= r.threshold) && r.last_low.is_some_and(|p| p > r.threshold)
        };
    let crossed = continuous
        && r.last_value.is_some_and(|v| {
            if r.direction == "above" {
                v < r.threshold && value >= r.threshold
            } else {
                v > r.threshold && value <= r.threshold
            }
        });
    r.last_value = Some(value);
    r.last_at = Some(q.source_at);
    r.last_day = Some(q.date.clone());
    r.last_high = high;
    r.last_low = low;
    let via_touch = !crossed && touched;
    if !(crossed || via_touch)
        || r.fired_day.as_deref() == Some(&q.date)
        || r.repeat == "once" && r.fired_day.is_some()
    {
        return None;
    }
    r.fired_day = Some(q.date.clone());
    if r.repeat == "once" {
        r.enabled = false;
    }
    let metric = match r.metric.as_str() {
        "price" => "成交價",
        "mid" => "買賣中間價（委託價，非成交價）",
        "midChangePercent" => "漲跌幅（買賣中間價，非成交價）",
        _ => "漲跌幅",
    };
    let direction = if r.direction == "above" {
        "升至"
    } else {
        "降至"
    };
    let unit = if matches!(r.metric.as_str(), "price" | "mid") {
        "元"
    } else {
        "%"
    };
    let (text, value) = if via_touch {
        let (extreme, side) = if r.direction == "above" {
            (high.unwrap_or(value), "高點")
        } else {
            (low.unwrap_or(value), "低點")
        };
        (
            format!(
                "{} {} · {}期間內曾{} {:.2}{}（門檻 {:.2}；兩次更新之間的{}，非即時價）",
                display_symbol(&q.symbol),
                q.name,
                metric,
                direction,
                extreme,
                unit,
                r.threshold,
                side
            ),
            extreme,
        )
    } else {
        (
            format!(
                "{} {} · {}{} {:.2}{}（門檻 {:.2}）",
                display_symbol(&q.symbol),
                q.name,
                metric,
                direction,
                value,
                unit,
                r.threshold
            ),
            value,
        )
    };
    Some(Event {
        id: uuid::Uuid::new_v4().to_string(),
        rule_id: r.id.clone(),
        symbol: q.symbol.clone(),
        text,
        source_at: q.source_at,
        detected_at: at,
        channel: r.channel.clone(),
        value,
        threshold: r.threshold,
        source: q.source.clone(),
    })
}

pub fn demo_quotes(codes: &[String], tick: u64) -> Vec<Quote> {
    let at = now();
    codes
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let base = if c == "t00" {
                24000.
            } else if c == "0050" {
                60.
            } else {
                1000. + i as f64 * 10.
            };
            let p = base + (tick as f64 / 2.).sin() * base * 0.02;
            Quote {
                symbol: c.clone(),
                name: match c.as_str() {
                    "2330" => "台積電".into(),
                    "0050" => "元大台灣50".into(),
                    "t00" => "加權指數".into(),
                    other => index_name(other).unwrap_or("模擬標的").into(),
                },
                price: Some((p * 100.).round() / 100.),
                previous: Some(base),
                bid: Some(p - 0.5),
                ask: Some(p + 0.5),
                change_percent: Some((p / base - 1.) * 100.),
                mid: Some(p),
                mid_change_percent: Some((p / base - 1.) * 100.),
                trade_at: Some(at),
                day_high: Some(p),
                day_low: Some(p),
                source_at: at,
                received_at: at,
                date: local(at).format("%Y%m%d").to_string(),
                source: "合成示範資料".into(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ev(r: &mut Rule, q: &Quote, at: i64, demo: bool) -> Option<Event> {
        evaluate(r, q, at, demo, trading(at))
    }
    fn q(p: Option<f64>, t: i64) -> Quote {
        Quote {
            symbol: "2330".into(),
            name: "台積電".into(),
            price: p,
            bid: Some(9999.),
            ask: None,
            previous: Some(100.),
            change_percent: p.map(|v| v - 100.),
            mid: None,
            mid_change_percent: None,
            trade_at: None,
            day_high: None,
            day_low: None,
            source_at: t,
            received_at: t,
            date: local(t).format("%Y%m%d").to_string(),
            source: "test".into(),
        }
    }
    fn rule() -> Rule {
        Rule {
            id: "r1".into(),
            symbol: "2330".into(),
            metric: "price".into(),
            direction: "above".into(),
            threshold: 105.,
            channel: "strip".into(),
            repeat: "daily".into(),
            enabled: true,
            last_value: None,
            last_at: None,
            last_day: None,
            last_high: None,
            last_low: None,
            fired_day: None,
        }
    }
    fn ts() -> i64 {
        FixedOffset::east_opt(28800)
            .unwrap()
            .with_ymd_and_hms(2026, 11, 6, 10, 0, 0)
            .unwrap()
            .timestamp_millis()
    }
    #[test]
    fn crossing_and_restart_latch() {
        let t = ts();
        let mut r = rule();
        assert!(ev(&mut r, &q(Some(106.), t), t, false).is_none());
        assert!(ev(&mut r, &q(Some(100.), t + 5000), t + 5000, false).is_none());
        assert!(ev(&mut r, &q(Some(106.), t + 10000), t + 10000, false).is_some());
        let saved = serde_json::to_string(&r).unwrap();
        let mut restored: Rule = serde_json::from_str(&saved).unwrap();
        ev(&mut restored, &q(Some(100.), t + 15000), t + 15000, false);
        assert!(ev(&mut restored, &q(Some(106.), t + 20000), t + 20000, false).is_none());
    }
    #[test]
    fn stale_missing_and_out_of_order_never_cross() {
        let t = ts();
        let mut r = rule();
        ev(&mut r, &q(Some(100.), t), t, false);
        assert!(ev(&mut r, &q(Some(106.), t - 5000), t, false).is_none());
        assert!(ev(&mut r, &q(Some(106.), t + 5000), t + 200000, false).is_none());
        assert!(ev(&mut r, &q(Some(106.), t + 205000), t + 205000, false).is_none());
        ev(&mut r, &q(None, t + 210000), t + 210000, false);
        assert!(ev(&mut r, &q(Some(106.), t + 215000), t + 215000, false).is_none());
    }
    #[test]
    fn day_reset_and_off_session() {
        let t = ts();
        let mut r = rule();
        ev(&mut r, &q(Some(100.), t), t, false);
        let tomorrow = t + 86400000;
        assert!(ev(&mut r, &q(Some(106.), tomorrow), tomorrow, false).is_none());
        let close = t + 4 * 3600000;
        assert!(ev(&mut r, &q(Some(110.), close), close, false).is_none());
    }
    #[test]
    fn missing_trade_not_book() {
        let t = ts();
        let v = json!({"rtcode":"0000","msgArray":[{"c":"2330","n":"台積電","d":"20261106","t":"10:00:00","z":"-","y":"100","b":"110_109_","a":"111_"}]});
        let parsed = parse_mis(&v, t).unwrap();
        assert_eq!(parsed[0].price, None);
        assert_eq!(parsed[0].bid, Some(110.));
        assert_eq!(parsed[0].change_percent, None);
    }
    #[test]
    fn symbols_and_sessions() {
        assert_eq!(symbol("0050.TW"), Some("0050".into()));
        assert!(symbol("2330&x=y").is_none());
        assert_eq!(symbol("大盤"), Some("t00".into()));
        assert!(trading(ts()));
        assert!(!trading(ts() + 86400000));
    }
    #[test]
    fn persistence_validation_and_pause_are_shared() {
        let dir = std::env::temp_dir().join(format!("smallstocks-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Core::load(dir.join("state.json"), true).unwrap();
        let bad = c.execute(
            "upsert_rule",
            json!({"symbol":"2330","metric":"volume","direction":"above","threshold":5}),
        );
        assert!(bad.is_err());
        assert!(c.saved.rules.is_empty());
        c.execute("upsert_rule",json!({"symbol":"2330","metric":"price","direction":"above","threshold":105,"repeat":"daily","channel":"strip"})).unwrap();
        assert!(c
            .execute("set_watchlist", json!({"symbols":["0050"]}))
            .is_err());
        let t = ts();
        c.ingest(vec![q(Some(100.), t)], t);
        let events = c.ingest(vec![q(Some(106.), t + 5000)], t + 5000);
        assert_eq!(events.len(), 1);
        let mut restored = Core::load(dir.join("state.json"), true).unwrap();
        assert_eq!(restored.saved.events.len(), 1);
        assert_eq!(
            restored.saved.rules[0].fired_day.as_deref(),
            Some("20261106")
        );
        assert!(restored.saved.rules[0].last_value.is_none());
        restored
            .execute("set_settings", json!({"paused":true}))
            .unwrap();
        assert!(restored.saved.settings.paused);
        assert!(restored
            .execute("set_settings", json!({"refreshSeconds":1}))
            .is_err());
        std::fs::remove_file(dir.join("state.json")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn failed_save_does_not_change_live_rules() {
        let path = std::env::temp_dir()
            .join(format!("missing-{}", uuid::Uuid::new_v4()))
            .join("state.json");
        let mut c = Core::load(path, true).unwrap();
        assert!(c
            .execute("set_watchlist", json!({"symbols":["6488"]}))
            .is_err());
        assert_eq!(c.saved.watchlist, vec!["t00", "2330", "0050"]);
    }
    #[test]
    fn failed_save_keeps_quotes_and_events_and_retries() {
        let dir = std::env::temp_dir().join(format!("smallstocks-ingest-{}", uuid::Uuid::new_v4()));
        let path = dir.join("state.json");
        // Directory does not exist yet, so every save fails.
        let mut c = Core::load(path.clone(), true).unwrap();
        c.saved.rules.push(rule());
        let t = ts();
        c.ingest(vec![q(Some(100.), t)], t);
        let events = c.ingest(vec![q(Some(106.), t + 5000)], t + 5000);
        assert_eq!(events.len(), 1, "crossing must still be reported");
        assert_eq!(c.quotes["2330"].price, Some(106.));
        assert_eq!(c.saved.events.len(), 1);
        assert!(c
            .error
            .as_deref()
            .is_some_and(|e| e.contains("事件儲存失敗")));
        // Once the folder is writable the next poll saves without losing the event.
        std::fs::create_dir_all(&dir).unwrap();
        c.ingest(vec![q(Some(107.), t + 10000)], t + 10000);
        assert_eq!(Core::load(path, true).unwrap().saved.events.len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn spent_once_rule_can_be_rearmed_but_daily_latch_is_kept() {
        let dir = std::env::temp_dir().join(format!("smallstocks-once-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Core::load(dir.join("state.json"), true).unwrap();
        let t = ts();
        let args = |id: &str, repeat: &str, enabled: bool| json!({"id":id,"symbol":"2330","metric":"price","direction":"above","threshold":105,"repeat":repeat,"channel":"strip","enabled":enabled});
        let mut once = rule();
        once.id = "once".into();
        once.repeat = "once".into();
        let mut daily = rule();
        daily.id = "daily".into();
        c.saved.rules.extend([once, daily]);
        c.ingest(vec![q(Some(100.), t)], t);
        assert_eq!(c.ingest(vec![q(Some(106.), t + 5000)], t + 5000).len(), 2);
        assert!(
            !c.saved
                .rules
                .iter()
                .find(|r| r.id == "once")
                .unwrap()
                .enabled
        );
        c.execute("upsert_rule", args("once", "once", true))
            .unwrap();
        c.execute("upsert_rule", args("daily", "daily", true))
            .unwrap();
        let get = |c: &Core, id: &str| c.saved.rules.iter().find(|r| r.id == id).unwrap().clone();
        assert!(
            get(&c, "once").fired_day.is_none(),
            "re-enabled once rule is armed again"
        );
        assert!(
            get(&c, "daily").fired_day.is_some(),
            "daily latch survives edits"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    fn calendar_with(closures: &[(&str, &str)], fetched_at: i64) -> calendar::Calendar {
        calendar::Calendar {
            year: 2026,
            fetched_at,
            closures: closures
                .iter()
                .map(|(date, name)| calendar::Closure {
                    date: (*date).into(),
                    name: (*name).into(),
                })
                .collect(),
        }
    }
    #[test]
    fn official_closure_closes_a_weekday_session_and_silences_alerts() {
        let t = ts();
        let mut c = Core::load(
            std::env::temp_dir()
                .join(format!("cal-test-{}", uuid::Uuid::new_v4()))
                .join("state.json"),
            false,
        )
        .unwrap();
        // 2026-11-06 is a Friday and normally open.
        assert!(c.market_open(t));
        assert_eq!(c.closed_reason(t), None);
        c.saved.calendars.insert(
            "2026".into(),
            calendar_with(&[("2026-11-06", "測試休市日")], t),
        );
        assert!(!c.market_open(t));
        assert_eq!(c.closed_reason(t).as_deref(), Some("測試休市日"));
        // A weekend is closed without needing a reason, and other days stay open.
        assert!(!c.market_open(t + 86_400_000));
        assert_eq!(c.closed_reason(t + 86_400_000), None);
        assert!(c.market_open(t - 86_400_000));
        let mut r = rule();
        assert!(evaluate(&mut r, &q(Some(100.), t), t, false, c.market_open(t)).is_none());
        assert!(evaluate(
            &mut r,
            &q(Some(106.), t + 5000),
            t + 5000,
            false,
            c.market_open(t)
        )
        .is_none());
        let snap = c.snapshot();
        assert_eq!(snap["calendar"]["years"][0], "2026");
    }
    #[test]
    fn polling_continues_through_the_closing_auction_until_the_delayed_close_limit() {
        let c = Core::load(
            std::env::temp_dir()
                .join(format!("poll-window-{}", uuid::Uuid::new_v4()))
                .join("state.json"),
            false,
        )
        .unwrap();
        let at = |h, m, s| {
            FixedOffset::east_opt(28800)
                .unwrap()
                .with_ymd_and_hms(2026, 11, 6, h, m, s)
                .unwrap()
                .timestamp_millis()
        };
        assert!(!c.polling_window(at(8, 50, 0)));
        assert!(c.polling_window(at(8, 55, 0)));
        assert!(c.polling_window(at(13, 29, 55)));
        // The closing price is published at 13:30 and a postponed close can run to 13:33.
        assert!(c.polling_window(at(13, 30, 5)));
        assert!(c.polling_window(at(13, 35, 59)));
        assert!(!c.polling_window(at(13, 36, 0)));
        // Alerts still only run inside the session.
        assert!(!c.market_open(at(13, 30, 5)));
        assert!(c.market_open(at(13, 29, 55)));
        // A weekend is never polled.
        assert!(!c.polling_window(at(10, 0, 0) + 86_400_000));
    }
    #[test]
    fn calendar_is_fetched_when_missing_stale_or_before_open_and_retried_slowly() {
        let t = ts(); // Friday 2026-11-06 10:00 Taipei
        let mut c = Core::load(
            std::env::temp_dir()
                .join(format!("cal-due-{}", uuid::Uuid::new_v4()))
                .join("state.json"),
            false,
        )
        .unwrap();
        // November: this year and next year are both wanted, because next year's table appears late.
        assert_eq!(c.calendar_due(t), vec![2026, 2027]);
        c.calendar_errors.insert(2027, ("尚未公布".into(), t));
        assert_eq!(c.calendar_due(t + 60_000), vec![2026]);
        assert_eq!(c.calendar_due(t + 3_600_000 + 1), vec![2026, 2027]);
        c.saved.calendars.insert(
            "2026".into(),
            calendar_with(&[("2026-12-25", "行憲紀念日")], t),
        );
        assert_eq!(c.calendar_due(t + 3_600_000), vec![2027]);
        assert_eq!(c.calendar_due(t + 6 * 3_600_000), vec![2026, 2027]);
        // Weekday 08:40 the next Monday: a table fetched on Friday is refreshed once before open.
        let early = FixedOffset::east_opt(28800)
            .unwrap()
            .with_ymd_and_hms(2026, 11, 9, 8, 40, 0)
            .unwrap()
            .timestamp_millis();
        assert!(c.calendar_due(early).contains(&2026));
        c.saved.calendars.get_mut("2026").unwrap().fetched_at = early - 60_000;
        assert!(!c.calendar_due(early).contains(&2026));
    }
    #[test]
    fn last_trade_comes_from_the_trade_block_when_z_is_a_dash_and_never_from_the_book() {
        let t = ts();
        let with_trade = json!({"rtcode":"0000","msgArray":[{
            "c":"2330","n":"台積電","d":"20261106","t":"10:00:00","z":"-","y":"100","b":"99_","a":"101_",
            "trade":{"t":"09:59:41","v":12,"z":"100.5000","ft":20}}]});
        let p = parse_mis(&with_trade, t).unwrap();
        assert_eq!(p[0].price, Some(100.5), "the last trade is the price");
        assert!((p[0].change_percent.unwrap() - 0.5).abs() < 1e-9);
        assert_eq!(p[0].mid, Some(100.), "the book mid stays a separate value");
        assert_eq!(
            p[0].trade_at,
            Some(ts() - 19_000),
            "the trade's own time is kept"
        );
        // A real z wins over the trade block.
        let both = json!({"rtcode":"0000","msgArray":[{
            "c":"2330","n":"x","d":"20261106","t":"10:00:00","z":"101.0000","y":"100",
            "trade":{"t":"09:59:41","z":"100.5000"}}]});
        assert_eq!(parse_mis(&both, t).unwrap()[0].price, Some(101.));
        // Neither z nor a usable trade block: no price, and the bid/ask is not promoted to one.
        let none = json!({"rtcode":"0000","msgArray":[{
            "c":"2330","n":"x","d":"20261106","t":"10:00:00","z":"-","y":"100","b":"99_","a":"101_","trade":null}]});
        let n = parse_mis(&none, t).unwrap();
        assert_eq!(n[0].price, None);
        assert_eq!(n[0].trade_at, None);
        let junk = json!({"rtcode":"0000","msgArray":[{
            "c":"2330","n":"x","d":"20261106","t":"10:00:00","z":"-","y":"100","trade":{"t":"09:59:41","z":"-"}}]});
        assert_eq!(parse_mis(&junk, t).unwrap()[0].price, None);
    }
    #[test]
    fn a_touch_between_two_updates_is_caught_from_the_day_high_and_low() {
        let t = ts();
        let snap = |price: f64, high: f64, low: f64, at: i64| {
            let mut x = q(Some(price), at);
            x.day_high = Some(high);
            x.day_low = Some(low);
            x
        };
        // Above 105: the day high rises through the threshold between two updates while the latest
        // price is back under it.
        let mut up = rule();
        assert!(ev(&mut up, &snap(101., 103., 99., t), t, false).is_none());
        let touched = ev(
            &mut up,
            &snap(102., 107., 99., t + 60_000),
            t + 60_000,
            false,
        )
        .expect("the high reached the threshold");
        assert!(touched.text.contains("期間內曾") && touched.text.contains("非即時價"));
        assert_eq!(touched.value, 107.);
        assert!(
            ev(
                &mut up,
                &snap(108., 108., 99., t + 120_000),
                t + 120_000,
                false
            )
            .is_none(),
            "once per day"
        );
        // Already above when first seen: nothing to report.
        let mut already = rule();
        assert!(ev(&mut already, &snap(106., 107., 99., t), t, false).is_none());
        assert!(ev(
            &mut already,
            &snap(104., 107., 99., t + 60_000),
            t + 60_000,
            false
        )
        .is_none());
        // Below 95: the day low reaches the threshold.
        let mut down = rule();
        down.direction = "below".into();
        down.threshold = 95.;
        assert!(ev(&mut down, &snap(100., 101., 97., t), t, false).is_none());
        let low = ev(
            &mut down,
            &snap(99., 101., 94., t + 60_000),
            t + 60_000,
            false,
        )
        .expect("the low reached the threshold");
        assert_eq!(low.value, 94.);
        assert!(low.text.contains("低點"));
        // A book-based rule has no day extremes and is not touched by them.
        let mut mid = rule();
        mid.metric = "mid".into();
        let mut a = snap(101., 107., 99., t);
        a.mid = Some(101.);
        let mut b = snap(102., 110., 99., t + 60_000);
        b.mid = Some(102.);
        assert!(ev(&mut mid, &a, t, false).is_none());
        assert!(ev(&mut mid, &b, t + 60_000, false).is_none());
    }
    #[test]
    fn book_mid_comes_only_from_a_sane_two_sided_book_and_is_never_a_trade() {
        let t = ts();
        let row = |b: &str, a: &str| json!({"rtcode":"0000","msgArray":[{"c":"2330","n":"台積電","d":"20261106","t":"10:00:00","z":"-","y":"100","b":b,"a":a}]});
        let p = parse_mis(&row("99_98_", "101_102_"), t).unwrap();
        assert_eq!(p[0].price, None);
        assert_eq!(p[0].mid, Some(100.));
        assert_eq!(p[0].mid_change_percent, Some(0.));
        assert_eq!(parse_mis(&row("0.0000_", "101_"), t).unwrap()[0].mid, None);
        assert_eq!(parse_mis(&row("99_", "-"), t).unwrap()[0].mid, None);
        assert_eq!(
            parse_mis(&row("102_", "101_"), t).unwrap()[0].mid,
            None,
            "a crossed book is not a price"
        );
    }
    #[test]
    fn sidecar_quotes_recompute_mid_and_change_and_use_local_receive_time() {
        let t = ts();
        // The wire claims a bogus mid and changePercent; both must be recomputed
        // from bid/ask and previous close, and receivedAt must be the local clock.
        let v = json!({"quotes":[{"symbol":"2330","name":"台積電","price":150.,"bid":148.,"ask":152.,"previous":100.,"changePercent":99.,"mid":55.,"tradeAt":t,"dayHigh":155.,"dayLow":145.,"sourceAt":t,"receivedAt":1,"date":"20261106","source":"Shioaji"}]});
        let p = parse_sidecar(&v, t + 1234).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].price, Some(150.));
        assert_eq!(p[0].mid, Some(150.));
        assert_eq!(p[0].change_percent, Some(50.));
        assert_eq!(p[0].mid_change_percent, Some(50.));
        assert_eq!(p[0].received_at, t + 1234);
        assert_eq!(p[0].source_at, t);
        assert_eq!(p[0].trade_at, Some(t));
        assert_eq!(p[0].source, "Shioaji");
    }
    #[test]
    fn sidecar_quotes_without_source_time_or_valid_date_are_dropped() {
        let t = ts();
        let v = json!({"quotes":[
            {"symbol":"2330","price":101.,"sourceAt":0,"date":"20261106"},
            {"symbol":"2317","price":50.,"sourceAt":t,"date":"2026-11-06"},
            {"symbol":"0050","price":100.,"sourceAt":t,"date":"20261106"}
        ]});
        let p = parse_sidecar(&v, t).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].symbol, "0050");
        assert!(parse_sidecar(&json!({"nope":[]}), t).is_err());
    }
    #[test]
    fn fill_missing_prefers_primary_quotes() {
        let t = ts();
        let mut a = q(Some(100.), t);
        a.symbol = "2330".into();
        a.source = "Shioaji".into();
        let mut b = q(Some(90.), t);
        b.symbol = "2330".into();
        let mut c = q(Some(50.), t);
        c.symbol = "2317".into();
        let merged = fill_missing(vec![a], vec![b, c]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].price, Some(100.));
        assert_eq!(merged[1].symbol, "2317");
    }
    #[test]
    fn settings_default_to_mis_and_accept_shioaji_source() {
        // Settings saved before quoteSource existed must still load, as MIS.
        let old: Settings = serde_json::from_str(
            r#"{"alwaysOnTop":true,"showCodes":false,"appearance":"small","paused":false,"refreshSeconds":15}"#,
        )
        .unwrap();
        assert_eq!(old.quote_source, "mis");
        assert_eq!(old.sidecar_port, DEFAULT_SIDECAR_PORT);
        let dir = std::env::temp_dir().join(format!("smallstocks-sidecar-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Core::load(dir.join("state.json"), true).unwrap();
        c.execute("set_settings", json!({"quoteSource":"shioaji","sidecarPort":18766}))
            .unwrap();
        assert_eq!(c.saved.settings.quote_source, "shioaji");
        assert_eq!(c.saved.settings.sidecar_port, 18766);
        assert!(c.execute("set_settings", json!({"quoteSource":"yahoo"})).is_err());
        assert!(c.execute("set_settings", json!({"sidecarPort":80})).is_err());
    }
    #[test]
    fn sidecar_token_is_read_trimmed_from_the_config_dir() {
        let dir = std::env::temp_dir().join(format!("smallstocks-token-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(sidecar_token(&dir), None);
        std::fs::write(dir.join("sidecar-token.txt"), "  abc123\n").unwrap();
        assert_eq!(sidecar_token(&dir).as_deref(), Some("abc123"));
    }
    #[test]
    fn mid_rules_cross_on_the_book_and_trade_rules_stay_silent_without_a_trade() {
        let t = ts();
        let book = |mid: f64, at: i64| {
            let mut x = q(None, at);
            x.mid = Some(mid);
            x.mid_change_percent = Some(mid - 100.);
            x
        };
        let mut mid_rule = rule();
        mid_rule.metric = "mid".into();
        let mut trade_rule = rule();
        let first = book(100., t);
        assert!(ev(&mut mid_rule, &first, t, false).is_none());
        assert!(ev(&mut trade_rule, &first, t, false).is_none());
        // Stock snapshots come about two minutes apart; that is still one continuous crossing.
        let second = book(106., t + 120_000);
        let event = ev(&mut mid_rule, &second, t + 125_000, false)
            .expect("a two-minute cadence still crosses");
        assert!(event.text.contains("買賣中間價") && event.text.contains("非成交價"));
        assert!(
            ev(&mut trade_rule, &second, t + 125_000, false).is_none(),
            "no trade price, no trade alert"
        );
    }
    #[test]
    fn rules_accept_book_metrics_and_reject_unknown_ones() {
        let dir = std::env::temp_dir().join(format!("smallstocks-mid-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Core::load(dir.join("state.json"), true).unwrap();
        for metric in ["mid", "midChangePercent"] {
            c.execute(
                "upsert_rule",
                json!({"symbol":"2330","metric":metric,"direction":"above","threshold":1}),
            )
            .unwrap();
        }
        assert!(c
            .execute(
                "upsert_rule",
                json!({"symbol":"2330","metric":"mid","direction":"above","threshold":0}),
            )
            .is_err());
        assert!(c
            .execute(
                "upsert_rule",
                json!({"symbol":"2330","metric":"volume","direction":"above","threshold":1}),
            )
            .is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn topmost_defaults_migrate_and_changes_persist() {
        let old: Settings = serde_json::from_value(json!({
            "appearance":"small", "paused":false, "refreshSeconds":15
        }))
        .unwrap();
        assert!(old.always_on_top);
        let dir =
            std::env::temp_dir().join(format!("smallstocks-topmost-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let mut core = Core::load(path.clone(), true).unwrap();
        core.execute("set_settings", json!({"alwaysOnTop":false}))
            .unwrap();
        assert!(
            !Core::load(path.clone(), true)
                .unwrap()
                .saved
                .settings
                .always_on_top
        );
        assert!(core
            .execute("set_settings", json!({"alwaysOnTop":"false"}))
            .is_err());
        assert!(!core.saved.settings.always_on_top);
        core.execute("set_settings", json!({"appearance":"terminal"}))
            .unwrap();
        assert!(!core.saved.settings.always_on_top);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn old_settings_keep_numbers_by_default() {
        let cfg: Settings =
            serde_json::from_value(json!({"appearance":"gray","paused":false,"refreshSeconds":15}))
                .unwrap();
        assert_eq!(cfg.row_display, "numbers");
        assert!(!cfg.show_codes);
        assert_eq!(cfg.appearance, "gray");
    }
    #[test]
    fn local_minutes_are_partial_and_ignore_missing_stale_future_trades() {
        let mut c = Core::load(
            std::env::temp_dir()
                .join(format!("line-test-{}", uuid::Uuid::new_v4()))
                .join("state.json"),
            false,
        )
        .unwrap();
        let t = ts();
        c.record_intraday(&q(None, t), t);
        assert!(c.saved.local_intraday.is_empty());
        c.record_intraday(&q(Some(100.), t), t);
        c.record_intraday(&q(Some(101.), t + 5000), t + 5000);
        assert_eq!(c.saved.local_intraday["2330"].points.len(), 1);
        assert_eq!(c.saved.local_intraday["2330"].points[0].price, Some(101.));
        c.record_intraday(&q(Some(102.), t + 60000), t + 60000);
        assert_eq!(c.saved.local_intraday["2330"].points.len(), 2);
        assert!(c.saved.local_intraday["2330"].partial);
        c.record_intraday(&q(Some(103.), t + 120000), t + 240000);
        c.record_intraday(&q(Some(103.), t + 600000), t);
        assert_eq!(c.saved.local_intraday["2330"].points.len(), 2);
        let monday = t + 3 * 86400000;
        c.record_intraday(&q(Some(110.), monday), monday);
        assert_eq!(c.saved.local_intraday["2330"].points.len(), 1);
    }
    #[test]
    fn saved_flat_watchlist_migrates_into_a_first_group_on_load() {
        let dir =
            std::env::temp_dir().join(format!("smallstocks-migrate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        // A pre-groups save file: a flat watchlist and no groups key at all.
        std::fs::write(
            &path,
            r#"{"watchlist":["2330","0050","2330"],"settings":{"appearance":"small","paused":false,"refreshSeconds":15}}"#,
        )
        .unwrap();
        let mut c = Core::load(path.clone(), true).unwrap();
        assert_eq!(c.saved.groups.len(), 1);
        assert_eq!(c.saved.groups[0].name, "自選1");
        assert_eq!(
            c.saved.groups[0].codes,
            vec!["2330", "0050"],
            "duplicates fold away"
        );
        assert_eq!(c.saved.active_group, 0);
        assert_eq!(
            c.saved.watchlist,
            vec!["2330", "0050"],
            "watchlist mirrors the active group"
        );
        let snap = c.snapshot();
        assert_eq!(snap["groups"][0]["name"], "自選1");
        assert_eq!(snap["groups"][0]["codes"][1], "0050");
        assert_eq!(snap["activeGroup"], 0);
        assert_eq!(snap["watchlist"][0], "2330");
        // Saving keeps the groups, so a second load does not migrate again or lose them.
        c.persist().unwrap();
        let reloaded = Core::load(path, true).unwrap();
        assert_eq!(reloaded.saved.groups.len(), 1);
        assert_eq!(reloaded.saved.groups[0].codes, vec!["2330", "0050"]);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn watch_group_commands_add_switch_rename_delete_and_cap() {
        let dir =
            std::env::temp_dir().join(format!("smallstocks-groups-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Core::load(dir.join("state.json"), true).unwrap();
        assert_eq!(c.saved.groups.len(), 1);
        assert_eq!(c.saved.groups[0].name, "自選1");
        c.execute("add_watch_group", json!({})).unwrap();
        assert_eq!(c.saved.groups.len(), 2);
        assert_eq!(c.saved.groups[1].name, "自選2");
        assert_eq!(c.saved.active_group, 1, "a new group becomes active");
        assert!(
            c.saved.watchlist.is_empty(),
            "the mirror follows the active group"
        );
        c.execute("set_watchlist", json!({"symbols":["6488"]})).unwrap();
        assert_eq!(c.saved.groups[1].codes, vec!["6488"]);
        assert_eq!(
            c.saved.groups[0].codes,
            vec!["t00", "2330", "0050"],
            "set_watchlist edits only the active group"
        );
        c.execute("rename_watch_group", json!({"index":1,"name":"半導體"}))
            .unwrap();
        assert_eq!(c.saved.groups[1].name, "半導體");
        assert!(
            c.execute("rename_watch_group", json!({"index":1,"name":"自選1"}))
                .is_err(),
            "names stay unique"
        );
        assert!(c
            .execute("rename_watch_group", json!({"index":1,"name":"  "}))
            .is_err());
        c.execute("set_active_group", json!({"index":0})).unwrap();
        assert_eq!(c.saved.watchlist, vec!["t00", "2330", "0050"]);
        assert!(c.execute("set_active_group", json!({"index":9})).is_err());
        // Ten groups in total at most.
        for _ in 0..8 {
            c.execute("add_watch_group", json!({})).unwrap();
        }
        assert_eq!(c.saved.groups.len(), 10);
        assert!(c.execute("add_watch_group", json!({})).is_err());
        // The per-group cap keeps the existing message.
        let many: Vec<String> = (0..31).map(|i| format!("1{i:03}")).collect();
        let err = c.execute("set_watchlist", json!({"symbols": many})).unwrap_err();
        assert!(err.contains("30"), "unexpected error: {err}");
        // Deleting groups walks the active index back; the last group cannot go.
        while c.saved.groups.len() > 1 {
            let idx = c.saved.active_group;
            c.execute("delete_watch_group", json!({"index": idx})).unwrap();
        }
        assert!(
            c.execute("delete_watch_group", json!({"index":0})).is_err(),
            "at least one group remains"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn monitoring_covers_the_union_of_all_groups() {
        let dir =
            std::env::temp_dir().join(format!("smallstocks-union-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Core::load(dir.join("state.json"), true).unwrap();
        let t = ts();
        // 6488 lives in a second group that is not the visible tab; 2330 stays in 自選1.
        c.execute("add_watch_group", json!({"name":"觀察"})).unwrap();
        c.execute("set_watchlist", json!({"symbols":["6488"]})).unwrap();
        c.execute("set_active_group", json!({"index":0})).unwrap();
        assert_eq!(c.saved.all_codes(), vec!["t00", "2330", "0050", "6488"]);
        let mut other = q(Some(50.), t);
        other.symbol = "6488".into();
        c.ingest(vec![other], t);
        assert_eq!(
            c.quotes["6488"].price,
            Some(50.),
            "quotes land for non-active groups too"
        );
        c.saved.rules.push(rule());
        c.ingest(vec![q(Some(100.), t)], t);
        let events = c.ingest(vec![q(Some(106.), t + 5000)], t + 5000);
        assert_eq!(events.len(), 1, "rules fire regardless of the visible tab");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn position_commands_add_merge_validate_and_delete() {
        let dir =
            std::env::temp_dir().join(format!("smallstocks-positions-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Core::load(dir.join("state.json"), true).unwrap();
        assert!(c.saved.positions.is_empty());
        c.execute(
            "add_lot",
            json!({"code":"2330","date":"2026-11-10","price":2395.0,"shares":64}),
        )
        .unwrap();
        // A second lot for the same code appends to the same position.
        c.execute("add_lot", json!({"code":"2330","price":2300.0,"shares":36}))
            .unwrap();
        assert_eq!(c.saved.positions.len(), 1);
        assert_eq!(c.saved.positions[0].lots.len(), 2);
        assert_eq!(c.saved.positions[0].lots[1].date, "");
        // US fractional shares and Yahoo normalization ride the same path.
        c.execute("add_lot", json!({"code":"nvda","price":168.8,"shares":9.43}))
            .unwrap();
        assert_eq!(c.saved.positions[1].code, "NVDA");
        // Validation: bad code, bad date, non-positive numbers all refuse.
        assert!(c
            .execute("add_lot", json!({"code":"!!","price":1,"shares":1}))
            .is_err());
        assert!(c
            .execute(
                "add_lot",
                json!({"code":"0050","date":"2026-13-01","price":100,"shares":1})
            )
            .is_err());
        assert!(c
            .execute(
                "add_lot",
                json!({"code":"0050","date":"2026-1-05","price":100,"shares":1})
            )
            .is_err());
        assert!(c
            .execute("add_lot", json!({"code":"0050","price":0,"shares":1}))
            .is_err());
        assert!(c
            .execute("add_lot", json!({"code":"0050","price":100,"shares":-2}))
            .is_err());
        assert_eq!(c.saved.positions.len(), 2, "refused lots change nothing");
        // Deleting the last lot of a position removes the position itself.
        c.execute("delete_lot", json!({"code":"NVDA","index":0})).unwrap();
        assert_eq!(c.saved.positions.len(), 1);
        assert!(c
            .execute("delete_lot", json!({"code":"NVDA","index":0}))
            .is_err());
        assert!(c
            .execute("delete_lot", json!({"code":"2330","index":9}))
            .is_err());
        c.execute("delete_position", json!({"code":"2330"})).unwrap();
        assert!(c.saved.positions.is_empty());
        assert!(c.execute("delete_position", json!({"code":"2330"})).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn import_positions_parses_lines_and_reports_failures() {
        let dir =
            std::env::temp_dir().join(format!("smallstocks-import-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let mut c = Core::load(path.clone(), true).unwrap();
        let text = "2330, 2026-11-10, 2395, 64\n\
                    0050\t100.95\t1524\n\
                    NVDA 168.8 9.43\n\
                    \n\
                    BAD!!, 2026-11-10, 10, 10\n\
                    006208, 2026-02-30, 231.95, 3375\n\
                    006208, 231.95, -3\n\
                    006208, 231.95, 3375";
        let snap = c.execute("import_positions", json!({"text": text})).unwrap();
        assert_eq!(snap["positionsImport"]["added"], 4);
        assert_eq!(snap["positionsImport"]["failedLines"], json!([5, 6, 7]));
        assert_eq!(c.saved.positions.len(), 4);
        let tsm = c
            .saved
            .positions
            .iter()
            .find(|p| p.code == "2330")
            .unwrap();
        assert_eq!(tsm.lots[0].date, "2026-11-10");
        let etf = c
            .saved
            .positions
            .iter()
            .find(|p| p.code == "0050")
            .unwrap();
        assert_eq!(etf.lots[0].date, "", "three fields read as code, price, shares");
        // One save for the whole batch: a reload sees every imported lot.
        let reloaded = Core::load(path, true).unwrap();
        assert_eq!(reloaded.saved.positions.len(), 4);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn positions_join_the_monitoring_union() {
        let dir =
            std::env::temp_dir().join(format!("smallstocks-posunion-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Core::load(dir.join("state.json"), true).unwrap();
        let t = ts();
        // 6488 is held but in no watch group; its quotes must still land.
        c.execute("add_lot", json!({"code":"6488","price":900.0,"shares":100}))
            .unwrap();
        assert_eq!(c.saved.all_codes(), vec!["t00", "2330", "0050", "6488"]);
        assert!(c.saved.watches("6488"));
        let mut held = q(Some(950.), t);
        held.symbol = "6488".into();
        c.ingest(vec![held], t);
        assert_eq!(c.quotes["6488"].price, Some(950.));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn a_save_without_positions_still_loads() {
        let dir =
            std::env::temp_dir().join(format!("smallstocks-oldsave-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let mut c = Core::load(path.clone(), true).unwrap();
        c.persist().unwrap();
        drop(c);
        // Simulate a pre-positions state file by dropping the key entirely.
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        v.as_object_mut().unwrap().remove("positions");
        std::fs::write(&path, v.to_string()).unwrap();
        let reloaded = Core::load(path, true).unwrap();
        assert!(reloaded.saved.positions.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn a_symbol_with_an_enabled_rule_can_move_between_groups_but_not_vanish() {
        let dir = std::env::temp_dir().join(format!("smallstocks-move-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Core::load(dir.join("state.json"), true).unwrap();
        c.execute(
            "upsert_rule",
            json!({"symbol":"2330","metric":"price","direction":"above","threshold":105}),
        )
        .unwrap();
        c.execute("add_watch_group", json!({})).unwrap();
        c.execute("set_watchlist", json!({"symbols":["2330"]})).unwrap();
        c.execute("set_active_group", json!({"index":0})).unwrap();
        // 2330 leaves 自選1 here, but it is still watched through the second group.
        c.execute("set_watchlist", json!({"symbols":["0050"]})).unwrap();
        assert!(c.saved.watches("2330"));
        // Deleting that second group would drop it from the union entirely.
        assert!(c.execute("delete_watch_group", json!({"index":1})).is_err());
        assert_eq!(c.saved.groups.len(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn display_flags_default_and_persist() {
        // Settings saved before the flags existed: names shown, point change shown.
        let old: Settings = serde_json::from_value(json!({
            "appearance":"small", "paused":false, "refreshSeconds":15
        }))
        .unwrap();
        assert!(!old.hide_names);
        assert!(old.show_change_points);
        let dir =
            std::env::temp_dir().join(format!("smallstocks-display-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let mut c = Core::load(path.clone(), true).unwrap();
        assert!(!c.saved.settings.hide_names);
        assert!(c.saved.settings.show_change_points);
        c.execute(
            "set_settings",
            json!({"hideNames":true,"showChangePoints":false}),
        )
        .unwrap();
        let reloaded = Core::load(path, true).unwrap();
        assert!(reloaded.saved.settings.hide_names);
        assert!(!reloaded.saved.settings.show_change_points);
        assert!(c.execute("set_settings", json!({"hideNames":"yes"})).is_err());
        assert!(c.saved.settings.hide_names, "a failed change changes nothing");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn window_opacity_defaults_clamps_and_persists() {
        // Settings saved before the field existed are fully opaque.
        let old: Settings = serde_json::from_value(json!({
            "appearance":"small", "paused":false, "refreshSeconds":15
        }))
        .unwrap();
        assert_eq!(old.window_opacity, 1.0);
        let dir =
            std::env::temp_dir().join(format!("smallstocks-opacity-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let mut c = Core::load(path.clone(), true).unwrap();
        assert_eq!(c.saved.settings.window_opacity, 1.0);
        c.execute("set_settings", json!({"windowOpacity":0.65}))
            .unwrap();
        assert_eq!(c.saved.settings.window_opacity, 0.65);
        let reloaded = Core::load(path, true).unwrap();
        assert_eq!(reloaded.saved.settings.window_opacity, 0.65);
        // Out-of-range values clamp into 0.4–1.0 instead of failing.
        c.execute("set_settings", json!({"windowOpacity":0.1}))
            .unwrap();
        assert_eq!(c.saved.settings.window_opacity, MIN_WINDOW_OPACITY);
        c.execute("set_settings", json!({"windowOpacity":2.0}))
            .unwrap();
        assert_eq!(c.saved.settings.window_opacity, 1.0);
        assert!(
            c.execute("set_settings", json!({"windowOpacity":"50%"}))
                .is_err()
        );
        assert_eq!(
            c.saved.settings.window_opacity, 1.0,
            "a failed change changes nothing"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn index_aliases_us_tickers_and_canonical_forms_normalize() {
        // The spreadsheet shorthands of the daily report fold to Yahoo symbols.
        assert_eq!(symbol(".IXIC"), Some("^IXIC".into()));
        assert_eq!(symbol("IXIC"), Some("^IXIC".into()));
        assert_eq!(symbol("^IXIC"), Some("^IXIC".into()));
        assert_eq!(symbol(".INX"), Some("^GSPC".into()));
        assert_eq!(symbol("GSPC"), Some("^GSPC".into()));
        assert_eq!(symbol(".DJI"), Some("^DJI".into()));
        assert_eq!(symbol("SOX"), Some("^SOX".into()));
        assert_eq!(symbol("RUT"), Some("^RUT".into()));
        assert_eq!(symbol("NI225"), Some("^N225".into()));
        assert_eq!(symbol("KOSPI"), Some("^KS11".into()));
        assert_eq!(symbol("IX0001"), Some("^TWII".into()));
        // An index this table does not know still passes in Yahoo's spelling.
        assert_eq!(symbol("^VIX"), Some("^VIX".into()));
        // US tickers pass through uppercased, class-share dot included.
        assert_eq!(symbol("VOO"), Some("VOO".into()));
        assert_eq!(symbol("nvda"), Some("NVDA".into()));
        assert_eq!(symbol("BRK.B"), Some("BRK.B".into()));
        assert_eq!(symbol("brk.b"), Some("BRK.B".into()));
        // Taiwan shapes are exactly what they were before Yahoo existed.
        assert_eq!(symbol("2330"), Some("2330".into()));
        assert_eq!(symbol("00685L"), Some("00685L".into()));
        assert_eq!(symbol("0050.TW"), Some("0050".into()));
        assert_eq!(symbol("t00"), Some("t00".into()));
        // Junk stays junk.
        assert_eq!(symbol(""), None);
        assert_eq!(symbol("HELLO WORLD"), None);
        assert_eq!(symbol("BRK.BF"), None);
        assert_eq!(symbol("TOOLONG7"), None);
        assert_eq!(index_name("^SOX"), Some("PHLX Semiconductor Sector"));
        assert_eq!(index_name("^TWII"), Some("加權指數"));
        assert_eq!(index_name("VOO"), None);
        assert_eq!(display_symbol("^IXIC"), "IXIC");
        assert_eq!(display_symbol("2330"), "2330");
    }
    #[test]
    fn yahoo_futures_and_fx_codes_normalize() {
        // Futures and FX codes keep their =F / =X suffix, uppercased.
        assert_eq!(symbol("BZ=F"), Some("BZ=F".into()));
        assert_eq!(symbol("bz=f"), Some("BZ=F".into()));
        assert_eq!(symbol("CL=F"), Some("CL=F".into()));
        assert_eq!(symbol("GC=F"), Some("GC=F".into()));
        assert_eq!(symbol("TWD=X"), Some("TWD=X".into()));
        assert_eq!(symbol("twd=x"), Some("TWD=X".into()));
        assert_eq!(symbol("JPY=X"), Some("JPY=X".into()));
        // A body may carry digits (euro FX future) and stays Yahoo-routed.
        assert_eq!(symbol("6E=F"), Some("6E=F".into()));
        // Any other shape with an '=' in it is rejected.
        assert_eq!(symbol("=F"), None);
        assert_eq!(symbol("=X"), None);
        assert_eq!(symbol("BZ=Y"), None);
        assert_eq!(symbol("BZ==F"), None);
        assert_eq!(symbol("A=F=X"), None);
        assert_eq!(symbol("TOOLONG=F"), None);
        assert_eq!(symbol("BZ =F"), None);
        // The built-in names win for the four the user asked about; an
        // unlisted future simply has no built-in name.
        assert_eq!(index_name("BZ=F"), Some("布蘭特原油"));
        assert_eq!(index_name("CL=F"), Some("西德州原油"));
        assert_eq!(index_name("GC=F"), Some("黃金"));
        assert_eq!(index_name("TWD=X"), Some("美元／台幣"));
        assert_eq!(index_name("ES=F"), None);
        // Nothing to strip: futures and FX codes display exactly as stored.
        assert_eq!(display_symbol("BZ=F"), "BZ=F");
        assert_eq!(display_symbol("TWD=X"), "TWD=X");
    }
    #[test]
    fn routes_split_taiwan_from_yahoo() {
        for tw in ["2330", "00685L", "t00", "o00", "6488"] {
            assert!(is_tw_symbol(tw), "{tw} is a Taiwan code");
        }
        for y in ["VOO", "NVDA", "BRK.B", "^IXIC", "^TWII", "^N225", "BZ=F", "TWD=X", "6E=F"] {
            assert!(!is_tw_symbol(y), "{y} is Yahoo-routed");
        }
    }
    #[test]
    fn taifex_market_open_covers_both_sessions_and_the_saturday_tail() {
        // Taipei epoch millis, November 2026: Thu 12th, Fri 13th, Sat 14th,
        // Mon 16th.
        let cases: [(&str, i64, bool); 11] = [
            ("Thu 10:00 regular session", 1794448800000, true),
            ("Thu 08:45 regular open", 1794444300000, true),
            ("Thu 13:45 regular close still in session", 1794462300000, true),
            ("Thu 14:00 between sessions", 1794463200000, false),
            ("Thu 16:00 after-hours session", 1794470400000, true),
            ("Thu 15:00 after-hours open", 1794466800000, true),
            ("Fri 03:00 night session, early leg", 1794510000000, true),
            ("Sat 03:00 Friday's night session tail", 1794596400000, true),
            ("Sat 10:00 no Saturday day session", 1794621600000, false),
            ("Mon 03:00 no Sunday night session", 1794769200000, false),
            ("Thu 05:00 night close, session over", 1794430800000, false),
        ];
        for (label, at, want) in cases {
            assert_eq!(taifex_market_open(at), want, "{label}");
        }
    }
    #[test]
    fn us_market_open_follows_eastern_hours_and_dst() {
        let et = |y: i32, mo: u32, d: u32, h: u32, mi: u32, off: i32| {
            FixedOffset::west_opt(off * 3600)
                .unwrap()
                .with_ymd_and_hms(y, mo, d, h, mi, 0)
                .unwrap()
                .timestamp_millis()
        };
        // Winter (EST, UTC−5): the 04:00–20:00 window in Eastern terms.
        assert!(us_market_open(et(2026, 1, 15, 10, 0, 5)));
        assert!(!us_market_open(et(2026, 1, 15, 3, 30, 5)));
        assert!(us_market_open(et(2026, 1, 15, 19, 59, 5)));
        assert!(!us_market_open(et(2026, 1, 15, 20, 0, 5)));
        // Summer (EDT, UTC−4): 04:30 EDT is inside, 03:30 EDT is not; a fixed
        // −5 offset would read both an hour earlier and fail the first.
        assert!(us_market_open(et(2026, 7, 15, 4, 30, 4)));
        assert!(!us_market_open(et(2026, 7, 15, 3, 30, 4)));
        // A Saturday never opens.
        assert!(!us_market_open(et(2026, 11, 14, 12, 0, 5)));
    }
    #[test]
    fn yahoo_quotes_fire_rules_when_fresh_and_stay_silent_when_stale() {
        let dir =
            std::env::temp_dir().join(format!("smallstocks-yahoo-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        // Not demo: the real session gates apply. Taipei is closed at this
        // instant; the US session (10:00 EST) is not.
        let mut c = Core::load(dir.join("state.json"), false).unwrap();
        let t = FixedOffset::west_opt(5 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 11, 12, 10, 0, 0)
            .unwrap()
            .timestamp_millis();
        assert!(!c.market_open(t), "Taipei is closed during US hours");
        assert!(us_market_open(t));
        let yq = |symbol: &str, price: f64, at: i64| Quote {
            symbol: symbol.into(),
            name: "測試".into(),
            price: Some(price),
            bid: None,
            ask: None,
            previous: Some(100.),
            change_percent: Some(price - 100.),
            mid: None,
            mid_change_percent: None,
            trade_at: None,
            day_high: None,
            day_low: None,
            source_at: at,
            received_at: at,
            date: local(at).format("%Y%m%d").to_string(),
            source: "Yahoo".into(),
        };
        c.execute("set_watchlist", json!({"symbols":["VOO",".IXIC"]}))
            .unwrap();
        assert_eq!(c.saved.all_codes(), vec!["VOO", "^IXIC"]);
        c.execute(
            "upsert_rule",
            json!({"symbol":"VOO","metric":"price","direction":"above","threshold":105,"repeat":"daily","channel":"strip"}),
        )
        .unwrap();
        c.execute(
            "upsert_rule",
            json!({"symbol":"^IXIC","metric":"changePercent","direction":"above","threshold":1,"repeat":"daily","channel":"strip"}),
        )
        .unwrap();
        // First fresh observations only establish baselines.
        assert!(c
            .ingest(vec![yq("VOO", 100., t), yq("^IXIC", 100.5, t)], t)
            .is_empty());
        // Crossings on fresh quotes fire, credited to Yahoo, displayed without ^.
        let events = c.ingest(
            vec![yq("VOO", 106., t + 5000), yq("^IXIC", 101.6, t + 5000)],
            t + 5000,
        );
        assert_eq!(events.len(), 2);
        let voo = events.iter().find(|e| e.symbol == "VOO").unwrap();
        assert_eq!(voo.source, "Yahoo");
        let ix = events.iter().find(|e| e.symbol == "^IXIC").unwrap();
        assert!(ix.text.contains("IXIC"), "display form drops the ^: {}", ix.text);
        assert!(!ix.text.contains("^IXIC"));
        // A stale quote (200 s old) never fires, even across the threshold.
        let mut stale_rule = rule();
        stale_rule.symbol = "VOO".into();
        assert!(evaluate(
            &mut stale_rule,
            &yq("VOO", 100., t),
            t,
            false,
            us_market_open(t)
        )
        .is_none());
        assert!(evaluate(
            &mut stale_rule,
            &yq("VOO", 106., t - 200_000),
            t,
            false,
            us_market_open(t)
        )
        .is_none());
        // Outside US hours (01:00 EST) a fresh Yahoo quote stays silent too.
        let night = FixedOffset::west_opt(5 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 11, 12, 1, 0, 0)
            .unwrap()
            .timestamp_millis();
        assert!(!us_market_open(night));
        let mut night_rule = rule();
        night_rule.symbol = "VOO".into();
        assert!(evaluate(
            &mut night_rule,
            &yq("VOO", 100., night),
            night,
            false,
            us_market_open(night)
        )
        .is_none());
        assert!(evaluate(
            &mut night_rule,
            &yq("VOO", 106., night + 5000),
            night + 5000,
            false,
            us_market_open(night)
        )
        .is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn snapshot_intraday_covers_held_taiwan_codes_outside_the_watchlist() {
        let dir =
            std::env::temp_dir().join(format!("smallstocks-pfchart-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Core::load(dir.join("state.json"), true).unwrap();
        // 00685L is held but sits in no watch group; the portfolio day chart
        // still needs its intraday view in every snapshot.
        c.execute(
            "add_lot",
            json!({"code":"00685L","date":"2026-11-10","price":10.24,"shares":37000}),
        )
        .unwrap();
        let snap = c.snapshot();
        let intraday = snap["intraday"].as_object().unwrap();
        assert!(intraday.contains_key("00685L"));
        assert!(intraday["00685L"]["series"].is_null() || intraday["00685L"]["series"].is_object());
        // A USD holding gets no Taiwan intraday view.
        c.execute(
            "add_lot",
            json!({"code":"VOO","date":"2026-11-10","price":600,"shares":6.3}),
        )
        .unwrap();
        let snap = c.snapshot();
        assert!(!snap["intraday"].as_object().unwrap().contains_key("VOO"));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn export_import_data_round_trips_groups_and_positions() {
        let dir =
            std::env::temp_dir().join(format!("smallstocks-export-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Core::load(dir.join("state.json"), true).unwrap();
        c.execute("set_watchlist", json!({"symbols":["2330","0050"]}))
            .unwrap();
        c.execute("add_watch_group", json!({"name":"美股"})).unwrap();
        c.execute("set_watchlist", json!({"symbols":["VOO"]})).unwrap();
        c.execute("set_active_group", json!({"index":0})).unwrap();
        c.execute(
            "add_lot",
            json!({"code":"2330","date":"2026-11-10","price":1487,"shares":64}),
        )
        .unwrap();
        c.execute(
            "add_lot",
            json!({"code":"00685L","date":"2026-11-10","price":10.24,"shares":37000}),
        )
        .unwrap();
        c.execute(
            "add_lot",
            json!({"code":"00685L","date":"2026-11-12","price":10.05,"shares":1000}),
        )
        .unwrap();
        let text = c
            .execute("export_data", json!({}))
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["app"], "smallstocks-tw");
        assert_eq!(parsed["version"], 1);

        let dir2 =
            std::env::temp_dir().join(format!("smallstocks-import-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir2).unwrap();
        let mut c2 = Core::load(dir2.join("state.json"), true).unwrap();
        let snap = c2.execute("import_data", json!({"json": text})).unwrap();
        assert_eq!(snap["dataImport"]["groups"], 2);
        assert_eq!(snap["dataImport"]["positions"], 2);
        assert_eq!(snap["dataImport"]["lots"], 3);
        assert_eq!(snap["dataImport"]["skippedSymbols"], json!([]));
        assert_eq!(snap["dataImport"]["skippedLots"], 0);
        assert_eq!(c2.saved.groups.len(), 2);
        assert_eq!(c2.saved.groups[0].codes, vec!["2330", "0050"]);
        assert_eq!(c2.saved.groups[1].name, "美股");
        assert_eq!(c2.saved.groups[1].codes, vec!["VOO"]);
        assert_eq!(c2.saved.active_group, 0);
        assert_eq!(c2.saved.watchlist, vec!["2330", "0050"]);
        let lev = c2
            .saved
            .positions
            .iter()
            .find(|p| p.code == "00685L")
            .unwrap();
        assert_eq!(lev.lots.len(), 2);
        assert_eq!(lev.lots[1].shares, 1000.);
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(dir2).unwrap();
    }
    #[test]
    fn import_data_rejects_bad_envelopes_and_skips_bad_rows() {
        let dir =
            std::env::temp_dir().join(format!("smallstocks-imp2-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Core::load(dir.join("state.json"), true).unwrap();
        c.execute("set_watchlist", json!({"symbols":["2330"]}))
            .unwrap();
        assert!(c.execute("import_data", json!({"json": "not json"})).is_err());
        assert!(
            c.execute("import_data", json!({"json": "{\"app\":\"other\",\"version\":1,\"groups\":[],\"positions\":[]}"}))
                .is_err()
        );
        assert!(
            c.execute("import_data", json!({"json": "{\"app\":\"smallstocks-tw\",\"version\":2,\"groups\":[],\"positions\":[]}"}))
                .is_err()
        );
        // Rejections changed nothing.
        assert_eq!(c.saved.groups[0].codes, vec!["2330"]);

        let mixed = json!({
            "app": "smallstocks-tw",
            "version": 1,
            "activeGroup": 9,
            "groups": [
                {"name": "指數", "codes": ["IX0001", "!!bad", "0050", "0050"]},
            ],
            "positions": [
                {"code": "2330", "lots": [
                    {"date": "2026-11-10", "price": 1487, "shares": 64},
                    {"date": "2026-13-01", "price": 1500, "shares": 10},
                    {"date": "", "price": -1, "shares": 10},
                ]},
                {"code": "??", "lots": [{"date": "", "price": 1, "shares": 1}]},
            ],
        })
        .to_string();
        let snap = c.execute("import_data", json!({"json": mixed})).unwrap();
        assert_eq!(snap["dataImport"]["groups"], 1);
        assert_eq!(snap["dataImport"]["positions"], 1);
        assert_eq!(snap["dataImport"]["lots"], 1);
        assert_eq!(snap["dataImport"]["skippedLots"], 2);
        assert_eq!(
            snap["dataImport"]["skippedSymbols"],
            json!(["!!bad", "??"])
        );
        // The previous group content is gone (whole-file replace), the bad
        // lots were dropped, the duplicate code collapsed, and an
        // out-of-range activeGroup clamped to the last group.
        assert_eq!(c.saved.groups.len(), 1);
        assert_eq!(c.saved.groups[0].codes, vec!["^TWII", "0050"]);
        assert_eq!(c.saved.active_group, 0);
        assert_eq!(c.saved.positions.len(), 1);
        assert_eq!(c.saved.positions[0].lots.len(), 1);

        // Over the group cap the file is refused outright.
        let many = json!({
            "app": "smallstocks-tw", "version": 1,
            "groups": (0..11).map(|i| json!({"name": format!("G{i}"), "codes": []})).collect::<Vec<_>>(),
            "positions": [],
        })
        .to_string();
        assert!(c.execute("import_data", json!({"json": many})).is_err());
        assert_eq!(c.saved.groups.len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn ingested_names_are_cached_into_the_snapshot_and_survive_a_reload() {
        let dir =
            std::env::temp_dir().join(format!("smallstocks-names-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let mut c = Core::load(path.clone(), true).unwrap();
        // The default watchlist holds 2330; a named quote teaches the cache.
        assert!(c.saved.names.is_empty());
        let t = ts();
        c.ingest(vec![q(Some(100.), t)], t);
        assert_eq!(c.saved.names.get("2330").map(String::as_str), Some("台積電"));
        assert_eq!(c.snapshot()["names"]["2330"], "台積電");
        // A nameless quote never erases a learned name.
        let mut blank = q(Some(101.), t + 1000);
        blank.name = String::new();
        c.ingest(vec![blank], t + 1000);
        assert_eq!(c.saved.names.get("2330").map(String::as_str), Some("台積電"));
        // The cache reached disk with the ingest save: a fresh load of the
        // same file carries the name before any quote arrives.
        let reloaded = Core::load(path, true).unwrap();
        assert_eq!(
            reloaded.snapshot()["names"]["2330"],
            "台積電",
            "names persist across a restart"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn a_save_without_names_loads_with_an_empty_cache() {
        let dir = std::env::temp_dir().join(format!(
            "smallstocks-oldnames-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        std::fs::write(&path, br#"{"watchlist":["2330"]}"#).unwrap();
        let c = Core::load(path, true).unwrap();
        assert!(c.saved.names.is_empty());
        assert_eq!(c.snapshot()["names"], json!({}));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn export_data_file_writes_the_export_payload() {
        let dir =
            std::env::temp_dir().join(format!("smallstocks-expfile-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Core::load(dir.join("state.json"), true).unwrap();
        c.execute(
            "add_lot",
            json!({"code":"2330","date":"2026-11-10","price":1487,"shares":64}),
        )
        .unwrap();
        let text = c
            .execute("export_data", json!({}))
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        let out = c.execute("export_data_file", json!({})).unwrap();
        let path = out["path"].as_str().unwrap();
        // Named by the local date; with no Downloads folder for the test
        // user it lands beside state.json (the temp dir either way here).
        assert!(
            path.ends_with(&format!(
                "smallstocks-data-{}.json",
                local(now()).format("%Y%m%d")
            )),
            "unexpected export path: {path}"
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), text);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn monthly_snapshot_records_once_deletes_and_rejects_bad_input() {
        let dir = std::env::temp_dir().join(format!(
            "smallstocks-monthly-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let mut c = Core::load(path.clone(), true).unwrap();
        let good = json!({"month":"2026-11","totalTwd":6_600_000.0,"twdValue":6_500_000.0,"usdValue":3030.0,"rate":33.0,"recordedOn":"2026-11-02"});
        let snap = c.execute("record_monthly_snapshot", good.clone()).unwrap();
        assert_eq!(snap["monthlySnapshots"].as_array().unwrap().len(), 1);
        assert_eq!(snap["monthlySnapshots"][0]["month"], "2026-11");
        assert_eq!(snap["monthlySnapshots"][0]["totalTwd"], 6_600_000.0);
        assert_eq!(snap["monthlySnapshots"][0]["recordedOn"], "2026-11-02");
        // First write wins: a second call for the same month — even with a
        // different total — is a no-op, not an overwrite and not an error.
        let other = json!({"month":"2026-11","totalTwd":9_999_999.0,"twdValue":9_999_999.0,"recordedOn":"2026-11-20"});
        let snap = c.execute("record_monthly_snapshot", other).unwrap();
        assert_eq!(snap["monthlySnapshots"].as_array().unwrap().len(), 1);
        assert_eq!(snap["monthlySnapshots"][0]["totalTwd"], 6_600_000.0);
        assert_eq!(snap["monthlySnapshots"][0]["recordedOn"], "2026-11-02");
        // The record reached disk.
        let reloaded = Core::load(path, true).unwrap();
        assert_eq!(reloaded.saved.monthly_snapshots.len(), 1);
        // Every invalid call is refused and records nothing.
        for bad in [
            json!({"month":"2026-1","totalTwd":100.0,"twdValue":100.0,"recordedOn":"2026-11-02"}),
            json!({"month":"2026-13","totalTwd":100.0,"twdValue":100.0,"recordedOn":"2026-11-02"}),
            json!({"month":"2026/11","totalTwd":100.0,"twdValue":100.0,"recordedOn":"2026-11-02"}),
            json!({"totalTwd":100.0,"twdValue":100.0,"recordedOn":"2026-11-02"}),
            json!({"month":"2026-12","totalTwd":0.0,"twdValue":0.0,"recordedOn":"2026-12-01"}),
            json!({"month":"2026-12","totalTwd":-5.0,"twdValue":0.0,"recordedOn":"2026-12-01"}),
            json!({"month":"2026-12","totalTwd":"100","twdValue":100.0,"recordedOn":"2026-12-01"}),
            json!({"month":"2026-12","totalTwd":100.0,"twdValue":100.0,"recordedOn":"2026-12-32"}),
            json!({"month":"2026-12","totalTwd":100.0,"twdValue":100.0,"recordedOn":"12/01/2026"}),
            json!({"month":"2026-12","totalTwd":100.0,"twdValue":100.0,"usdValue":-1.0,"recordedOn":"2026-12-01"}),
            json!({"month":"2026-12","totalTwd":100.0,"twdValue":100.0,"rate":0.0,"recordedOn":"2026-12-01"}),
        ] {
            assert!(
                c.execute("record_monthly_snapshot", bad).is_err(),
                "bad input accepted"
            );
        }
        assert_eq!(c.saved.monthly_snapshots.len(), 1);
        // Delete removes the row; deleting it again is a harmless no-op,
        // and a malformed month is refused.
        let snap = c
            .execute("delete_monthly_snapshot", json!({"month":"2026-11"}))
            .unwrap();
        assert_eq!(snap["monthlySnapshots"].as_array().unwrap().len(), 0);
        let snap = c
            .execute("delete_monthly_snapshot", json!({"month":"2026-11"}))
            .unwrap();
        assert_eq!(snap["monthlySnapshots"].as_array().unwrap().len(), 0);
        assert!(c
            .execute("delete_monthly_snapshot", json!({"month":"nope"}))
            .is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn export_import_data_round_trips_monthly_snapshots() {
        let dir =
            std::env::temp_dir().join(format!("smallstocks-mexp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Core::load(dir.join("state.json"), true).unwrap();
        c.execute(
            "record_monthly_snapshot",
            json!({"month":"2026-10","totalTwd":6_100_000.0,"twdValue":6_100_000.0,"recordedOn":"2026-10-01"}),
        )
        .unwrap();
        c.execute(
            "record_monthly_snapshot",
            json!({"month":"2026-11","totalTwd":6_600_000.0,"twdValue":6_500_000.0,"usdValue":3030.0,"rate":33.0,"recordedOn":"2026-11-02"}),
        )
        .unwrap();
        let text = c
            .execute("export_data", json!({}))
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["monthlySnapshots"].as_array().unwrap().len(), 2);

        let dir2 =
            std::env::temp_dir().join(format!("smallstocks-mimp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir2).unwrap();
        let mut c2 = Core::load(dir2.join("state.json"), true).unwrap();
        let snap = c2.execute("import_data", json!({"json": text})).unwrap();
        assert_eq!(snap["dataImport"]["skippedSnapshots"], 0);
        assert_eq!(snap["monthlySnapshots"].as_array().unwrap().len(), 2);
        let nov = c2
            .saved
            .monthly_snapshots
            .iter()
            .find(|s| s.month == "2026-11")
            .unwrap();
        assert_eq!(nov.total_twd, 6_600_000.);
        assert_eq!(nov.usd_value, Some(3030.));
        assert_eq!(nov.rate, Some(33.));

        // Bad entries are skipped and tallied; a duplicate month keeps its
        // first entry; good rows still land.
        let mixed = json!({
            "app": "smallstocks-tw", "version": 1,
            "groups": [{"name": "自選1", "codes": ["2330"]}],
            "positions": [],
            "monthlySnapshots": [
                {"month": "2026-09", "totalTwd": 5_900_000, "twdValue": 5_900_000, "recordedOn": "2026-09-01"},
                {"month": "2026-13", "totalTwd": 1, "twdValue": 1, "recordedOn": "2026-09-01"},
                {"month": "2026-08", "totalTwd": -1, "twdValue": 1, "recordedOn": "2026-08-01"},
                {"month": "2026-07", "totalTwd": 1, "twdValue": 1, "recordedOn": "2026-07-32"},
                {"month": "2026-09", "totalTwd": 2, "twdValue": 2, "recordedOn": "2026-09-03"},
            ],
        })
        .to_string();
        let snap = c2.execute("import_data", json!({"json": mixed})).unwrap();
        assert_eq!(snap["dataImport"]["skippedSnapshots"], 4);
        assert_eq!(c2.saved.monthly_snapshots.len(), 1);
        assert_eq!(c2.saved.monthly_snapshots[0].month, "2026-09");
        assert_eq!(c2.saved.monthly_snapshots[0].total_twd, 5_900_000.);

        // An old export without the field imports as an empty list and
        // replaces what was there (whole-file replace semantics).
        let old = json!({
            "app": "smallstocks-tw", "version": 1,
            "groups": [{"name": "自選1", "codes": ["2330"]}],
            "positions": [],
        })
        .to_string();
        let snap = c2.execute("import_data", json!({"json": old})).unwrap();
        assert_eq!(snap["dataImport"]["skippedSnapshots"], 0);
        assert!(c2.saved.monthly_snapshots.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(dir2).unwrap();
    }
    #[test]
    fn telegram_settings_persist_but_the_token_never_enters_a_snapshot() {
        let dir = std::env::temp_dir().join(format!(
            "smallstocks-tg-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let mut c = Core::load(path.clone(), true).unwrap();
        c.execute(
            "set_settings",
            json!({"telegramEnabled":true,"telegramChatId":"1926601428","telegramBotToken":"123456:TESTTOKEN","backupEnabled":true,"backupDir":"/somewhere/backups"}),
        )
        .unwrap();
        assert!(c.saved.settings.telegram_enabled);
        assert_eq!(c.saved.settings.telegram_chat_id, "1926601428");
        assert_eq!(c.saved.settings.telegram_bot_token, "123456:TESTTOKEN");
        assert!(c.saved.settings.backup_enabled);
        assert_eq!(c.saved.settings.backup_dir, "/somewhere/backups");
        // The settings survive a reload (they live in state.json)…
        let mut restored = Core::load(path, true).unwrap();
        assert_eq!(restored.saved.settings.telegram_bot_token, "123456:TESTTOKEN");
        assert_eq!(restored.saved.settings.telegram_chat_id, "1926601428");
        // …but the snapshot only reports that a token is set, never the
        // token itself: no field, and the value nowhere in the JSON text.
        let snap = restored.snapshot();
        assert_eq!(snap["settings"]["telegramEnabled"], json!(true));
        assert_eq!(snap["settings"]["telegramChatId"], "1926601428");
        assert_eq!(snap["settings"]["telegramTokenSet"], json!(true));
        assert!(snap["settings"].get("telegramBotToken").is_none());
        assert!(
            !snap.to_string().contains("TESTTOKEN"),
            "snapshot must never carry the bot token"
        );
        // A blank token submit keeps the stored token (the settings field
        // is write-only); only the explicit clear flag drops it.
        restored
            .execute("set_settings", json!({"telegramBotToken":""}))
            .unwrap();
        assert_eq!(restored.saved.settings.telegram_bot_token, "123456:TESTTOKEN");
        restored
            .execute("set_settings", json!({"telegramClearToken":true}))
            .unwrap();
        assert!(restored.saved.settings.telegram_bot_token.is_empty());
        assert_eq!(
            restored.snapshot()["settings"]["telegramTokenSet"],
            json!(false)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn chat_window_setting_round_trips_and_defaults_off() {
        let dir = std::env::temp_dir().join(format!(
            "smallstocks-chat-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let mut c = Core::load(path.clone(), true).unwrap();
        // Absent from an old state file, the toggle defaults to off.
        assert!(!c.saved.settings.chat_window);
        assert_eq!(c.snapshot()["settings"]["chatWindow"], json!(false));
        c.execute("set_settings", json!({"chatWindow": true}))
            .unwrap();
        assert!(c.saved.settings.chat_window);
        assert_eq!(c.snapshot()["settings"]["chatWindow"], json!(true));
        // It persists in state.json across a reload…
        let restored = Core::load(path, true).unwrap();
        assert!(restored.saved.settings.chat_window);
        // …and a non-boolean value is refused, leaving the setting alone.
        let mut c2 = restored;
        assert!(c2.execute("set_settings", json!({"chatWindow": "yes"})).is_err());
        assert!(c2.saved.settings.chat_window);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn telegram_test_is_guarded_and_alerts_mirror_only_notice_channels() {
        let dir = std::env::temp_dir().join(format!(
            "smallstocks-tgtest-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Core::load(dir.join("state.json"), true).unwrap();
        // Nothing configured: the test command refuses before any send,
        // naming the missing piece.
        let err = c.execute("telegram_test", json!({})).unwrap_err();
        assert!(err.contains("bot token"), "{err}");
        c.execute("set_settings", json!({"telegramBotToken":"123456:TESTTOKEN"}))
            .unwrap();
        let err = c.execute("telegram_test", json!({})).unwrap_err();
        assert!(err.contains("chat id"), "{err}");
        // Guard errors never carry the token either.
        assert!(!err.contains("TESTTOKEN"), "{err}");
        // The request builder pairs the stored chat with the text.
        c.saved.settings.telegram_chat_id = "42".into();
        let (url, body) = telegram_request(&c.saved.settings, "🔔 測試").unwrap();
        assert_eq!(body["chat_id"], "42");
        assert_eq!(body["text"], "🔔 測試");
        assert!(url.starts_with("https://api.telegram.org/bot"));
        // Alerts mirror strip/card events only, prefixed, and only when
        // the push is enabled.
        let event = |channel: &str| Event {
            id: "e".into(),
            rule_id: "r".into(),
            symbol: "2330".into(),
            text: "台積電 觸及門檻".into(),
            source_at: 0,
            detected_at: 0,
            channel: channel.into(),
            value: 0.,
            threshold: 0.,
            source: String::new(),
        };
        let events = vec![event("strip"), event("card"), event("log"), event("off")];
        assert!(telegram_alerts(&c.saved.settings, &events).is_empty());
        c.saved.settings.telegram_enabled = true;
        let alerts = telegram_alerts(&c.saved.settings, &events);
        assert_eq!(alerts.len(), 2);
        assert_eq!(alerts[0].1["text"], "🔔 台積電 觸及門檻");
        assert_eq!(alerts[0].1["chat_id"], "42");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn state_backups_write_throttle_prune_and_noop() {
        let dir = std::env::temp_dir().join(format!(
            "smallstocks-backup-{}",
            uuid::Uuid::new_v4()
        ));
        let bdir = dir.join("backups");
        std::fs::create_dir_all(&bdir).unwrap();
        let state_path = dir.join("state.json");
        let mut c = Core::load(state_path.clone(), true).unwrap();
        // Folder configured but backups disabled: saving writes nothing.
        c.execute(
            "set_settings",
            json!({"backupDir": bdir.to_string_lossy()}),
        )
        .unwrap();
        assert!(backup_files(&bdir).is_empty());
        assert!(c.last_backup_at.is_none());
        // Enabling triggers the first backup through the persist itself,
        // byte-identical to the state just saved.
        c.execute("set_settings", json!({"backupEnabled": true}))
            .unwrap();
        let files = backup_files(&bdir);
        assert_eq!(files.len(), 1);
        assert_eq!(
            std::fs::read(&files[0]).unwrap(),
            std::fs::read(&state_path).unwrap()
        );
        assert!(c.last_backup_at.is_some());
        assert!(c.last_backup_error.is_none());
        // A second automatic call the same day adds no file…
        c.maybe_backup(false);
        assert_eq!(backup_files(&bdir).len(), 1);
        // …while backup_now ignores the throttle and reports its path.
        let out = c.execute("backup_now", json!({})).unwrap();
        let written = out["path"].as_str().unwrap();
        assert!(std::path::Path::new(written).exists());
        // Pruning: 15 seeded older backups plus today's passes the cap, so
        // the next write keeps only the newest BACKUP_KEEP.
        for d in 1..=15 {
            std::fs::write(
                bdir.join(format!("state-backup-202601{d:02}-000000.json")),
                "{}",
            )
            .unwrap();
        }
        c.maybe_backup(true);
        let files = backup_files(&bdir);
        assert_eq!(files.len(), BACKUP_KEEP);
        assert!(!bdir.join("state-backup-20260101-000000.json").exists());
        assert!(!bdir.join("state-backup-20260102-000000.json").exists());
        assert!(bdir.join("state-backup-20260115-000000.json").exists());
        std::fs::remove_dir_all(dir).unwrap();
        // A missing folder is a quiet no-op for the automatic path, and a
        // clear error for the manual one.
        let dir2 = std::env::temp_dir().join(format!(
            "smallstocks-backup2-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir2).unwrap();
        let mut c2 = Core::load(dir2.join("state.json"), true).unwrap();
        c2.saved.settings.backup_enabled = true;
        c2.saved.settings.backup_dir = dir2.join("gone").to_string_lossy().into_owned();
        c2.maybe_backup(false);
        assert!(c2.last_backup_at.is_none());
        assert!(c2.last_backup_error.is_none());
        let err = c2.execute("backup_now", json!({})).unwrap_err();
        assert!(err.contains("不存在"), "{err}");
        std::fs::remove_dir_all(dir2).unwrap();
    }
}
