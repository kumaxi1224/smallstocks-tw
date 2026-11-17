//! Plain-text (notepad-style) rendering of the PTT chat channel's
//! public preview feed, for the chat window's "text" style.
//!
//! The app fetches https://t.me/s/ptt_stock_follow_chat itself (the
//! window never loads t.me directly in this style), parses the
//! server-rendered widget HTML with `parse_messages`, and serves a
//! cached monospace page through a custom protocol (see lib.rs). The
//! page is deliberately boring: white background, dark monospace
//! text — the same document look as the main window's txt skin —
//! one message per block, `HH:MM 作者：` up front — a .txt file,
//! not a chat UI.
//!
//! Reading position: the page refreshes itself in place — a small
//! script re-fetches this same URL every minute, swaps the `#feed`
//! contents (and the header's fetch time) via DOMParser, and never
//! touches the scroll position unless the reader is already at the
//! bottom, so someone scrolled up reading older messages stays put
//! while new ones append below. The earlier design — a shell-side
//! full reload every minute plus a sessionStorage scroll anchor —
//! broke on Windows, where chattext:// surfaces as
//! http://chattext.localhost and sessionStorage does not survive
//! those reloads: every round found no anchor, force-scrolled to the
//! bottom, and the page felt unscrollable. No reload, no anchor now.

use scraper::{Html, Selector};
use std::sync::{Arc, Mutex, PoisonError};

/// One parsed channel message. `service` lines (joins, pins, …) have no
/// author and render centered/dim instead of with the `作者：` prefix.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessage {
    /// The message's `data-post` id (e.g. "ptt_stock_follow_chat/1083812"),
    /// carried through to the rendered page as the block's attribute.
    /// Empty when the source block carried none.
    pub id: String,
    pub author: String,
    /// Plain text body; line breaks from the source are `\n`.
    pub text: String,
    /// "HH:MM" in Asia/Taipei.
    pub time_hhmm: String,
    pub service: bool,
}

/// The latest rendered feed page, shared between the background fetch
/// loop (writer) and the custom-protocol handler (reader). `None` until
/// the first successful fetch+parse.
#[derive(Clone, Default)]
pub struct ChatCache {
    page: Arc<Mutex<Option<String>>>,
}

impl ChatCache {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn page(&self) -> Option<String> {
        self.page
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    pub fn store(&self, page: String) {
        *self.page.lock().unwrap_or_else(PoisonError::into_inner) = Some(page);
    }
}

fn sel(selector: &str) -> Selector {
    Selector::parse(selector).expect("static selector")
}

/// The channel's display title from the page header, if present.
pub fn channel_title(html: &str) -> Option<String> {
    let doc = Html::parse_document(html);
    let el = doc.select(&sel(".tgme_channel_info_header_title")).next()?;
    let title = collapse_spaces(&el.text().collect::<String>());
    if title.is_empty() {
        None
    } else {
        Some(title)
    }
}

/// Parse every message block out of a t.me/s page, oldest first (the
/// page's own order). Pure: no I/O, so tests drive it with fixtures.
pub fn parse_messages(html: &str) -> Vec<ChatMessage> {
    let doc = Html::parse_document(html);
    let mut out = Vec::new();
    for el in doc.select(&sel(".tgme_widget_message")) {
        let id = el.value().attr("data-post").unwrap_or("").trim().to_string();
        let service_el = el.select(&sel(".tgme_widget_message_service")).next();
        let is_service_block = service_el.is_some()
            || el.value().classes().any(|c| c == "service_message");
        // Author: group-style posts carry `author_name`, channel posts
        // `owner_name`; the real feed uses the latter.
        let author = [".tgme_widget_message_author_name", ".tgme_widget_message_owner_name"]
            .iter()
            .filter_map(|s| el.select(&sel(s)).next())
            .map(|a| collapse_spaces(&a.text().collect::<String>()))
            .find(|a| !a.is_empty())
            .unwrap_or_default();
        let mut text = el
            .select(&sel(".tgme_widget_message_text"))
            .next()
            .map(|t| extract_text(&t.inner_html()))
            .unwrap_or_default();
        if text.is_empty() {
            if let Some(media) = media_placeholder(&el) {
                text = media.to_string();
            }
        } else if let Some(media) = media_placeholder(&el) {
            text = format!("{text}\n{media}");
        }
        let service = is_service_block || author.is_empty();
        if service {
            if let Some(svc) = &service_el {
                let svc_text = extract_text(&svc.inner_html());
                if !svc_text.is_empty() {
                    text = svc_text;
                }
            }
        }
        let (datetime, time_text) = el
            .select(&sel("time"))
            .next()
            .map(|t| {
                (
                    t.value().attr("datetime").unwrap_or("").to_string(),
                    t.text().collect::<String>(),
                )
            })
            .unwrap_or_default();
        let time_hhmm = taipei_hhmm(&datetime, &time_text);
        if text.is_empty() && !service {
            continue;
        }
        out.push(ChatMessage {
            id,
            author: if service { String::new() } else { author },
            text,
            time_hhmm,
            service,
        });
    }
    out
}

/// The media placeholder for a message block, when it carries media:
/// photo beats video beats anything else (document, sticker, voice…).
fn media_placeholder(el: &scraper::ElementRef) -> Option<&'static str> {
    if el.select(&sel(".tgme_widget_message_photo_wrap")).next().is_some() {
        return Some("[圖片]");
    }
    if el.select(&sel(".tgme_widget_message_video_wrap")).next().is_some()
        || el.select(&sel(".tgme_widget_message_video_player")).next().is_some()
        || el.select(&sel("video")).next().is_some()
    {
        return Some("[影片]");
    }
    if el.select(&sel(".tgme_widget_message_document_wrap")).next().is_some()
        || el.select(&sel(".tgme_widget_message_document")).next().is_some()
        || el.select(&sel(".tgme_widget_message_sticker_wrap")).next().is_some()
        || el.select(&sel(".tgme_widget_message_voice")).next().is_some()
    {
        return Some("[媒體]");
    }
    None
}

