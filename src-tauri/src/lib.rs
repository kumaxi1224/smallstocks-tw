mod calendar;
mod core;
mod daily;
mod history;
mod intraday;
mod mcp;
mod sidecar_keys;
mod taifex;
mod yahoo;
use core::{now, Core};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

fn demo() -> bool {
    std::env::args().any(|a| a == "--demo")
}
/// Files carried over when the data folder moved from %APPDATA%\SmallStocks-TW
/// to %APPDATA%\MaxiStocks (the v0.1.32 rebrand): everything the app and the
/// sidecar keep side by side in the data folder.
const DATA_DIR_FILES: [&str; 4] = [
    "state.json",
    "windows.json",
    "sidecar-token.txt",
    "mcp-connection.json",
];

/// One-time copy of the data files from the legacy folder into the new one.
/// Runs only while the new folder has no state.json yet and the legacy folder
/// has one; the legacy folder is left untouched as a fallback, and nothing
/// already in the new folder is ever overwritten.
fn migrate_legacy_data_dir(new_dir: &std::path::Path, old_dir: &std::path::Path) {
    if new_dir.join("state.json").exists() || !old_dir.join("state.json").is_file() {
        return;
    }
    if std::fs::create_dir_all(new_dir).is_err() {
        return;
    }
    for name in DATA_DIR_FILES {
        let from = old_dir.join(name);
        if from.is_file() {
            let _ = std::fs::copy(&from, new_dir.join(name));
        }
    }
}

fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("SMALLSTOCKS_CONFIG_DIR") {
        return PathBuf::from(p);
    }
    // APPDATA only exists on Windows; without this macOS fell back to a purgeable temp folder.
    let base = if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("APPDATA").map(PathBuf::from)
    }
    .unwrap_or_else(std::env::temp_dir);
    if demo() {
        return base.join("MaxiStocks-demo");
    }
    let dir = base.join("MaxiStocks");
    migrate_legacy_data_dir(&dir, &base.join("SmallStocks-TW"));
    dir
}
/// Runs a background worker for the life of the app. A panic must not leave the monitor
/// silently dead: record it for the UI/MCP status and start the worker again.
fn supervise<F: FnMut() + Send + 'static>(state: mcp::Shared, name: &'static str, mut work: F) {
    std::thread::spawn(move || loop {
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut work)).is_err() {
            state.lock().unwrap_or_else(PoisonError::into_inner).error =
                Some(format!("{name}發生內部錯誤，已自動重啟"));
            std::thread::sleep(Duration::from_secs(5));
        }
    });
}
/// QA aid: with SMALLSTOCKS_CAPTURE_RAW=<folder> every source response is appended, whole, to
/// <folder>/mis-raw.ndjson with the local receive time. Off by default; nothing leaves the machine.
fn capture_raw(text: &str) {
    use std::io::Write;
    let Some(dir) = std::env::var_os("SMALLSTOCKS_CAPTURE_RAW") else {
        return;
    };
    let path = PathBuf::from(dir).join("mis-raw.ndjson");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let line = json!({"receivedAt": now(), "body": text});
        let _ = writeln!(file, "{line}");
    }
}
pub fn bridge() {
    mcp::bridge(&data_dir());
}
/// One TWSE MIS fetch + parse, shared by the plain-MIS path and the sidecar fallback.
fn fetch_mis(
    client: &reqwest::blocking::Client,
    codes: &[String],
) -> Result<Vec<core::Quote>, String> {
    client
        .get(core::mis_url(codes))
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.text())
        .map_err(|e| e.to_string())
        .and_then(|text| {
            capture_raw(&text);
            // A body that is not the expected JSON (an error page, a throttle
            // notice) is reported with its first characters, not just "decode".
            serde_json::from_str::<Value>(&text).map_err(|e| {
                format!(
                    "行情格式錯誤：{e}（開頭：{}）",
                    text.chars().take(160).collect::<String>()
                )
            })
        })
        .and_then(|v| core::parse_mis(&v, now()))
}
/// One local Shioaji sidecar fetch + parse. Loopback only, token-authenticated.
fn fetch_sidecar(
    client: &reqwest::blocking::Client,
    port: u16,
    token: &str,
    codes: &[String],
) -> Result<Vec<core::Quote>, String> {
    client
        .get(core::sidecar_url(port, codes))
        .header("X-Sidecar-Token", token)
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.text())
        .map_err(|e| e.to_string())
        .and_then(|text| {
            serde_json::from_str::<Value>(&text)
                .map_err(|e| format!("sidecar 行情格式錯誤：{e}"))
        })
        .and_then(|v| core::parse_sidecar(&v, now()))
}
/// One Taiwan fetch round under the configured source: the local Shioaji
/// sidecar first when selected (MIS fills whatever it did not return), plain
/// MIS otherwise. Ok carries the quotes served plus a note whenever a fallback
/// happened, for the UI footer; Err means the round served nothing at all.
fn fetch_tw(
    client: &reqwest::blocking::Client,
    sidecar_client: &reqwest::blocking::Client,
    quote_source: &str,
    sidecar_port: u16,
    codes: &[String],
) -> Result<(Vec<core::Quote>, Option<String>), String> {
    if quote_source != "shioaji" {
        return fetch_mis(client, codes).map(|q| (q, None));
    }
    match core::sidecar_token(&data_dir()) {
        Some(token) => match fetch_sidecar(sidecar_client, sidecar_port, &token, codes) {
            Ok(quotes) => {
                let missing: Vec<String> = codes
                    .iter()
                    .filter(|c| !quotes.iter().any(|q| &q.symbol == *c))
                    .cloned()
                    .collect();
                if missing.is_empty() {
                    Ok((quotes, None))
                } else {
                    match fetch_mis(client, &missing) {
                        Ok(mis) => Ok((
                            core::fill_missing(quotes, mis),
                            Some(format!("Shioaji 缺 {} 檔，已用 TWSE MIS 補", missing.len())),
                        )),
                        Err(e) => Ok((
                            quotes,
                            Some(format!("Shioaji 缺 {} 檔且 MIS 失敗：{e}", missing.len())),
                        )),
                    }
                }
            }
            Err(e) => fetch_mis(client, codes)
                .map(|q| (q, Some(format!("Shioaji sidecar 無回應（{e}），已改用 TWSE MIS")))),
        },
        None => fetch_mis(client, codes).map(|q| {
            (
                q,
                Some("已選 Shioaji 但找不到 sidecar token，請先啟動 sidecar".into()),
            )
        }),
    }
}
/// Quits the program (the same as the tray menu's quit), so it can be stopped from inside the app.
#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}
/// Starts the program again with the same arguments and environment after this copy has gone (a
/// second copy cannot start while this one runs: the program is single-instance), then quits.
#[tauri::command]
fn restart_app(app: tauri::AppHandle) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
        let mut script = format!(
            "Start-Sleep -Seconds 2; Start-Process -FilePath {}",
            quote(&exe.to_string_lossy())
        );
        if !args.is_empty() {
            let list: Vec<String> = args.iter().map(|a| quote(a)).collect();
            script.push_str(&format!(" -ArgumentList {}", list.join(",")));
        }
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script])
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("sh")
            .arg("-c")
            .arg("sleep 2; exec \"$0\" \"$@\"")
            .arg(&exe)
            .args(&args)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    app.exit(0);
    Ok(())
}
/// The one way back from every kind of hiding (the hide button, the close box, minimizing, the
/// privacy shortcut): the tray menu, a second launch and the shortcut all come here.
fn show_main_window(app: &tauri::AppHandle) {
    app.state::<mcp::Shared>()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .notices_hidden = false;
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    // The PTT chat window comes back too, but only when its setting is on
    // (the dispatched sync also recreates it if it is somehow gone).
    sync_chat_window(app);
}
/// The privacy shortcut is a toggle: it hides everything, and brings the monitor back when the
/// main window is hidden or minimized.
#[tauri::command]
fn privacy_toggle(app: tauri::AppHandle) {
    let showing = app
        .get_webview_window("main")
        .is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false));
    if showing {
        privacy_hide(app);
    } else {
        show_main_window(&app);
    }
}
#[tauri::command]
fn privacy_hide(app: tauri::AppHandle) {
    app.state::<mcp::Shared>()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .notices_hidden = true;
    let _ = app.emit("privacy-hide", ());
    for window in app.webview_windows().values() {
        let _ = window.hide();
    }
}
fn privacy_shortcut() -> &'static str {
    if cfg!(target_os = "macos") {
        "Command+Alt+S"
    } else {
        "Ctrl+Alt+S"
    }
}
/// Where this program is, for the MCP setup the settings page offers to copy.
#[tauri::command]
fn mcp_exe() -> Result<String, String> {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn hide_stock_detail(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("detail") {
        let _ = w.hide();
    }
}
#[tauri::command]
fn fit_minimal_height(app: tauri::AppHandle, height: f64) -> Result<(), String> {
    if !height.is_finite() || !(28.0..=410.0).contains(&height) {
        return Err("無效的視窗高度".into());
    }
    let compact = {
        let state = app.state::<mcp::Shared>();
        let s = state.lock().unwrap_or_else(PoisonError::into_inner);
        if s.saved.settings.row_display != "minimal" {
            return Ok(());
        }
        matches!(s.saved.settings.appearance.as_str(), "small" | "gray")
    };
    let w = app
        .get_webview_window("main")
        .ok_or("main window unavailable")?;
    let size = w.inner_size().map_err(|e| e.to_string())?;
    let scale = w.scale_factor().map_err(|e| e.to_string())?;
    if w.is_maximized().map_err(|e| e.to_string())? {
        return Ok(());
    }
    w.set_min_size(Some(tauri::LogicalSize::new(
        300.,
        if compact { 28. } else { 100. },
    )))
    .map_err(|e| e.to_string())?;
    w.set_size(tauri::PhysicalSize::new(
        size.width,
        (height * scale).round() as u32,
    ))
    .map_err(|e| e.to_string())?;
    if let Some(m) = w.current_monitor().map_err(|e| e.to_string())? {
        let area = m.work_area();
        let p = w.outer_position().map_err(|e| e.to_string())?;
        let bottom = area.position.y as i64 + area.size.height as i64;
        let needed = (height * scale).round() as i64;
        if p.y as i64 + needed > bottom {
            w.set_position(tauri::PhysicalPosition::new(
                p.x,
                (bottom - needed).max(area.position.y as i64) as i32,
            ))
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
#[tauri::command]
fn show_stock_detail(
    app: tauri::AppHandle,
    detail: Value,
    x: f64,
    y: f64,
    top: f64,
) -> Result<(), String> {
    if ![x, y, top]
        .iter()
        .all(|n| n.is_finite() && n.abs() < 20000.)
        || detail.to_string().len() > 8192
    {
        return Err("無效的詳細資料".into());
    }
    if app
        .state::<mcp::Shared>()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .notices_hidden
    {
        return Ok(());
    }
    let main = app
        .get_webview_window("main")
        .ok_or("main window unavailable")?;
    let w = app
        .get_webview_window("detail")
        .ok_or("detail window unavailable")?;
    let position = main.outer_position().map_err(|e| e.to_string())?;
    let scale = main.scale_factor().map_err(|e| e.to_string())?;
    let width = 280.;
    let height = 186.;
    let mut px = position.x as f64 + x * scale;
    let mut py = position.y as f64 + (y + 5.) * scale;
    if let Some(m) = main.current_monitor().map_err(|e| e.to_string())? {
        let area = m.work_area();
        let left = area.position.x as f64;
        let right = left + area.size.width as f64;
        let above = area.position.y as f64;
        let below = above + area.size.height as f64;
        if py + height * scale > below {
            py = position.y as f64 + top * scale - height * scale - 5. * scale;
        }
        px = px.clamp(left + 4., (right - width * scale - 4.).max(left + 4.));
        py = py.clamp(above + 4., (below - height * scale - 4.).max(above + 4.));
    }
    w.set_size(tauri::LogicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    w.set_position(tauri::PhysicalPosition::new(px as i32, py as i32))
        .map_err(|e| e.to_string())?;
    app.emit_to("detail", "stock-detail", detail)
        .map_err(|e| e.to_string())?;
    w.show().map_err(|e| e.to_string())
}
#[tauri::command]
fn core_request(
    app: tauri::AppHandle,
    state: tauri::State<mcp::Shared>,
    tool: String,
    args: Value,
) -> Result<Value, String> {
    let result = state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .execute(&tool, args)?;
    if tool == "set_settings" {
        sync_main_window(&app)?;
        // Fire-and-forget: building the chat window must not happen in
        // this synchronous handler's stack (see sync_chat_window).
        sync_chat_window(&app);
        // Bring the Shioaji sidecar up if the new settings call for it.
        // Conditions, a liveness probe and an attempt budget inside
        // ensure_sidecar keep this a cheap no-op when nothing applies.
        let _ = ensure_sidecar(&app, false);
    }
    Ok(result)
}
fn sync_main_window(app: &tauri::AppHandle) -> Result<(), String> {
    let enabled = app
        .state::<mcp::Shared>()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .saved
        .settings
        .always_on_top;
    let window = app
        .get_webview_window("main")
        .ok_or("main window unavailable")?;
    window.set_always_on_top(enabled).map_err(|e| e.to_string())
}
/// The Shioaji keys are set only from the settings page, through these
/// commands: nothing here is registered with MCP, and the key values are
/// never returned — status reports two booleans, set/clear return
/// nothing. The keys live in their own file (see sidecar_keys.rs),
/// outside the core's state entirely.
#[tauri::command]
fn shioaji_keys_set(api_key: String, secret_key: String) -> Result<(), String> {
    sidecar_keys::save(&data_dir(), &api_key, &secret_key)
}
#[tauri::command]
fn shioaji_keys_status() -> Value {
    sidecar_keys::status_json(&data_dir())
}
#[tauri::command]
fn shioaji_keys_clear() -> Result<(), String> {
    sidecar_keys::clear(&data_dir())
}
/// Manual trigger for the settings page's start button: same conditions
/// and liveness probe as the automatic path, but it skips the retry
/// interval (the per-run attempt cap still applies). The returned line
/// says what happened, for the settings page to show.
#[tauri::command]
fn sidecar_start_now(app: tauri::AppHandle) -> Result<String, String> {
    Ok(ensure_sidecar(&app, true))
}
/// Spawn-attempt bookkeeping for ensure_sidecar: how many times this run
/// has tried to start the sidecar and when the last try happened. Shell
/// state only — process management, like the chat-window sync — and
/// deliberately not in the core, whose single ownership covers
/// monitoring, rules, events and settings.
#[derive(Default)]
struct SidecarStartGate(Mutex<SidecarStartAttempts>);
#[derive(Default)]
struct SidecarStartAttempts {
    attempts: u32,
    last: Option<std::time::Instant>,
}
const SIDECAR_MAX_ATTEMPTS: u32 = 5;
const SIDECAR_RETRY_INTERVAL: Duration = Duration::from_secs(30);
/// Bring the Shioaji sidecar up when every condition holds: the quote
/// source is Shioaji, auto-start is on, both keys are stored, and the
/// configured folder holds shioaji_sidecar.py. A sidecar that already
/// answers on its port is left alone (the probe is a bare TCP connect —
/// /health demands the token header, so it cannot serve as the probe).
/// The sidecar is spawned as `python shioaji_sidecar.py` in its folder
/// with the keys in its environment, exactly like run-sidecar.bat;
/// python is a console program, so it gets its own console window with
/// the listening line and its logs — nothing is hidden. Returns a line
/// describing the outcome (the settings page shows it for the manual
/// button); automatic callers ignore it, except that spawn failures and
/// a spent attempt budget are also written into the core's error field,
/// where the settings page surfaces errors.
fn ensure_sidecar(app: &tauri::AppHandle, manual: bool) -> String {
    let (quote_source, auto_start, sidecar_dir, sidecar_port) = {
        let shared = app.state::<mcp::Shared>();
        let s = shared.lock().unwrap_or_else(PoisonError::into_inner);
        (
            s.saved.settings.quote_source.clone(),
            s.saved.settings.sidecar_auto_start,
            s.saved.settings.sidecar_dir.clone(),
            s.saved.settings.sidecar_port,
        )
    };
    if quote_source != "shioaji" {
        return "未啟動：行情來源不是永豐 Shioaji。".into();
    }
    if !auto_start {
        return "未啟動：請先開啟 sidecar 自動啟動。".into();
    }
    let dir = data_dir();
    let Some(keys) = sidecar_keys::load(&dir) else {
        return "未啟動：Shioaji 金鑰尚未儲存。".into();
    };
    let folder = sidecar_dir.trim();
    if folder.is_empty() {
        return "未啟動：尚未設定 sidecar 資料夾路徑。".into();
    }
    if !std::path::Path::new(folder)
        .join("shioaji_sidecar.py")
        .is_file()
    {
        return "未啟動：sidecar 資料夾裡找不到 shioaji_sidecar.py。".into();
    }
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], sidecar_port));
    if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(400)).is_ok() {
        return "sidecar 已在執行中。".into();
    }
    {
        let gate = app.state::<SidecarStartGate>();
        let mut g = gate.0.lock().unwrap_or_else(PoisonError::into_inner);
        if g.attempts >= SIDECAR_MAX_ATTEMPTS {
            let msg = format!(
                "sidecar 自動啟動已嘗試 {SIDECAR_MAX_ATTEMPTS} 次仍未連上，已停止嘗試；請確認 python 已安裝、sidecar 資料夾正確，重新啟動程式後再試"
            );
            app.state::<mcp::Shared>()
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .error = Some(msg.clone());
            return msg;
        }
        if !manual && g.last.is_some_and(|t| t.elapsed() < SIDECAR_RETRY_INTERVAL) {
            return "sidecar 剛嘗試啟動過，節流中，稍後會再試。".into();
        }
        g.attempts += 1;
        g.last = Some(std::time::Instant::now());
    }
    match std::process::Command::new("python")
        .arg("shioaji_sidecar.py")
        .current_dir(folder)
        .env("SHIOAJI_API_KEY", &keys.api_key)
        .env("SHIOAJI_SECRET_KEY", &keys.secret_key)
        .env("SMALLSTOCKS_CONFIG_DIR", &dir)
        .spawn()
    {
        Ok(_child) => "sidecar 已啟動，請看它自己的主控台視窗確認 listening 行。".into(),
        Err(e) => {
            let msg = format!("sidecar 自動啟動失敗：{e}");
            app.state::<mcp::Shared>()
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .error = Some(msg.clone());
            msg
        }
    }
}
/// The PTT chat window: a plain decorated webview on the channel's public
/// preview feed (t.me/s renders server-side, so no Telegram login is
/// needed). The label appears in no capability file, so the remote page
/// gets no IPC access at all — it is a browser pane, nothing more.
const CHAT_WINDOW_LABEL: &str = "ptt-chat";
const CHAT_URL: &str = "https://t.me/s/ptt_stock_follow_chat";
/// Ask for the chat window to be reconciled with the `chatWindow`
/// setting. This never touches a window itself: building a WebviewWindow
/// runs WebView2 controller setup, which needs the main-thread message
/// loop to make progress, and most callers — the synchronous
/// `core_request` command above all — sit inside that loop's call stack.
/// Building there wedged the half-created window (blank, its close box
/// dead) and every command queued behind the handler. So the real work
/// is posted to the event loop from a throwaway thread — posting from
/// another thread guarantees the closure is queued for the loop, never
/// run inline in the caller's stack — and this returns immediately.
/// Fire-and-forget: the worker (apply_chat_window) re-reads the setting
/// when it runs, and failures land in the core's error field, where the
/// settings page surfaces them. Called after every set_settings (UI and
/// MCP alike), at startup, and when the main window is shown again.
fn sync_chat_window(app: &tauri::AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || {
        let worker = handle.clone();
        let _ = handle.run_on_main_thread(move || apply_chat_window(&worker));
    });
}
/// The event-loop half of `sync_chat_window`: make the chat window match
/// the setting as it reads *now* — create and show it when the setting
/// is on, close it when off. Re-reading here (rather than trusting the
/// value at request time) keeps a fast on-off-on from acting on a stale
/// copy. Runs only as an event-loop callback, so WebView2 controller
/// initialization can proceed.
fn apply_chat_window(app: &tauri::AppHandle) {
    let enabled = app
        .state::<mcp::Shared>()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .saved
        .settings
        .chat_window;
    let result = match app.get_webview_window(CHAT_WINDOW_LABEL) {
        Some(w) if enabled => {
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
            Ok(())
        }
        Some(w) => w.close().map_err(|e| e.to_string()),
        None if enabled => tauri::Url::parse(CHAT_URL)
            .map_err(|_| "聊天視窗網址無效".to_string())
            .and_then(|url| {
                tauri::WebviewWindowBuilder::new(
                    app,
                    CHAT_WINDOW_LABEL,
                    tauri::WebviewUrl::External(url),
                )
                .title("PTT 股票閒聊")
                .inner_size(440., 640.)
                .build()
                .map(|_| ())
                .map_err(|e| e.to_string())
            }),
        None => Ok(()),
    };
    if let Err(e) = result {
        app.state::<mcp::Shared>()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .error = Some(format!("聊天視窗開啟失敗：{e}"));
    }
}
#[tauri::command]
async fn search_stocks(query: String) -> Result<Value, String> {
    if query.trim().is_empty() || query.chars().count() > 40 {
        return Err("請輸入名稱或代號（最多 40 字）".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| e.to_string())?;
        let v: Value = client
            .get("https://mis.twse.com.tw/stock/api/getStockNames.jsp")
            .query(&[("n", query.trim())])
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json())
            .map_err(|e| e.to_string())?;
        let mut list: Vec<Value> = v["datas"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| {
                let c = r["c"].as_str().and_then(core::symbol)?;
                Some(json!({"symbol":c,"name":r["n"].as_str().unwrap_or("")}))
            })
            .collect();
        list.sort_by_key(|r| {
            if r["name"].as_str() == Some(query.trim())
                || r["symbol"].as_str() == Some(query.trim())
            {
                0
            } else if r["symbol"].as_str().is_some_and(|c| c.len() == 4) {
                1
            } else {
                2
            }
        });
        list.truncate(30);
        Ok(json!(list))
    })
    .await
    .map_err(|e| e.to_string())?
}
pub fn run() {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).expect("cannot create app data directory");
    let mut context = tauri::generate_context!();
    // Test profiles get their own instance lock, so QA cannot focus the user's running app.
    if std::env::var_os("SMALLSTOCKS_CONFIG_DIR").is_some() {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        std::fs::canonicalize(&dir)
            .unwrap_or_else(|_| dir.clone())
            .to_string_lossy()
            .to_lowercase()
            .hash(&mut hash);
        context.config_mut().identifier =
            format!("{}.test.{:x}", context.config().identifier, hash.finish());
    }
    let state: mcp::Shared = Arc::new(Mutex::new(
        Core::load(dir.join("state.json"), demo())
            .expect("cannot read settings; original file preserved"),
    ));
    // One automatic-backup attempt per launch; the core throttles it to
    // once a day and stays silent when backups are off or unconfigured.
    state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .maybe_backup(false);
    let setup_state = state.clone();
    if let Some(window) = context
        .config_mut()
        .app
        .windows
        .iter_mut()
        .find(|w| w.label == "main")
    {
        window.always_on_top = state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .saved
            .settings
            .always_on_top;
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_main_window(app);
        }))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_filename(dir.join("windows.json").to_string_lossy().to_string())
                .with_state_flags(tauri_plugin_window_state::StateFlags::POSITION)
                .build(),
        )
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _, event| {
                    if event.state() == ShortcutState::Pressed {
                        privacy_toggle(app.clone());
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .manage(SidecarStartGate::default())
        .invoke_handler(tauri::generate_handler![
            mcp_exe,
            privacy_toggle,
            quit_app,
            restart_app,
            core_request,
            privacy_hide,
            search_stocks,
            show_stock_detail,
            hide_stock_detail,
            fit_minimal_height,
            shioaji_keys_set,
            shioaji_keys_status,
            shioaji_keys_clear,
            sidecar_start_now
        ])
        .setup(move |app| {
            tauri::WebviewWindowBuilder::new(
                app,
                "detail",
                tauri::WebviewUrl::App("detail.html".into()),
            )
            .title("Stock details")
            .inner_size(280., 186.)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .focused(false)
            .focusable(false)
            .skip_taskbar(true)
            .visible(false)
            .build()?
            .set_ignore_cursor_events(true)?;
            tauri::WebviewWindowBuilder::new(
                app,
                "notify",
                tauri::WebviewUrl::App("notify.html".into()),
            )
            .title("Maxi stocks notice")
            .inner_size(300., 56.)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .focused(false)
            .focusable(false)
            .skip_taskbar(true)
            .visible(false)
            .build()?;
            let show = MenuItem::with_id(app, "show", "顯示 Maxi stocks", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "結束監看並離開", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray = TrayIconBuilder::new()
                .tooltip("Maxi stocks · 本機監看")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone())
            }
            tray.build(app)?;
            // Restore the PTT chat window if its setting was left on.
            // Dispatched like every other sync point: the window is built
            // from the event loop once it is running, and a failure lands
            // in the core's error field (see apply_chat_window).
            sync_chat_window(app.handle());
            // A long-running test instance must not take the user's own hide shortcut from them.
            let skip_shortcut = std::env::var_os("SMALLSTOCKS_NO_GLOBAL_SHORTCUT").is_some();
            let registered = if skip_shortcut {
                Ok(())
            } else {
                app.global_shortcut().register(privacy_shortcut())
            };
            if let Err(error) = registered {
                setup_state.lock().unwrap_or_else(PoisonError::into_inner).privacy_shortcut_error = Some(format!("全域隱藏快捷鍵無法註冊（可能已被占用）：{error}。程式內仍可使用快捷鍵或隱藏按鈕。"));
            }
            if let Err(e) = mcp::serve(setup_state.clone(), &dir, app.handle().clone()) {
                setup_state.lock().unwrap_or_else(PoisonError::into_inner).error = Some(format!("MCP 啟動失敗：{e}"));
            }
            // Auto-start the Shioaji sidecar when the saved settings and
            // the locally stored keys call for it; a no-op otherwise,
            // and failures land in the core's error field (see
            // ensure_sidecar).
            let _ = ensure_sidecar(app.handle(), false);
            let handle = app.handle().clone();
            let poll_state = setup_state.clone();
            supervise(setup_state.clone(), "行情輪詢", move || {
                let client = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(15))
                    .user_agent(concat!("MaxiStocks/", env!("CARGO_PKG_VERSION")))
                    .build()
                    .unwrap();
                // The sidecar is on loopback: fail fast so the MIS fallback is not delayed.
                let sidecar_client = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(3))
                    .build()
                    .unwrap();
                let mut tick = 0;
                loop {
                    std::thread::sleep(Duration::from_millis(500));
                    let (tw_codes, yahoo_codes, taifex_codes, is_demo, revision, quote_source, sidecar_port) = {
                        let mut s = poll_state.lock().unwrap_or_else(PoisonError::into_inner);
                        let at = now();
                        // Polling covers the union of every watch group, not just the active tab.
                        let all_codes = s.saved.all_codes();
                        if s.saved.settings.paused || at < s.next_poll || all_codes.is_empty() {
                            continue;
                        }
                        let tw_codes: Vec<String> = all_codes
                            .iter()
                            .filter(|c| core::is_tw_symbol(c))
                            .cloned()
                            .collect();
                        // TAIFEX family codes split out before the Yahoo bucket:
                        // letters only, so without this they would fall through
                        // to Yahoo, which has no Taiwan futures to give them.
                        let taifex_codes: Vec<String> = all_codes
                            .iter()
                            .filter(|c| core::is_taifex_symbol(c))
                            .cloned()
                            .collect();
                        let mut yahoo_codes: Vec<String> = all_codes
                            .iter()
                            .filter(|c| !core::is_tw_symbol(c) && !core::is_taifex_symbol(c))
                            .cloned()
                            .collect();
                        // Internal USD/TWD rate for the header's combined
                        // (≈ TWD) total: fetched whenever a USD position
                        // exists. It joins no watch group and renders as
                        // no row; after ingest it is parked in the quotes
                        // map directly, since ingest drops unwatched codes.
                        let has_usd_position = s
                            .saved
                            .positions
                            .iter()
                            .any(|p| !core::is_tw_symbol(&p.code) && !core::is_taifex_symbol(&p.code));
                        if has_usd_position && !yahoo_codes.iter().any(|c| c == "TWD=X") {
                            yahoo_codes.push("TWD=X".into());
                        }
                        // The TWSE polling window gates only the Taiwan fetch. Yahoo
                        // symbols trade on US hours (Taipei night) and TAIFEX
                        // symbols trade the futures night session, so either
                        // keeps polling whenever any are watched; with none
                        // watched the loop rests outside the window exactly as
                        // before. A cold start (nothing fetched yet) still
                        // fills every list once.
                        let tw_due = s.demo || s.polling_window(at) || s.last_fetch.is_none();
                        if !tw_due && yahoo_codes.is_empty() && taifex_codes.is_empty() {
                            s.next_poll = at + 60_000;
                            continue;
                        }
                        s.next_poll = at + s.saved.settings.refresh_seconds as i64 * 1000;
                        (
                            if tw_due { tw_codes } else { Vec::new() },
                            yahoo_codes,
                            taifex_codes,
                            s.demo,
                            s.config_revision,
                            s.saved.settings.quote_source.clone(),
                            s.saved.settings.sidecar_port,
                        )
                    };
                    let mut note: Option<String> = None;
                    let result = if is_demo {
                        tick += 1;
                        let mut codes = tw_codes.clone();
                        codes.extend(yahoo_codes.iter().cloned());
                        codes.extend(taifex_codes.iter().cloned());
                        Ok(core::demo_quotes(&codes, tick))
                    } else {
                        let mut quotes: Vec<core::Quote> = Vec::new();
                        let mut errors: Vec<String> = Vec::new();
                        if !tw_codes.is_empty() {
                            match fetch_tw(
                                &client,
                                &sidecar_client,
                                &quote_source,
                                sidecar_port,
                                &tw_codes,
                            ) {
                                Ok((served, n)) => {
                                    quotes.extend(served);
                                    note = n;
                                }
                                Err(e) => errors.push(e),
                            }
                        }
                        if !yahoo_codes.is_empty() {
                            match yahoo::fetch(&client, &yahoo_codes) {
                                Ok(served) => quotes.extend(served),
                                Err(e) => errors.push(e),
                            }
                        }
                        if !taifex_codes.is_empty() {
                            match taifex::fetch(&client, &taifex_codes) {
                                Ok(served) => quotes.extend(served),
                                Err(e) => errors.push(e),
                            }
                        }
                        if quotes.is_empty() && !errors.is_empty() {
                            Err(errors.join("；"))
                        } else {
                            // One source failing while the other serves is a note,
                            // not an outage: the served quotes still ingest.
                            if !errors.is_empty() {
                                let extra = format!("部分行情來源失敗：{}", errors.join("；"));
                                note = Some(match note {
                                    Some(n) => format!("{n}；{extra}"),
                                    None => extra,
                                });
                            }
                            Ok(quotes)
                        }
                    };
                    let mut s = poll_state.lock().unwrap_or_else(PoisonError::into_inner);
                    if s.config_revision != revision || s.saved.settings.paused {
                        continue;
                    }
                    match result {
                        Ok(quotes) => {
                            // The internal TWD=X rate rides the Yahoo fetch
                            // but is not a watched symbol, so ingest would
                            // drop it: set it aside and park it in the
                            // quotes map, where the header readout looks.
                            let fx = quotes.iter().find(|q| q.symbol == "TWD=X").cloned();
                            let events = s.ingest(quotes, now());
                            if let Some(fx) = fx {
                                s.quotes.insert("TWD=X".into(), fx);
                            }
                            s.source_note = note;
                            if !s.notices_hidden {
                                for e in &events {
                                    let _ = handle.emit("stock-notice", e);
                                }
                            }
                            // Mirror strip/card events to Telegram alongside
                            // the local notices: one fire-and-forget send
                            // per event on its own thread (the poll loop and
                            // the local notices never wait on it), silent on
                            // failure — the request URL carries the bot
                            // token and must never reach a log or the UI.
                            if !s.demo {
                                let tg_settings = s.saved.settings.clone();
                                for (url, body) in core::telegram_alerts(&tg_settings, &events) {
                                    std::thread::spawn(move || {
                                        if let Ok(client) =
                                            reqwest::blocking::Client::builder()
                                                .timeout(Duration::from_secs(10))
                                                .build()
                                        {
                                            let _ = client.post(&url).json(&body).send();
                                        }
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            s.source_note = note;
                            s.error = Some(format!("行情連線失敗：{e}"));
                            s.failures = (s.failures + 1).min(6);
                            s.next_poll = now()
                                + ((s.saved.settings.refresh_seconds * 2u64.pow(s.failures))
                                    .min(300) as i64)
                                    * 1000;
                            s.reset_baselines();
                        }
                    }
                }
            });
            let line_state = setup_state.clone();
            supervise(setup_state.clone(), "分鐘線佇列", move || {
                let client = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(15))
                    .user_agent("Mozilla/5.0 MaxiStocks")
                    .build()
                    .unwrap();
                loop {
                    std::thread::sleep(Duration::from_secs(2));
                    let task = line_state.lock().unwrap_or_else(PoisonError::into_inner).intraday_queue.pop_front();
                    let Some(code) = task else { continue };
                    let (demo, otc) = {
                        let s = line_state.lock().unwrap_or_else(PoisonError::into_inner);
                        (
                            s.demo,
                            s.saved
                                .daily_cache
                                .values()
                                .any(|v| v.symbol == code && v.source == "TPEx"),
                        )
                    };
                    let result = if demo {
                        Ok(intraday::demo_series(&code))
                    } else {
                        intraday::fetch(&client, &code, otc)
                    };
                    let mut s = line_state.lock().unwrap_or_else(PoisonError::into_inner);
                    s.intraday_pending.remove(&code);
                    match result {
                        Ok(series) => {
                            if s.saved.watches(&code) {
                                s.saved.intraday_cache.insert(code.clone(), series);
                                s.intraday_errors.remove(&code);
                                if let Err(e) = s.persist() {
                                    s.intraday_errors
                                        .insert(code, (format!("分鐘線快取無法儲存：{e}"), now()));
                                }
                            }
                        }
                        Err(e) => {
                            s.intraday_errors.insert(code, (e, now()));
                        }
                    }
                    s.revision += 1;
                }
            });
            let history_state = setup_state.clone();
            supervise(setup_state.clone(), "日資料佇列", move || {
                let client = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(20))
                    .user_agent(concat!("MaxiStocks/", env!("CARGO_PKG_VERSION")))
                    .build()
                    .unwrap();
                loop {
                    std::thread::sleep(Duration::from_secs(2));
                    let task = history_state.lock().unwrap_or_else(PoisonError::into_inner).daily_queue.pop_front();
                    let Some((code, month)) = task else { continue };
                    let is_demo = history_state.lock().unwrap_or_else(PoisonError::into_inner).demo;
                    let result = if is_demo {
                        Ok(daily::demo_series(&code, &month))
                    } else {
                        daily::fetch(&client, &code, &month)
                    };
                    let mut s = history_state.lock().unwrap_or_else(PoisonError::into_inner);
                    let key = format!("{code}:{month}");
                    s.daily_pending.remove(&key);
                    match result {
                        Ok(series) => {
                            s.saved.daily_cache.insert(key.clone(), series);
                            s.daily_errors.remove(&key);
                            while s.saved.daily_cache.len() > 80 {
                                let oldest = s
                                    .saved
                                    .daily_cache
                                    .iter()
                                    .min_by_key(|(_, s)| s.fetched_at)
                                    .map(|(k, _)| k.clone())
                                    .unwrap();
                                s.saved.daily_cache.remove(&oldest);
                            }
                            if let Err(e) = s.persist() {
                                s.daily_errors
                                    .insert(key, (format!("歷史快取無法儲存：{e}"), now()));
                            }
                        }
                        Err(e) => {
                            s.daily_errors.insert(key, (e, now()));
                        }
                    }
                    s.revision += 1;
                }
            });
            let pf_history_state = setup_state.clone();
            supervise(setup_state.clone(), "組合歷史佇列", move || {
                let client = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(30))
                    .user_agent("Mozilla/5.0 MaxiStocks")
                    .build()
                    .unwrap();
                loop {
                    std::thread::sleep(Duration::from_secs(2));
                    let task = pf_history_state.lock().unwrap_or_else(PoisonError::into_inner).history_queue.pop_front();
                    let Some(code) = task else { continue };
                    let is_demo = pf_history_state.lock().unwrap_or_else(PoisonError::into_inner).demo;
                    // Display-only data: the series lands in the in-memory
                    // history cache and reaches the UI as aggregates on the
                    // snapshot; it is never persisted and never ingested.
                    let result = if is_demo {
                        Ok(history::demo_series(&code))
                    } else {
                        history::fetch(&client, &code)
                    };
                    let mut s = pf_history_state.lock().unwrap_or_else(PoisonError::into_inner);
                    s.history_pending.remove(&code);
                    match result {
                        Ok(series) => {
                            s.history_cache.insert(code.clone(), series);
                            s.history_errors.remove(&code);
                        }
                        Err(e) => {
                            s.history_errors.insert(code, (e, now()));
                        }
                    }
                    s.revision += 1;
                }
            });
            let calendar_state = setup_state.clone();
            supervise(setup_state.clone(), "休市日曆", move || {
                let client = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(20))
                    .user_agent(concat!("MaxiStocks/", env!("CARGO_PKG_VERSION")))
                    .build()
                    .unwrap();
                loop {
                    let due = {
                        let s = calendar_state.lock().unwrap_or_else(PoisonError::into_inner);
                        if s.demo {
                            Vec::new()
                        } else {
                            s.calendar_due(now())
                        }
                    };
                    for year in due {
                        let result = calendar::fetch(&client, year);
                        let mut s = calendar_state.lock().unwrap_or_else(PoisonError::into_inner);
                        match result {
                            Ok(table) => {
                                s.saved.calendars.insert(year.to_string(), table);
                                s.calendar_errors.remove(&year);
                                if let Err(e) = s.persist() {
                                    s.calendar_errors
                                        .insert(year, (format!("休市日曆無法儲存：{e}"), now()));
                                }
                            }
                            Err(e) => {
                                s.calendar_errors.insert(year, (e, now()));
                            }
                        }
                        s.revision += 1;
                    }
                    std::thread::sleep(Duration::from_secs(30));
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    hide_stock_detail(window.app_handle().clone());
                    let _ = window.hide();
                }
            }
            if window.label() == CHAT_WINDOW_LABEL {
                if let tauri::WindowEvent::CloseRequested { .. } = event {
                    // The user closed the chat window by hand: flip the
                    // setting off so state and screen agree (the next poll
                    // re-renders the settings page with the toggle off).
                    // When apply_chat_window closes it programmatically
                    // the setting is already false, so this stays a no-op.
                    // Quitting the app destroys windows without a
                    // CloseRequested, so the setting survives a restart.
                    // The write-back runs on a throwaway thread: this
                    // handler must return at once, never sit on the state
                    // lock (and the persist it triggers) while the window
                    // is mid-close.
                    let app = window.app_handle().clone();
                    std::thread::spawn(move || {
                        let state = app.state::<mcp::Shared>();
                        let mut core = state.lock().unwrap_or_else(PoisonError::into_inner);
                        if core.saved.settings.chat_window {
                            let _ = core.execute("set_settings", json!({"chatWindow": false}));
                        }
                    });
                }
            }
        })
        .run(context)
        .expect("desktop runtime failed");
}

