# dma_bot_rust
fast, async trading engine in Rust for crypto. It streams multi-symbol market data (mock or Binance), runs pluggable strategies (mean-reversion, MA crossover, volatility breakout), applies risk limits, routes orders across venues, and exposes Prometheus metrics, plus JSONL event recording for audits. Built for low-latency testing.
