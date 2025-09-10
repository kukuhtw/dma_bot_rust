# dma_bot_rust

DMA (Direct Market Access)

A fast, async **crypto trading engine** written in Rust. It streams **multi-symbol** market data (mock or Binance), runs **pluggable strategies** (mean-reversion, MA crossover, volatility breakout), enforces **risk limits**, **routes** orders across venues (mock/Binance), tracks **positions/PnL**, and exposes rich **Prometheus metrics**. It can also **record every event** as JSONL for auditing/backtesting.

---
![Alt text](https://github.com/kukuhtw/dma_bot_rust/blob/main/2.png?raw=true)
---

## Table of Contents

* [Features](#features)
* [Architecture](#architecture)
* [Requirements](#requirements)
* [Environment & Presets](#environment--presets)
* [Quick Start](#quick-start)
* [Configuration Examples](#configuration-examples)
* [What You’ll See](#what-youll-see)
* [Prometheus & Grafana Setup](#prometheus--grafana-setup)
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

```bash
sudo apt install pkg-config libssl-dev
```

Or use `reqwest` with **rustls**:

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
```

---

## Environment & Presets

App loads `.env` at startup via `dotenvy`.

Presets:

* `.env.mock` → simulation
* `.env.sandbox` → Binance testnet
* `.env.mainnet` → Binance mainnet

Switch preset:

```bash
cp .env.mock .env
cargo run
```

or:

```bash
ln -sf .env.sandbox .env
cargo run
```

---

## Quick Start

```bash
cargo build
cp .env.mock .env
cargo run
```

You should see:

```
metrics listening on http://0.0.0.0:9898/ (and /metrics)
```

Check metrics:

```bash
curl -s localhost:9898/metrics | head -n 20
```

---

## Configuration Examples

### `.env.mock`

```env
FEED_MODE=mock
VENUE_MODE=mock
SYMBOLS=BTCUSDT,ETHUSDT,SOLUSDT,BNBUSDT
SYMBOL=BTCUSDT
STRATEGIES=mean_reversion,ma_crossover,vol_breakout
STRATEGY_WORKERS=2
METRICS_PORT=9898
RUST_LOG=info
RECORD_FILE=events_mock.jsonl
```

### `.env.sandbox`

```env
FEED_MODE=binance_sandbox
VENUE_MODE=binance_sandbox
BINANCE_WS_URL=wss://testnet.binance.vision/ws
BINANCE_REST_URL=https://testnet.binance.vision
BINANCE_API_KEY=your_testnet_key
BINANCE_API_SECRET=your_testnet_secret
SYMBOLS=BTCUSDT,ETHUSDT
SYMBOL=BTCUSDT
STRATEGIES=mean_reversion
STRATEGY_WORKERS=2
METRICS_PORT=9898
RUST_LOG=info
```

### `.env.mainnet`

⚠️ **Risk: live trading**

```env
FEED_MODE=binance_mainnet
VENUE_MODE=binance_mainnet
BINANCE_WS_URL=wss://stream.binance.com:9443/ws
BINANCE_REST_URL=https://api.binance.com
BINANCE_API_KEY=your_mainnet_key
BINANCE_API_SECRET=your_mainnet_secret
SYMBOLS=BTCUSDT,ETHUSDT
SYMBOL=BTCUSDT
STRATEGIES=ma_crossover
STRATEGY_WORKERS=1
METRICS_PORT=9898
RUST_LOG=info
```

---

## What You’ll See

* **Metrics**:
  `exec_reports_total`, `inventory_total_qty`, `latency_signal_to_ack_ms_bucket`
* **Events JSONL**:
  written to `events_*.jsonl`
* **Prometheus scraping**:
  `http://localhost:9090/targets` shows job `dma_bot_rust` UP

---

## Prometheus & Grafana Setup

### Install Prometheus

```bash
sudo apt install prometheus promtool
```

Config `/etc/prometheus/prometheus.yml`:

```yaml
global:
  scrape_interval: 15s

scrape_configs:
  - job_name: 'prometheus'
    static_configs:
      - targets: ['localhost:9090']

  - job_name: 'dma_bot_rust'
    scrape_interval: 5s
    static_configs:
      - targets: ['localhost:9898']
```

Check & restart:

```bash
sudo promtool check config /etc/prometheus/prometheus.yml
sudo systemctl restart prometheus
```

---

### Install Grafana

```bash
sudo apt install grafana
sudo systemctl enable grafana-server
sudo systemctl start grafana-server
```

Open: [http://localhost:3000](http://localhost:3000)
Login: `admin / admin` (change password)

---

### Connect Grafana to Prometheus

* **Connections → Data sources → Add data source**
* Choose **Prometheus**
* URL: `http://localhost:9090`
* Save & Test → must be green

---

### Import Dashboard

1. Grafana → **Dashboards → Import**
2. Paste provided JSON (see `grafana_dma_bot_dashboard.json`)
3. Select datasource = Prometheus
4. Import

You’ll get panels for:

* Exec reports rate (`ack`, `filled`)
* Latency p50/p90/p95/p99
* Inventory per venue & total
* Active strategies / symbols
* Health & scrape duration

---

## Prometheus/Grafana Cheats

* Fill rate per venue

```promql
rate(exec_reports_total{status="filled"}[1m]) by (venue)
```

* Inventory snapshot

```promql
inventory_qty
```

* Latency quantile

```promql
histogram_quantile(0.95, sum(rate(latency_signal_to_ack_ms_bucket[5m])) by (le))
```

---

## Strategies

* Mean Reversion → range trading
* MA Crossover → trend following
* Volatility Breakout → momentum

---

## Recording (JSONL)

Enable recorder:

```env
RECORD_FILE=events.jsonl
```

Each line = `Event` (Md, Sig, Ord, Exec).

---

## Troubleshooting

* **No data in Grafana** → check data source URL = `http://localhost:9090` (not `:9898/metrics`).
* **Config error in Prometheus** → run `promtool check config`.
* **Latency histogram empty** → instrumentation may not emit samples yet.

---

## Project Layout

* `src/main.rs` — task wiring
* `src/feed.rs` — mock & Binance feed
* `src/strategy.rs` — strategies
* `src/risk.rs` — limits
* `src/router.rs` — order routing
* `src/gateway.rs` — mock gateway
* `src/gateway_binance.rs` — Binance REST + WS
* `src/positions.rs` — PnL tracker
* `src/metrics.rs` — Prometheus exporter
* `src/recorder.rs` — JSONL recorder

---

## License

MIT (or your choice).

---

## Author

**Kukuh Tripamungkas Wicaksono (Kukuh TW)**
📧 Email: [kukuhtw@gmail.com](mailto:kukuhtw@gmail.com)
📱 WhatsApp: [wa.me/628129893706](https://wa.me/628129893706)
🔗 LinkedIn: [id.linkedin.com/in/kukuhtw](https://id.linkedin.com/in/kukuhtw)

---

## Disclaimer

This code is for **research & testing**. Markets are risky. If you connect to **mainnet**, you accept full responsibility for all orders and outcomes. Start small, use strict limits, and monitor closely.

