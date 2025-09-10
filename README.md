# dma\_bot\_rust

A fast, async **crypto trading engine** written in Rust. It streams multi-symbol market data (mock or Binance), runs **pluggable strategies** (mean-reversion, MA crossover, volatility breakout), enforces **risk limits**, **routes** orders across venues (mock or Binance), tracks **positions/PnL**, and exports rich **Prometheus metrics**. It can also **record every event** as JSONL for audit & backtesting.

---

## Features

* **Sources**

  * Mock tick generator (high-rate random walk)
  * Binance Spot: **Testnet** (sandbox) or **Mainnet** (bookTicker feed + REST trading + User Data Stream)
* **Strategies** (run any mix, N workers each)

  * Mean Reversion
  * Moving Average Crossover
  * Volatility Breakout
* **Risk**: price bands, notional cap, QPS throttle
* **SOR/Router**: multi-venue scoring & fanout
* **Gateways**

  * Mock (ACK → Filled after latency)
  * Binance (REST + userDataStream WS)
* **Positions/PnL**: per-venue inventory, realized & unrealized PnL
* **Observability**

  * Prometheus metrics on `/:9898` (no deps)
  * Config visibility (feed/venue modes, strategies, symbols)
  * WS health (connected, reconnects, last event age)
* **Recorder**: append-only JSONL (`Event::Md/Sig/Ord/Exec`) for audit

---

## Architecture (high level)

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

All buses are Tokio channels; everything runs as async tasks.

---

## Quick start

### 1) Requirements

* Rust 1.75+ (stable)
* Linux/macOS (Windows should work, not tested)

**If you hit OpenSSL build errors**, either:

* Install system OpenSSL dev package (e.g. `sudo apt install pkg-config libssl-dev`), **or**
* Use `reqwest` with **rustls** (recommended) in `Cargo.toml`:

  ```toml
  reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
  urlencoding = "2"

  ```

Environment presets

This project loads .env at startup (via dotenvy). We ship three ready-made presets:

.env.mock – safe local simulation: mock market data + mock gateway.

.env.sandbox – Binance Testnet: real WS feed + REST trading on testnet (no real money).

.env.mainnet – Binance Mainnet: live markets & live orders. Use with care.

How the loader works

The app automatically reads .env in the current directory.

Process env vars override values in .env. You can do one-off overrides like:

MAX_QPS=5 RUST_LOG=debug cargo run

Quick usage (Linux/macOS)

Pick one preset and make it the active .env:

# 1) MOCK (recommended for dev)
cp .env.mock .env
cargo run

# 2) BINANCE SANDBOX (requires keys)
cp .env.sandbox .env
# edit .env and set BINANCE_API_KEY / BINANCE_API_SECRET
cargo run

# 3) BINANCE MAINNET (live trading – be careful)
cp .env.mainnet .env
# edit .env and set real BINANCE_API_KEY / BINANCE_API_SECRET
# verify limits (MAX_NOTIONAL, PX_MIN/PX_MAX, MAX_QPS) before running
cargo run


If you prefer not to copy files, you can source a preset for a single run:

# Run with sandbox vars without touching .env
set -a; source .env.sandbox; set +a; cargo run


Or symlink:

ln -sf .env.mock .env        # switch to mock
ln -sf .env.sandbox .env     # switch to sandbox
ln -sf .env.mainnet .env     # switch to mainnet
cargo run

What each preset contains

.env.mock

FEED_MODE=mock, VENUE_MODE=mock

Multiple symbols via SYMBOLS=BTCUSDT,ETHUSDT,SOLUSDT,BNBUSDT

All strategies enabled by default (STRATEGIES=…) with STRATEGY_WORKERS=2

Optional RECORD_FILE=events_mockup.jsonl for JSONL audit

Safe for development; no external connections.

.env.sandbox

FEED_MODE=binance_sandbox, VENUE_MODE=binance_sandbox

BINANCE_WS_URL, BINANCE_REST_URL point to testnet

Set BINANCE_API_KEY and BINANCE_API_SECRET

Good for end-to-end testing without real funds.

.env.mainnet

FEED_MODE=binance_mainnet, VENUE_MODE=binance_mainnet

Set real BINANCE_API_KEY and BINANCE_API_SECRET

Double-check limits: MAX_NOTIONAL, PX_MIN/PX_MAX, MAX_QPS

Start with minimal risk (few symbols, 1 strategy, small QPS).

### 2) Build & run (mock mode)

```bash
cargo build
cp .env.mock .env    # or create your own .env (see below)
cargo run
```

You should see a line like:

```
metrics listening on http://0.0.0.0:9898/ (and /metrics)
```

### 3) Check metrics

