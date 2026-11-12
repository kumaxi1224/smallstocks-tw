#!/usr/bin/env python3
"""Shioaji quote sidecar for SmallStocks TW (prototype).

Serves the latest SinoPac Shioaji quotes to the Tauri app over a
token-protected loopback HTTP endpoint, in the same JSON shape as the
app's internal Quote struct (camelCase). Read-only by design:

- logs in with an API key / secret key only
- never downloads or activates a CA certificate
- never calls any order API, so it cannot place trades

Modes:
  --mock        synthetic quotes, no Shioaji login (for chain testing)
  (default)     real Shioaji login; needs env SHIOAJI_API_KEY and
                SHIOAJI_SECRET_KEY; add --simulation for the test env

Usage:
  python3 shioaji_sidecar.py --mock --port 18765
  SHIOAJI_API_KEY=... SHIOAJI_SECRET_KEY=... python3 shioaji_sidecar.py

The auth token is generated on first run into sidecar-token.txt next to
this script (same pattern as the app's MCP bridge token). The app reads
that file and sends the token as X-Sidecar-Token. Binds 127.0.0.1 only.
"""

import argparse
import json
import os
import secrets
import sys
import threading
import time
from datetime import datetime
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

HERE = os.path.dirname(os.path.abspath(__file__))
TOKEN_FILE = os.path.join(HERE, "sidecar-token.txt")
TZ_OFFSET_MS = 8 * 3600 * 1000  # Asia/Taipei, matches the app's +0800 handling


def now_ms() -> int:
    return int(time.time() * 1000)


def load_or_create_token() -> str:
    # When the app's config dir is known, the token lives there so the app
    # finds it at <data_dir>/sidecar-token.txt; otherwise next to this script.
    config_dir = os.environ.get("SMALLSTOCKS_CONFIG_DIR")
    token_file = (os.path.join(config_dir, "sidecar-token.txt")
                  if config_dir else TOKEN_FILE)
    if os.path.exists(token_file):
        with open(token_file, "r", encoding="utf-8") as f:
            tok = f.read().strip()
        if tok:
            return tok
    tok = secrets.token_hex(16)
    os.makedirs(os.path.dirname(token_file), exist_ok=True)
    with open(token_file, "w", encoding="utf-8") as f:
        f.write(tok)
    try:
        os.chmod(token_file, 0o600)
    except OSError:
        pass
    return tok


class QuoteCache:
    """Latest tick + bid/ask per code, thread-safe."""

    def __init__(self):
        self._lock = threading.Lock()
        self._q = {}

    def _blank(self, code):
        return {
            "symbol": code, "name": "", "price": None, "bid": None,
            "ask": None, "previous": None, "tradeAt": None,
            "dayHigh": None, "dayLow": None, "sourceAt": None,
            "_pctChg": None,
        }

    def on_tick(self, tick):
        code = tick.code
        ts = int(tick.datetime.timestamp() * 1000)
        with self._lock:
            q = self._q.setdefault(code, self._blank(code))
            price = float(tick.close)
            q["price"] = price
            q["tradeAt"] = ts
            q["sourceAt"] = ts
            q["_pctChg"] = float(tick.pct_chg)
            q["dayHigh"] = price if q["dayHigh"] is None else max(q["dayHigh"], price)
            q["dayLow"] = price if q["dayLow"] is None else min(q["dayLow"], price)
            pct = q["_pctChg"]
            if pct is not None and pct > -100:
                q["previous"] = round(price / (1 + pct / 100.0), 4)

    def on_bidask(self, bidask):
        code = bidask.code
        ts = int(bidask.datetime.timestamp() * 1000)
        with self._lock:
            q = self._q.setdefault(code, self._blank(code))
            bids = [float(x) for x in (bidask.bid_price or []) if x and float(x) > 0]
            asks = [float(x) for x in (bidask.ask_price or []) if x and float(x) > 0]
            q["bid"] = bids[0] if bids else None
            q["ask"] = asks[0] if asks else None
            if q["sourceAt"] is None or ts > q["sourceAt"]:
                q["sourceAt"] = ts

    def set_name(self, code, name):
        with self._lock:
            q = self._q.setdefault(code, self._blank(code))
            q["name"] = name

    def snapshot(self, codes):
        out = []
        with self._lock:
            for code in codes:
                q = self._q.get(code)
                if q is None or q["sourceAt"] is None:
                    continue
                price, prev = q["price"], q["previous"]
                bid, ask = q["bid"], q["ask"]
                mid = None
                if bid is not None and ask is not None and bid <= ask:
                    mid = (bid + ask) / 2.0
                out.append({
                    "symbol": q["symbol"],
                    "name": q["name"],
                    "price": price,
                    "bid": bid,
                    "ask": ask,
                    "previous": prev,
                    "changePercent": (
                        (price / prev - 1) * 100 if price is not None and prev else None
                    ),
                    "mid": mid,
                    "midChangePercent": (
                        (mid / prev - 1) * 100 if mid is not None and prev else None
                    ),
                    "tradeAt": q["tradeAt"],
                    "dayHigh": q["dayHigh"],
                    "dayLow": q["dayLow"],
                    "sourceAt": q["sourceAt"],
                    "receivedAt": now_ms(),
                    "date": datetime.now().strftime("%Y%m%d"),
                    "source": "Shioaji",
                })
        return out


