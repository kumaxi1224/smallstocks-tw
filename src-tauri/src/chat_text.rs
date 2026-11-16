//! Plain-text (CMD-style) rendering of the PTT chat channel's public
//! preview feed, for the chat window's "text" style.
//!
//! The app fetches https://t.me/s/ptt_stock_follow_chat itself (the
//! window never loads t.me directly in this style), parses the
//! server-rendered widget HTML with `parse_messages`, and serves a
//! cached monospace page through a custom protocol (see lib.rs). The
//! page is deliberately boring: black background, light-gray monospace
//! text, one message per block, `HH:MM 作者：` up front — a .txt file,
//! not a chat UI.
//!
//! Reading position: the hosted t.me page anchors at the newest
//! message, so a reload used to yank the user away from older messages
//! they had scrolled up to read. The rendered page carries a small
//! script that keeps the data-post id of the topmost visible message
//! (plus its offset) in sessionStorage and scrolls back to it after a
//! reload; only a first load with nothing stored goes to the bottom.

use scraper::{Html, Selector};
use std::sync::{Arc, Mutex, PoisonError};

/// One parsed channel message. `service` lines (joins, pins, …) have no
/// author and render centered/dim instead of with the `作者：` prefix.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessage {
    /// The message's `data-post` id (e.g. "ptt_stock_follow_chat/1083812");
    /// the page script anchors scroll restoration on it. Empty when the
    /// source block carried none.
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

/// Keeps the reading position across reloads: remembers the topmost
/// visible message's data-post (and its viewport offset) in
/// sessionStorage — on scroll and before unload — and scrolls back to
/// it on load. With nothing stored, goes to the bottom (newest).
const ANCHOR_SCRIPT: &str = r#"<script>
(function () {
  var KEY = "chatTextAnchor";
  function escAttr(v) { return v.replace(/\\/g, "\\\\").replace(/"/g, '\\"'); }
  function save() {
    try {
      var msgs = document.querySelectorAll("[data-post]");
      for (var i = 0; i < msgs.length; i++) {
        var r = msgs[i].getBoundingClientRect();
        if (r.bottom > 0) {
          sessionStorage.setItem(KEY, JSON.stringify({ id: msgs[i].getAttribute("data-post"), off: r.top }));
          return;
        }
      }
    } catch (e) {}
  }
  window.addEventListener("beforeunload", save);
  var pending = false;
  window.addEventListener("scroll", function () {
    if (!pending) {
      pending = true;
      setTimeout(function () { pending = false; save(); }, 150);
    }
  }, { passive: true });
  function restore() {
    var el = document.scrollingElement || document.documentElement;
    try {
      var raw = sessionStorage.getItem(KEY);
      if (raw) {
        var a = JSON.parse(raw);
        var t = document.querySelector('[data-post="' + escAttr(a.id) + '"]');
        if (t) {
          el.scrollTop += t.getBoundingClientRect().top - a.off;
          return;
        }
      }
    } catch (e) {}
    el.scrollTop = el.scrollHeight;
  }
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", restore);
  } else {
    restore();
  }
})();
</script>"#;

fn page_shell(channel_title: &str, fetched_hhmm: Option<&str>, body: &str, script: bool) -> String {
    let fetched = fetched_hhmm
        .map(|t| format!("（抓取時間 {t}）"))
        .unwrap_or_default();
    format!(
        "<!DOCTYPE html>\n<html lang=\"zh-Hant\"><head><meta charset=\"utf-8\">\
<title>{title}</title><style>\
html{{background:#0C0C0C}}\
body{{margin:0;padding:6px 8px;background:#0C0C0C;color:#C0C0C0;\
font-family:Consolas,\"Courier New\",monospace;font-size:13px;line-height:1.45}}\
.hdr{{color:#FFFFFF;border-bottom:1px solid #444444;padding-bottom:4px;margin-bottom:6px;\
white-space:pre-wrap;word-break:break-all}}\
.msg{{white-space:pre-wrap;word-break:break-all;margin:0 0 2px}}\
.time{{color:#808080}}\
.author{{color:#FFFFFF;font-weight:bold}}\
.svc{{color:#808080;text-align:center;white-space:pre-wrap;word-break:break-all;margin:4px 0}}\
</style></head><body>\
<div class=\"hdr\">{title}{fetched}</div>\n{body}\n{script}</body></html>",
        title = esc(channel_title),
        fetched = esc(&fetched),
        body = body,
        script = if script { ANCHOR_SCRIPT } else { "" },
    )
}

/// Render the whole feed page: header line (channel + fetch time), then
/// one block per message — `HH:MM 作者：內文` — oldest first, newest at
/// the bottom, exactly like the source page's order.
pub fn render_page(channel_title: &str, messages: &[ChatMessage], fetched_hhmm: &str) -> String {
    let mut body = String::new();
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
    page_shell(channel_title, Some(fetched_hhmm), &body, true)
}

/// A shell page with a single centered line — the "載入中…" first-run
/// page and the could-not-load explanation page. No anchor script:
/// there is nothing to anchor to yet.
pub fn render_status_page(channel_title: &str, message: &str) -> String {
    let body = format!("<div class=\"svc\">{}</div>", esc(message));
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
    fn renders_cmd_style_page_with_escaping_and_anchor_script() {
        let msgs = parse_messages(FIXTURE);
        let page = render_page("PTT 11/16盤中閒聊·台指強空", &msgs, "13:41");
        // Header line: channel title + fetch time.
        assert!(page.contains("PTT 11/16盤中閒聊·台指強空（抓取時間 13:41）"), "{page}");
        // Message line format: HH:MM 作者：內文, anchored by data-post.
        assert!(
            page.contains("data-post=\"ptt_stock_follow_chat/101\""),
            "{page}"
        );
        assert!(page.contains("13:35"), "{page}");
        assert!(page.contains("作者甲："), "{page}");
        // The message's literal <script> text is escaped, never raw —
        // the only <script> in the page is the anchor script itself.
        assert!(page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"), "{page}");
        assert!(!page.contains("<script>alert"), "{page}");
        // Photo placeholder and the centered service line render.
        assert!(page.contains("[圖片]"), "{page}");
        assert!(page.contains("class=\"svc\""), "{page}");
        // CMD look + the scroll-anchor machinery.
        assert!(page.contains("#0C0C0C"), "{page}");
        assert!(page.contains("Consolas"), "{page}");
        assert!(page.contains("sessionStorage"), "{page}");
        // Status pages carry the message and no anchor script.
        let status = render_status_page("PTT 股票閒聊", "載入中…");
        assert!(status.contains("載入中…"), "{status}");
        assert!(!status.contains("sessionStorage"), "{status}");
    }

    #[test]
    fn empty_or_foreign_html_parses_to_no_messages() {
        assert!(parse_messages("").is_empty());
        assert!(parse_messages("<html><body><p>hi</p></body></html>").is_empty());
        assert_eq!(channel_title("<html><body></body></html>"), None);
    }
}
