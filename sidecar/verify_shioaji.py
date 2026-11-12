#!/usr/bin/env python3
"""One-shot Shioaji verification: login -> contracts -> snapshots -> tick sample.

Read-only: uses the API key login only; never activates a CA certificate
and never calls any order API, so it cannot place trades.

Usage:
  SHIOAJI_API_KEY=... SHIOAJI_SECRET_KEY=... \
      python3 verify_shioaji.py [--simulation] [code ...]

Default codes: 2330 0050. Prints the snapshot (last close etc.) for each
code; snapshots work outside trading hours. During trading hours it also
subscribes to ticks for a few seconds and prints what arrives.
"""

import os
import sys
import time

import shioaji as sj
from shioaji.constant import QuoteType, QuoteVersion


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    simulation = "--simulation" in sys.argv
    codes = args or ["2330", "0050"]

    api_key = os.environ.get("SHIOAJI_API_KEY", "")
    secret_key = os.environ.get("SHIOAJI_SECRET_KEY", "")
    if not api_key or not secret_key:
        print("ERROR: SHIOAJI_API_KEY / SHIOAJI_SECRET_KEY are not set")
        return 2

    api = sj.Shioaji(simulation=simulation)
    accounts = api.login(api_key=api_key, secret_key=secret_key)
    print(f"login ok ({'simulation' if simulation else 'production'}), "
          f"accounts: {[a.account_type for a in accounts]}")

    contracts = []
    for code in codes:
        contract = None
        for market in (api.Contracts.Stocks.TSE, api.Contracts.Stocks.OTC):
            try:
                contract = market[code]
            except Exception:
                contract = None
            if contract is not None:
                break
        if contract is None:
            print(f"{code}: contract not found")
            continue
        contracts.append(contract)
        print(f"{code}: contract ok - {getattr(contract, 'name', '')} "
              f"({getattr(contract, 'exchange', '')})")

    if contracts:
        snaps = api.snapshots(contracts)
        for s in snaps:
            print(f"snapshot {s.code}: close={s.close} open={s.open} "
                  f"high={s.high} low={s.low} volume={s.total_volume}")

    ticks = []

    @api.on_tick_stk_v1()
    def _tick(exchange, tick):
        ticks.append(tick)

    for contract in contracts:
        api.quote.subscribe(contract, quote_type=QuoteType.Tick,
                            version=QuoteVersion.v1)
    time.sleep(6)
    for contract in contracts:
        api.quote.unsubscribe(contract, quote_type=QuoteType.Tick,
                              version=QuoteVersion.v1)
    if ticks:
        last = ticks[-1]
        print(f"tick sample: {len(ticks)} ticks, last {last.code} "
              f"close={last.close} at {last.datetime}")
    else:
        print("tick sample: no ticks in 6s (expected outside trading hours)")

    api.logout()
    print("logout ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