class ShioajiFeed:
    """Real feed: login + streaming subscriptions, read-only."""

    def __init__(self, cache: QuoteCache, simulation: bool):
        import shioaji as sj  # imported lazily so --mock needs no install

        self.cache = cache
        self.api = sj.Shioaji(simulation=simulation)
        api_key = os.environ.get("SHIOAJI_API_KEY", "")
        secret_key = os.environ.get("SHIOAJI_SECRET_KEY", "")
        if not api_key or not secret_key:
            raise SystemExit(
                "SHIOAJI_API_KEY / SHIOAJI_SECRET_KEY are not set. "
                "Create a key at the SinoPac API management page and export both."
            )
        self.api.login(api_key=api_key, secret_key=secret_key)
        self._subscribed = set()
        self._lock = threading.Lock()

        @self.api.on_tick_stk_v1()
        def _tick(exchange, tick):
            self.cache.on_tick(tick)

        @self.api.on_bidask_stk_v1()
        def _bidask(exchange, bidask):
            self.cache.on_bidask(bidask)

    def ensure(self, codes):
        from shioaji.constant import QuoteType, QuoteVersion

        with self._lock:
            todo = [c for c in codes if c not in self._subscribed]
            for code in todo:
                contract = None
                for market in (self.api.Contracts.Stocks.TSE,
                               self.api.Contracts.Stocks.OTC):
                    try:
                        contract = market[code]
                    except Exception:
                        contract = None
                    if contract is not None:
                        break
                if contract is None:
                    continue
                self.cache.set_name(code, getattr(contract, "name", "") or "")
                self.api.quote.subscribe(
                    contract, quote_type=QuoteType.Tick,
                    version=QuoteVersion.v1)
                self.api.quote.subscribe(
                    contract, quote_type=QuoteType.BidAsk,
                    version=QuoteVersion.v1)
                self._subscribed.add(code)


MOCK_BASE = {"2330": 2340.0, "2317": 182.5, "0050": 100.95}


class MockFeed:
    """Deterministic synthetic ticks so the chain can be tested offline."""

    def __init__(self, cache: QuoteCache):
        self.cache = cache
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True)

    def start(self):
        self._thread.start()

    def _run(self):
        class T:
            pass

        step = 0
        while not self._stop.is_set():
            step += 1
            for code, base in MOCK_BASE.items():
                price = base + ((step * 7) % 11) - 5
                t = T()
                t.code = code
                t.datetime = datetime.now()
                t.close = price
                t.pct_chg = (price / (base * 0.99) - 1) * 100
                self.cache.on_tick(t)
                b = T()
                b.code = code
                b.datetime = datetime.now()
                b.bid_price = [price - 0.5]
                b.ask_price = [price + 0.5]
                self.cache.on_bidask(b)
                self.cache.set_name(code, {"2330": "台積電", "2317": "鴻海",
                                           "0050": "元大台灣50"}.get(code, ""))
            self._stop.wait(2.0)


def make_handler(cache, feed, token):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):  # keep stdout clean
            pass

        def _json(self, obj, status=200):
            body = json.dumps(obj, ensure_ascii=False).encode("utf-8")
            self.send_response(status)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            if self.headers.get("X-Sidecar-Token") != token:
                self._json({"error": "unauthorized"}, 401)
                return
            parsed = urlparse(self.path)
            if parsed.path == "/health":
                self._json({"ok": True, "source": "Shioaji sidecar",
                            "at": now_ms()})
                return
            if parsed.path == "/quotes":
                codes = [c for c in parse_qs(parsed.query).get(
                    "codes", [""])[0].split(",") if c]
                if feed is not None and hasattr(feed, "ensure"):
                    feed.ensure(codes)
                self._json({"quotes": cache.snapshot(codes)})
                return
            self._json({"error": "not found"}, 404)

    return Handler


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=18765)
    ap.add_argument("--mock", action="store_true")
    ap.add_argument("--simulation", action="store_true",
                    help="use the Shioaji simulation environment")
    args = ap.parse_args()

    token = load_or_create_token()
    cache = QuoteCache()
    feed = None
    if args.mock:
        feed = MockFeed(cache)
        feed.start()
        mode = "mock"
    else:
        feed = ShioajiFeed(cache, simulation=args.simulation)
        mode = "simulation" if args.simulation else "production"
    server = ThreadingHTTPServer(
        ("127.0.0.1", args.port), make_handler(cache, feed, token))
    print(f"shioaji sidecar listening on 127.0.0.1:{args.port} ({mode})",
          flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