/// Plain text out of a message-text fragment: `<br>` becomes a newline,
/// every other tag is dropped keeping its text (links keep their label,
/// emoji `<i><b>🟢</b></i>` keeps the emoji).
fn extract_text(inner_html: &str) -> String {
    let with_breaks = br_to_newlines(inner_html);
    let frag = Html::parse_fragment(&with_breaks);
    let raw: String = frag.root_element().text().collect();
    normalize_text(&raw)
}

/// Replace `<br …>` tags (any case) with `\n`. Byte positions are found
/// with an ASCII case-insensitive search on the original string, so
/// multi-byte text is never sliced at a wrong offset.
fn br_to_newlines(html: &str) -> String {
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<'
            && i + 2 < bytes.len()
            && bytes[i + 1].eq_ignore_ascii_case(&b'b')
            && bytes[i + 2].eq_ignore_ascii_case(&b'r')
        {
            match html[i..].find('>') {
                Some(end) => {
                    out.push('\n');
                    i += end + 1;
                    continue;
                }
                None => break,
            }
        }
        // Copy one UTF-8 character verbatim.
        let ch_len = utf8_len(bytes[i]);
        out.push_str(&html[i..i + ch_len]);
        i += ch_len;
    }
    out
}

fn utf8_len(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first >> 5 == 0b110 {
        2
    } else if first >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

/// Trim trailing spaces per line, drop leading/trailing blank lines,
/// and collapse runs of blank lines to at most one.
fn normalize_text(raw: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    for line in raw.lines() {
        let line = line.trim_end();
        if line.is_empty() && (lines.is_empty() || lines.last() == Some(&"")) {
            continue;
        }
        lines.push(line);
    }
    while lines.last() == Some(&"") {
        lines.pop();
    }
    lines.join("\n")
}

fn collapse_spaces(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// "HH:MM" in Asia/Taipei (fixed UTC+8; Taiwan has no DST) from the
/// `<time datetime>` UTC ISO stamp; falls back to the element's own
/// text when the attribute is missing or unparseable.
fn taipei_hhmm(datetime: &str, fallback: &str) -> String {
    if !datetime.trim().is_empty() {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(datetime.trim()) {
            let offset = chrono::FixedOffset::east_opt(8 * 3600).expect("UTC+8");
            return dt.with_timezone(&offset).format("%H:%M").to_string();
        }
    }
    fallback.trim().to_string()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// In-page self-update (see the module note for why there is no
/// reload-and-anchor design anymore): every minute the page fetches
/// its own URL — the protocol handler answers with the freshest
/// cached render — and swaps the `#feed` contents plus the header's
/// fetch time via DOMParser. The scroll position is only adjusted
/// when the reader is already at the bottom (within 80px), where it
/// re-pins to the newest message; a reader scrolled up keeps their
/// exact position while new messages append below. A failed fetch or
/// parse keeps the current page and waits for the next round. The
/// shell can trigger the same update when the window is brought
/// forward, through `window.__chatTextUpdate`.
const UPDATE_SCRIPT: &str = r#"<script>
(function () {
  var INTERVAL_MS = 60000;
  function scroller() { return document.scrollingElement || document.documentElement; }
  function atBottom() {
    var e = scroller();
    if (!e) return true;
    return (e.scrollTop + e.clientHeight) >= (e.scrollHeight - 80);
  }
  function toBottom() {
    var e = scroller();
    if (e) e.scrollTop = e.scrollHeight;
  }
  function update() {
    fetch(location.href, { cache: "no-store" })
      .then(function (r) { return r.text(); })
      .then(function (html) {
        var doc = new DOMParser().parseFromString(html, "text/html");
        var newFeed = doc.getElementById("feed");
        var feed = document.getElementById("feed");
        if (!newFeed || !feed) return;
        var stick = atBottom();
        feed.innerHTML = newFeed.innerHTML;
        var newHdr = doc.getElementById("hdr");
        var hdr = document.getElementById("hdr");
        if (newHdr && hdr) hdr.innerHTML = newHdr.innerHTML;
        if (stick) toBottom();
      })
      .catch(function () { /* keep the current page; retry next round */ });
  }
  window.__chatTextUpdate = update;
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", toBottom);
  } else {
    toBottom();
  }
  setInterval(update, INTERVAL_MS);
})();
</script>"#;

fn page_shell(channel_title: &str, fetched_hhmm: Option<&str>, body: &str, script: bool) -> String {
    let fetched = fetched_hhmm
        .map(|t| format!("（抓取時間 {t}）"))
        .unwrap_or_default();
    format!(
        "<!DOCTYPE html>\n<html lang=\"zh-Hant\"><head><meta charset=\"utf-8\">\
<title>{title}</title><style>\
html{{background:#FFFFFF}}\
body{{margin:0;padding:6px 8px;background:#FFFFFF;color:#202020;\
font-family:\"Cascadia Mono\",Consolas,\"Microsoft JhengHei UI\",monospace;font-size:13px;line-height:1.45}}\
.hdr{{color:#202020;border-bottom:1px solid #E1E1E1;padding-bottom:4px;margin-bottom:6px;\
white-space:pre-wrap;word-break:break-all}}\
.msg{{white-space:pre-wrap;word-break:break-all;margin:0 0 2px}}\
.time{{color:#808080}}\
.author{{color:#202020;font-weight:bold}}\
.svc{{color:#808080;text-align:center;white-space:pre-wrap;word-break:break-all;margin:4px 0}}\
</style></head><body>\
<div class=\"hdr\" id=\"hdr\">{title}{fetched}</div>\n{body}\n{script}</body></html>",
        title = esc(channel_title),
        fetched = esc(&fetched),
        body = body,
        script = if script { UPDATE_SCRIPT } else { "" },
    )
}

/// Render the whole feed page: header line (channel + fetch time), then
/// one block per message — `HH:MM 作者：內文` — oldest first, newest at
/// the bottom, exactly like the source page's order. The blocks sit in
/// `<div id="feed">` so the page's self-update script can swap them
/// (and the `#hdr` header) without a reload.
pub fn render_page(channel_title: &str, messages: &[ChatMessage], fetched_hhmm: &str) -> String {
    let mut body = String::from("<div id=\"feed\">\n");
    for m in messages {
        let post_attr = if m.id.is_empty() {
            String::new()
        } else {
            format!(" data-post=\"{}\"", esc(&m.id))
        };
        if m.service {
            body.push_str(&format!(
                "<div class=\"svc\"{post_attr}>{}</div>\n",
                esc(&m.text)
            ));
        } else {
            body.push_str(&format!(
                "<div class=\"msg\"{post_attr}><span class=\"time\">{}</span> \
<span class=\"author\">{}：</span>{}</div>\n",
                esc(&m.time_hhmm),
                esc(&m.author),
                esc(&m.text),
            ));
        }
    }
    body.push_str("</div>");
    page_shell(channel_title, Some(fetched_hhmm), &body, true)
}

/// A shell page with a single centered line — the "載入中…" first-run
/// page and the could-not-load explanation page. Same shell structure
/// (header + `#feed`) as the feed page, but deliberately no script:
/// there is nothing to update in place yet, and the real page takes
/// over on the next window (re)build once the cache has one.
pub fn render_status_page(channel_title: &str, message: &str) -> String {
    let body = format!(
        "<div id=\"feed\"><div class=\"svc\">{}</div></div>",
        esc(message)
    );
    page_shell(channel_title, None, &body, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixture modeled on the live t.me/s structure (verified against
    /// the real page 2026-11-16): channel posts carry `owner_name`,
    /// text holds `<br/>` and inline tags, `<time datetime>` is UTC.
    const FIXTURE: &str = r#"<!DOCTYPE html><html><body>
<div class="tgme_channel_info">
  <div class="tgme_channel_info_header_title"><span dir="auto">PTT 11/16盤中閒聊·台指強空</span></div>
</div>
<section class="tgme_channel_history">
<div class="tgme_widget_message_wrap js-widget_message_wrap"><div class="tgme_widget_message text_not_supported_wrap js-widget_message" data-post="ptt_stock_follow_chat/101">
  <div class="tgme_widget_message_author accent_color"><a class="tgme_widget_message_author_name" href="https://t.me/ptt_stock_follow_chat"><span dir="auto">作者甲</span></a></div>
  <div class="tgme_widget_message_text js-message_text" dir="auto">第一行<br/>第二行 &lt;script&gt;alert(1)&lt;/script&gt; 結尾</div>
  <div class="tgme_widget_message_footer compact"><div class="tgme_widget_message_info short"><a class="tgme_widget_message_date" href="https://t.me/ptt_stock_follow_chat/101"><time datetime="2026-11-16T05:35:15+00:00">05:35</time></a></div></div>
</div></div>
<div class="tgme_widget_message_wrap js-widget_message_wrap"><div class="tgme_widget_message js-widget_message" data-post="ptt_stock_follow_chat/102">
  <div class="tgme_widget_message_author accent_color"><a class="tgme_widget_message_owner_name" href="https://t.me/ptt_stock_follow_chat"><span dir="auto">PTT 11/16盤中閒聊·台指強空</span></a></div>
  <div class="tgme_widget_message_text js-message_text" dir="auto">推 <code>jenchieh5   ：</code><br>龍鴿上周的超級大換股<br/><br/>詳見 <a href="https://example.com/x">這篇連結</a> 說明</div>
  <div class="tgme_widget_message_footer compact"><div class="tgme_widget_message_info short"><a class="tgme_widget_message_date" href="https://t.me/ptt_stock_follow_chat/102"><time datetime="2026-11-16T16:05:00+00:00">16:05</time></a></div></div>
</div></div>
<div class="tgme_widget_message_wrap js-widget_message_wrap"><div class="tgme_widget_message js-widget_message" data-post="ptt_stock_follow_chat/103">
  <div class="tgme_widget_message_author accent_color"><a class="tgme_widget_message_owner_name" href="https://t.me/ptt_stock_follow_chat"><span dir="auto">PTT 11/16盤中閒聊·台指強空</span></a></div>
  <a class="tgme_widget_message_photo_wrap" href="https://t.me/ptt_stock_follow_chat/103" style="background-image:url('https://cdn.example/p.jpg')"></a>
  <div class="tgme_widget_message_footer compact"><div class="tgme_widget_message_info short"><a class="tgme_widget_message_date" href="https://t.me/ptt_stock_follow_chat/103"><time datetime="2026-11-16T05:40:00+00:00">05:40</time></a></div></div>
</div></div>
<div class="tgme_widget_message_wrap js-widget_message_wrap"><div class="tgme_widget_message service_message js-widget_message" data-post="ptt_stock_follow_chat/104">
  <div class="tgme_widget_message_service">頻道名稱已更改為「PTT 11/16盤中閒聊·台指強空」</div>
</div></div>
</section>
</body></html>"#;

    #[test]
    fn parses_title_and_messages_from_widget_html() {
        assert_eq!(
            channel_title(FIXTURE).as_deref(),
            Some("PTT 11/16盤中閒聊·台指強空")
        );
        let msgs = parse_messages(FIXTURE);
        assert_eq!(msgs.len(), 4, "{msgs:?}");
        // Ordinary message: author_name variant, <br/> → newline, tags
        // stripped but their text kept (the escaped <script> in the
        // source is literal message text here).
        assert_eq!(msgs[0].id, "ptt_stock_follow_chat/101");
        assert_eq!(msgs[0].author, "作者甲");
        assert_eq!(msgs[0].text, "第一行\n第二行 <script>alert(1)</script> 結尾");
        assert!(!msgs[0].service);
        // UTC 05:35 → Taipei 13:35.
        assert_eq!(msgs[0].time_hhmm, "13:35");
        // owner_name variant; multi-line with a collapsed blank line;
        // link keeps its label; UTC 16:05 → Taipei 00:05 next day.
        assert_eq!(msgs[1].author, "PTT 11/16盤中閒聊·台指強空");
        assert_eq!(
            msgs[1].text,
            "推 jenchieh5   ：\n龍鴿上周的超級大換股\n\n詳見 這篇連結 說明"
        );
        assert_eq!(msgs[1].time_hhmm, "00:05");
        // Photo-only message gets the placeholder as its body.
        assert_eq!(msgs[2].text, "[圖片]");
        assert!(!msgs[2].service);
        assert_eq!(msgs[2].time_hhmm, "13:40");
        // Service message: no author, flagged, text from the service div.
        assert!(msgs[3].service);
        assert_eq!(msgs[3].author, "");
        assert_eq!(msgs[3].text, "頻道名稱已更改為「PTT 11/16盤中閒聊·台指強空」");
    }

    #[test]
    fn renders_notepad_style_page_with_escaping_and_update_script() {
        let msgs = parse_messages(FIXTURE);
        let page = render_page("PTT 11/16盤中閒聊·台指強空", &msgs, "13:41");
        // Header line: channel title + fetch time.
        assert!(page.contains("PTT 11/16盤中閒聊·台指強空（抓取時間 13:41）"), "{page}");
        // Message line format: HH:MM 作者：內文, carrying its data-post.
        assert!(
            page.contains("data-post=\"ptt_stock_follow_chat/101\""),
            "{page}"
        );
        assert!(page.contains("13:35"), "{page}");
        assert!(page.contains("作者甲："), "{page}");
        // The message's literal <script> text is escaped, never raw —
        // the only <script> in the page is the update script itself.
        assert!(page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"), "{page}");
        assert!(!page.contains("<script>alert"), "{page}");
        // Photo placeholder and the centered service line render.
        assert!(page.contains("[圖片]"), "{page}");
        assert!(page.contains("class=\"svc\""), "{page}");
        // Notepad look: white page, dark text, the main window txt
        // skin's font stack.
        assert!(page.contains("#FFFFFF"), "{page}");
        assert!(page.contains("#202020"), "{page}");
        assert!(!page.contains("#0C0C0C"), "{page}");
        assert!(page.contains("Cascadia Mono"), "{page}");
        assert!(page.contains("Consolas"), "{page}");
        assert!(page.contains("Microsoft JhengHei UI"), "{page}");
        // Self-update machinery: messages live in #feed under a #hdr
        // header; the script re-fetches this URL, swaps both via
        // DOMParser, sticks to the bottom only when already there
        // (scrollHeight/scrollTop), and is exposed for the shell.
        assert!(page.contains("id=\"feed\""), "{page}");
        assert!(page.contains("id=\"hdr\""), "{page}");
        assert!(page.contains("fetch(location.href"), "{page}");
        assert!(page.contains("no-store"), "{page}");
        assert!(page.contains("DOMParser"), "{page}");
        assert!(page.contains("scrollHeight"), "{page}");
        assert!(page.contains("__chatTextUpdate"), "{page}");
        // The reload-era sessionStorage anchor is gone for good.
        assert!(!page.contains("sessionStorage"), "{page}");
        // Status pages share the shell structure but carry no script.
        let status = render_status_page("PTT 股票閒聊", "載入中…");
        assert!(status.contains("載入中…"), "{status}");
        assert!(status.contains("id=\"feed\""), "{status}");
        assert!(!status.contains("sessionStorage"), "{status}");
        assert!(!status.contains("__chatTextUpdate"), "{status}");
    }

    #[test]
    fn empty_or_foreign_html_parses_to_no_messages() {
        assert!(parse_messages("").is_empty());
        assert!(parse_messages("<html><body><p>hi</p></body></html>").is_empty());
        assert_eq!(channel_title("<html><body></body></html>"), None);
    }
}
