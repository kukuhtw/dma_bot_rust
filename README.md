
# dma\_bot\_rust

A fast, async **crypto trading engine** written in Rust. It streams **multi-symbol** market data (mock or Binance), runs **pluggable strategies** (mean-reversion, MA crossover, volatility breakout), enforces **risk limits**, **routes** orders across venues (mock/Binance), tracks **positions/PnL**, and exposes rich **Prometheus metrics**. It can also **record every event** as JSONL for auditing/backtesting.

---

## Table of Contents

* [Features](#features)
* [Architecture](#architecture)
* [Requirements](#requirements)
* [Environment & Presets](#environment--presets)
* [Quick Start](#quick-start)
* [Configuration Examples](#configuration-examples)
* [What You’ll See](#what-youll-see)
* [Prometheus/Grafana Cheats](#prometheusgrafana-cheats)
* [Strategies](#strategies)
* [Recording (JSONL)](#recording-jsonl)
* [Troubleshooting](#troubleshooting)
* [Project Layout](#project-layout)
* [License](#license)
* [Disclaimer](#disclaimer)

---

## Features

* **Sources**

  * Mock tick generator (high-rate random walk)
  * Binance Spot: **Testnet** (sandbox) or **Mainnet** (bookTicker WS + REST trading + User Data Stream)
* **Strategies** (mix & match, N workers each)

  * Mean Reversion
  * Moving Average Crossover
  * Volatility Breakout
* **Risk**: price bands, notional cap, QPS throttle
* **SOR/Router**: multi-venue scoring & fan-out
* **Gateways**

  * Mock (ACK → Filled after latency)
  * Binance (REST + userDataStream WS)
* **Positions/PnL**: per-venue inventory, realized & unrealized PnL
* **Observability**

  * Prometheus metrics on `:9898` (built-in HTTP server)
  * Config visibility (feed/venue modes, strategies, symbols)
  * WS health (connected, reconnects, last event age)
* **Recorder**: append-only JSONL (`Event::Md/Sig/Ord/Exec`) for audit

---

## Architecture

```
        feed.rs ──> MdTick ──┐
                             ├─> strategy.rs ──> Signal ──> risk.rs ──> Order
                             │
                             └─> positions.rs (mark-to-market)
                                                     │
router.rs <─ Order ──────────────────────────────────┘
  │
  ├─> gateway (mock / binance) ──> ExecReport ──┐
  │                                             ├─> posttrade.rs
  └──────────────────────────────────────────────┘    positions.rs (fills)
```

All buses are Tokio channels; components run as async tasks.

---

## Requirements

* Rust **1.75+** (stable)
* Linux/macOS (Windows should work, untested)

**OpenSSL build errors?** Either:

* Install system deps: `sudo apt install pkg-config libssl-dev`, **or**
* Use `reqwest` with **rustls** (recommended) in `Cargo.toml`:

  ```toml
  reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
  urlencoding = "2"
  ```

---

## Environment & Presets

The app loads **`.env`** at startup (via `dotenvy`). Three presets are included:

* **`.env.mock`** – safe local **simulation**: mock market data + mock gateway.
* **`.env.sandbox`** – **Binance Testnet**: real WS feed + REST trading on testnet (no real money).
* **`.env.mainnet`** – **Binance Mainnet**: live markets & live orders. **Use with care.**

**How loading works**

* The app reads **`.env`** in the current directory.
* Process env vars override `.env` values, e.g.:

  ```bash
  MAX_QPS=5 RUST_LOG=debug cargo run
  ```

**Switching presets**

```bash
# 1) MOCK (recommended for dev)
cp .env.mock .env
cargo run

# 2) BINANCE SANDBOX (requires keys)
cp .env.sandbox .env
# edit .env to set BINANCE_API_KEY / BINANCE_API_SECRET
cargo run

# 3) BINANCE MAINNET (live trading – careful!)
cp .env.mainnet .env
# set real BINANCE_API_KEY / BINANCE_API_SECRET
# verify MAX_NOTIONAL, PX_MIN/PX_MAX, MAX_QPS
cargo run
```

No-copy alternatives:

```bash
# Source a preset just for this run
set -a; source .env.sandbox; set +a; cargo run

# Or symlink
ln -sf .env.mock .env        # switch to mock
ln -sf .env.sandbox .env     # switch to sandbox
ln -sf .env.mainnet .env     # switch to mainnet
cargo run
```

---

## Quick Start

```bash
cargo build
cp .env.mock .env      # or pick another preset
cargo run
```

You should see:

```
metrics listening on http://0.0.0.0:9898/ (and /metrics)
```

Check metrics:

```bash
curl -s localhost:9898/metrics | head -n 50
```

---

## Configuration Examples

### `.env.mock` (default safe mode)

```env
FEED_MODE=mock
VENUE_MODE=mock

SYMBOLS=BTCUSDT,ETHUSDT,SOLUSDT,BNBUSDT
SYMBOL=BTCUSDT

MAX_NOTIONAL=2000000000
PX_MIN=1000
PX_MAX=200000
MAX_QPS=50

# Single: STRATEGY=ma_crossover
# Multi:
STRATEGIES=mean_reversion,ma_crossover,vol_breakout
STRATEGY_WORKERS=2

METRICS_PORT=9898
RUST_LOG=info
RECORD_FILE=events_mockup.jsonl
```

### `.env.sandbox` (Binance Testnet)

```env
FEED_MODE=binance_sandbox
VENUE_MODE=binance_sandbox

BINANCE_WS_URL=wss://testnet.binance.vision/ws
BINANCE_REST_URL=https://testnet.binance.vision
BINANCE_API_KEY=your_testnet_key
BINANCE_API_SECRET=your_testnet_secret
BINANCE_RECV_WINDOW=5000

SYMBOLS=BTCUSDT,ETHUSDT
SYMBOL=BTCUSDT

STRATEGIES=mean_reversion
STRATEGY_WORKERS=2

METRICS_PORT=9898
RUST_LOG=info
RECORD_FILE=events_testnet.jsonl
```

### `.env.mainnet` (Binance Mainnet — **at your own risk**)

```env
FEED_MODE=binance_mainnet
VENUE_MODE=binance_mainnet

BINANCE_WS_URL=wss://stream.binance.com:9443/ws
BINANCE_REST_URL=https://api.binance.com
BINANCE_API_KEY=your_mainnet_key
BINANCE_API_SECRET=your_mainnet_secret
BINANCE_RECV_WINDOW=5000

SYMBOLS=BTCUSDT,ETHUSDT
SYMBOL=BTCUSDT

STRATEGIES=ma_crossover
STRATEGY_WORKERS=1

METRICS_PORT=9898
RUST_LOG=info
RECORD_FILE=events_mainnet.jsonl
```

> **Safety:** Start with **mock** or **sandbox**. Apply conservative limits (notional, price bands, QPS).

---

## What You’ll See

**Startup log**

```
startup config feed_mode=mock venue_mode=mock symbols=["BTCUSDT","ETHUSDT",...]
strategies=["mean_reversion","ma_crossover","vol_breakout"] workers_per_strategy=2 ...
recorder enabled path=events_mockup.jsonl
```

**Metrics endpoint**

```bash
curl -s localhost:9898/metrics | egrep '^config_|^ticks_total|^exec_reports_total'
```

**Event recorder**

```bash
ls -l events_mockup.jsonl
tail -n 5 events_mockup.jsonl
```

---

## Prometheus/Grafana Cheats

* Feed coverage per symbol

  ```promql
  rate(ticks_total_by_symbol[5m])
  ```
* Strategy activity per symbol

  ```promql
  rate(signals_total_by[5m]) by (strategy, symbol)
  ```
* Venue fill-rate

  ```promql
  (rate(exec_reports_total{status="filled"}[10m])
   / ignoring(status) group_left()
   rate(exec_reports_total{status="ack"}[10m])) by (venue)
  ```
* WS health (Binance)

  * `binance_ws_connected{venue="binance"}`
  * `binance_ws_last_event_age_seconds{venue="binance"}`
  * `binance_ws_reconnects_total{venue="binance"}`

---

## Strategies

* **Mean Reversion**
  Buy when ask ≪ rolling mean − edge; sell when bid ≫ mean + edge. Good in ranging markets.
* **MA Crossover**
  Signals when fast SMA crosses slow SMA (with min edge & cooldown). Trend-following.
* **Volatility Breakout**
  Breaks out of rolling high/low ± edge with cooldown. Momentum-oriented.

Enable any subset via `STRATEGIES` and scale with `STRATEGY_WORKERS`.

---

## Recording (JSONL)

Set `RECORD_FILE=/path/events.jsonl` to enable.
Each line is one `Event` (`Md`, `Sig`, `Ord`, `Exec`) — great for audits, offline sims, and debugging fills vs signals.

---

## Troubleshooting

* **No `events_*.jsonl` file**

  * Ensure `.env` is loaded (run from repo root).
  * Look for “recorder enabled path=…”.
  * Try an absolute path: `RECORD_FILE=/tmp/events.jsonl`.

* **Only one symbol seems active**

  * Positions run per symbol. Verify via:

    ```
    curl -s localhost:9898/metrics | grep '^inventory_qty{symbol='
    ```
  * Ensure `SYMBOLS=BTCUSDT,ETHUSDT,...` has no stray spaces.

* **OpenSSL build error**

  * `sudo apt install pkg-config libssl-dev`, or use `reqwest` with `rustls-tls`.

* **Latency histogram empty**

  * Add instrumentation mapping signal/order timestamps to ACK, then `LAT_SIG_ACK.observe(delta_ms)`.

---

## Project Layout

* `src/main.rs` — task wiring, buses, spawns
* `src/config.rs` — env parsing (modes, symbols, strategies)
* `src/feed.rs` — mock & Binance bookTicker WS
* `src/strategy.rs` — 3 strategies
* `src/risk.rs` — acceptance, bands, notional, QPS
* `src/router.rs` — scoring & fan-out to venues
* `src/gateway.rs` — mock gateway (ACK → FILLED)
* `src/gateway_binance.rs` — REST trading + userDataStream
* `src/positions.rs` — inventory/PnL tracker
* `src/metrics.rs` — Prometheus registry & HTTP server
* `src/recorder.rs` — JSONL recorder
* `src/domain.rs` — shared types

---

## License

MIT (or your choice). Add a `LICENSE` file if publishing.


---

## Author

**Kukuh Tripamungkas Wicaksono (Kukuh TW)**  
Email: kukuhtw@gmail.com  
WhatsApp: https://wa.me/628129893706  
LinkedIn: https://id.linkedin.com/in/kukuhtw

---
## Disclaimer

This code is for **research & testing**. Markets are risky. If you connect to **mainnet**, you accept full responsibility for all orders and outcomes. Start small, use strict limits, and monitor closely.