```bash
curl -s localhost:9898/metrics | head -n 50
```

Look for:

* `config_feed_mode`, `config_venue_mode`
* `config_symbol{symbol="..."}`
* `config_strategy_active{strategy="..."}`
* Activity: `ticks_total`, `exec_reports_total{...}`
* (Optional) per-symbol: `ticks_total_by_symbol`, `signals_total_by`

---

## Configuration (.env)

The app loads `.env` at startup. Three typical presets:

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

# Strategies: single STRATEGY=... or multi STRATEGIES=a,b,c
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

### `.env.mainnet` (Binance Mainnet – **at your own risk**)

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

> **Safety:** Mainnet sends real orders. Start with **mock**/**sandbox**. Set conservative limits (notional, price bands, QPS).

---

## What you’ll see

* **Startup log**

  ```
  startup config feed_mode=mock venue_mode=mock symbols=["BTCUSDT","ETHUSDT",...] strategies=["mean_reversion","ma_crossover","vol_breakout"] workers_per_strategy=2 ...
  recorder enabled path=events_mockup.jsonl
  ```

* **Metrics endpoint**

  ```
  curl -s localhost:9898/metrics | egrep '^config_|^ticks_total|^exec_reports_total'
  ```

* **Event recorder**

  ```
  ls -l events_mockup.jsonl
  tail -n 5 events_mockup.jsonl
  ```

---

## Prometheus / Grafana cheats

* Feed coverage per symbol:

  ```promql
  rate(ticks_total_by_symbol[5m])
  ```

* Strategy activity per symbol:

  ```promql
  rate(signals_total_by[5m]) by (strategy, symbol)
  ```

* Venue fill-rate:

  ```promql
  (rate(exec_reports_total{status="filled"}[10m])
   / ignoring(status) group_left()
   rate(exec_reports_total{status="ack"}[10m])) by (venue)
  ```

* WS health (Binance):

  * `binance_ws_connected{venue="binance"}`
  * `binance_ws_last_event_age_seconds{venue="binance"}`
  * `binance_ws_reconnects_total{venue="binance"}`

---

## Strategies

* **Mean Reversion**
  Buys when ask << rolling mean − edge; sells when bid >> mean + edge. Good for ranging markets.

* **MA Crossover**
  Signals when fast SMA crosses slow SMA (with min edge & cooldown). Trend-following.

* **Volatility Breakout**
  Breaks out of rolling high/low ± edge with cooldown. Momentum focus.

Enable any subset via `STRATEGIES` and scale workers via `STRATEGY_WORKERS`.

---

## Recording (JSONL)

Set `RECORD_FILE=/path/file.jsonl` to enable.
Each line is one `Event` (`Md`, `Sig`, `Ord`, `Exec`).
Useful for:

* Audits (what happened & when)
* Offline sims / quick backtests
* Debugging fills vs. signals

---

## Troubleshooting

* **No `events_*.jsonl` appears**

  * Ensure `.env` is read (the code calls `dotenvy::dotenv()`).
  * Check log: “recorder enabled path=…”
  * Use absolute path: `RECORD_FILE=/tmp/events.jsonl`

* **Only one symbol seems to have positions**

  * Positions run **per symbol**; verify dispatch works via metrics:

    * `inventory_qty{symbol="...",venue="..."}`
  * Make sure `SYMBOLS=…` has **no stray spaces**.

* **OpenSSL build error**

  * `sudo apt install pkg-config libssl-dev`, **or** use `reqwest` with `rustls-tls`.

* **Latency histogram empty**

  * You must map signal/order timestamps to ACK, then `LAT_SIG_ACK.observe(delta_ms)`. (Easy patch; ask if you want a snippet.)

---

## Project layout (key files)

* `src/main.rs` – task wiring, buses, spawns
* `src/config.rs` – env parsing (modes, symbols, strategies)
* `src/feed.rs` – mock & Binance bookTicker WS
* `src/strategy.rs` – 3 strategies
* `src/risk.rs` – acceptance, bands, notional, QPS
* `src/router.rs` – scoring & fanout to venues
* `src/gateway.rs` – mock gateway (ACK → FILLED)
* `src/gateway_binance.rs` – REST trading + userDataStream
* `src/positions.rs` – inventory/PnL tracker
* `src/metrics.rs` – Prometheus registry & HTTP server
* `src/recorder.rs` – JSONL recorder
* `src/domain.rs` – shared types

---

## License

MIT (or your choice). Add a `LICENSE` file if you publish.

---

## Disclaimer

This code is for **research & testing**. Markets are risky. If you connect to **mainnet**, you accept full responsibility for all orders and outcomes. Use limits, start tiny, and observe carefully.