#[cfg(test)]
mod tests {
    use super::migrate_legacy_data_dir;
    use std::path::PathBuf;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "maxistocks-migrate-{tag}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn migration_copies_every_data_file_into_an_empty_new_dir() {
        let root = temp_root("copy");
        let new = root.join("MaxiStocks");
        let old = root.join("SmallStocks-TW");
        std::fs::create_dir_all(&old).unwrap();
        for name in [
            "state.json",
            "windows.json",
            "sidecar-token.txt",
            "mcp-connection.json",
        ] {
            std::fs::write(old.join(name), format!("content of {name}")).unwrap();
        }
        migrate_legacy_data_dir(&new, &old);
        for name in [
            "state.json",
            "windows.json",
            "sidecar-token.txt",
            "mcp-connection.json",
        ] {
            assert_eq!(
                std::fs::read_to_string(new.join(name)).unwrap(),
                format!("content of {name}")
            );
        }
        // The legacy folder is left in place as a fallback.
        assert!(old.join("state.json").is_file());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn migration_never_touches_a_new_dir_that_already_has_state() {
        let root = temp_root("keep");
        let new = root.join("MaxiStocks");
        let old = root.join("SmallStocks-TW");
        std::fs::create_dir_all(&new).unwrap();
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(new.join("state.json"), "new state").unwrap();
        std::fs::write(old.join("state.json"), "old state").unwrap();
        std::fs::write(old.join("sidecar-token.txt"), "old token").unwrap();
        migrate_legacy_data_dir(&new, &old);
        assert_eq!(
            std::fs::read_to_string(new.join("state.json")).unwrap(),
            "new state"
        );
        // Not even the other files are copied once the new dir is in use.
        assert!(!new.join("sidecar-token.txt").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn migration_is_a_noop_without_a_legacy_state() {
        let root = temp_root("noop");
        let new = root.join("MaxiStocks");
        let old = root.join("SmallStocks-TW");
        // Legacy folder missing entirely.
        migrate_legacy_data_dir(&new, &old);
        assert!(!new.join("state.json").exists());
        // Legacy folder present but holding no state.json: nothing to carry.
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("windows.json"), "{}").unwrap();
        migrate_legacy_data_dir(&new, &old);
        assert!(!new.join("state.json").exists());
        assert!(!new.join("windows.json").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
