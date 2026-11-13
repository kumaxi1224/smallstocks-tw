# Shioaji sidecar（永豐即時行情）— 原型與整合設計

目的：讓 Maxi stocks 改用永豐金證券 Shioaji 的即時逐筆行情，
取代 TWSE MIS 快照（盤中約 1–2 分鐘才更新一次），股價更即時、更準確。
只讀行情：不下載憑證、不啟用 CA、程式裡沒有任何下單呼叫，物理上無法成交。

## 架構

```
Shioaji 串流 (Tick + BidAsk)
        │  shioaji (Python, 只有它有官方函式庫)
        ▼
sidecar/shioaji_sidecar.py ── 快取每檔最新一筆
        │  HTTP 127.0.0.1:18765，X-Sidecar-Token 驗證（比照 MCP bridge 模式）
        ▼
src-tauri/src/core.rs ── 唯一的監看核心照原節奏抓取 → ingest() → 規則/通知
        └─ sidecar 無回應或某檔無資料時，該檔自動退回 TWSE MIS
```

遵守 repo 既有規則：core.rs 仍是唯一擁有者，UI/MCP 不各自輪詢；
source_at（來源時間）與 received_at（取得時間）分開；
成交價只來自 Tick 的成交價，bid/ask 只用於 mid，不冒充成交價；
新鮮度沿用 FRESH_MS（180 秒），Shioaji 逐筆遠比這即時。

## Sidecar API

- `GET /health` → `{"ok": true, ...}`
- `GET /quotes?codes=2330,0050` → `{"quotes": [Quote, ...]}`
  欄位與 core.rs 的 `Quote`（camelCase）完全一致：symbol, name, price,
  bid, ask, previous, changePercent, mid, midChangePercent, tradeAt,
  dayHigh, dayLow, sourceAt, receivedAt, date, source="Shioaji"。
- previous（昨收）由 Tick 的 pct_chg 反推：`close / (1 + pct_chg/100)`，
  不依賴合約靜態欄位；dayHigh/dayLow 由當日逐筆累計。
- 首次被請求的代號才訂閱（TSE 找不到再找 OTC），避免訂閱額度浪費。

## 啟動

```powershell
# 測試鏈路（免金鑰、合成行情）
python shioaji_sidecar.py --mock

# 模擬環境（開通流程的測試階段用）
$env:SHIOAJI_API_KEY="..."; $env:SHIOAJI_SECRET_KEY="..."
python shioaji_sidecar.py --simulation

# 正式環境
python shioaji_sidecar.py
```

金鑰只放環境變數，不寫進任何設定檔或 repo。
token 首次啟動產生：有設 `SMALLSTOCKS_CONFIG_DIR` 時寫入該目錄的
`sidecar-token.txt`（app 的資料目錄，app 從那裡讀）；沒設時寫在 sidecar
同目錄。app 以 header 帶入 token；sidecar 只綁 127.0.0.1，不對外。

## Tauri 端整合（已完成，2026-11-12）

1. core.rs 新增 `parse_sidecar`（比照 `parse_mis` 的紀律：mid 與漲跌幅在 app 端
   重算、received_at 用 app 自己的時鐘、無 sourceAt 或日期不合者丟棄）、
   `fill_missing`（sidecar 優先、MIS 只補缺檔）、`sidecar_url`、`sidecar_token`。
2. 抓取順序：設定 `quoteSource = "shioaji"` 時先問 sidecar（3 秒逾時），
   缺的代號再用 MIS 補；sidecar 整個連不上就全走 MIS，行為與原本相同；
   任何退回都會在頁尾與設定頁註明（Core.source_note），不靜默。
3. 設定頁可切換行情來源與 sidecar 連接埠（預設 18765）；事件新增 source 欄位，
   通知卡片標示事件真正的來源。
4. 驗證：cargo test 33 個全過（含 5 個新增）、npm build 與 29 個前端測試全過；
   sidecar mock 模式端到端實測（token 寫入 SMALLSTOCKS_CONFIG_DIR 後取價成功）。
   尚未做的是 Windows 實機長時間盤中驗證（需等 API 金鑰開通後以真行情跑）。

## Maxi 需先完成的前置（永豐端，程式代替不了）

1. 有永豐金證券帳戶（沒有要先開戶）。
2. 到理財網「個人服務 → API 管理」新增 API Key：
   https://www.sinotrade.com.tw/newweb/PythonAPIKey/
   完成手機/Email 雙因子驗證，設定到期日、勾選行情權限、設定 IP 限制；
   **API Key 與 Secret Key 只顯示一次，當場抄下保存**。
3. 在簽署中心簽署 API 約定書（證券與期貨分開，本案只需證券）。
4. 依規定先在模擬環境完成登入測試（測試報告），正式環境才會開通；
   金鑰啟用後約隔一個交易日才生效。
5. 只讀行情**不需要**下載/啟用 CA 憑證——憑證是下單用的，本整合永遠不碰。
