// ===============================
// src/config.rs
// ===============================
/*
=============================================================================
Project : dma_bot_rust — low-latency async crypto trading engine in Rust
Module  : <module_name>.rs
Version : 0.5.0
Author  : Kukuh Tripamungkas Wicaksono (Kukuh TW)
Email   : kukuhtw@gmail.com
WhatsApp: https://wa.me/628129893706
LinkedIn: https://id.linkedin.com/in/kukuhtw
License : MIT (see LICENSE)

Summary : Streams multi-symbol market data (mock/Binance), runs pluggable
          strategies (mean-reversion, MA crossover, vol breakout), applies
          risk limits, routes orders across venues, tracks positions/PnL,
          exposes Prometheus metrics, and records JSONL events.

(c) 2025 Kukuh TW. All rights reserved where applicable.
=============================================================================
*/
use std::env;
use dotenvy::dotenv;

/// Mode sumber market data / venue trading
#[derive(Clone, Debug)]
pub enum MarketMode {
    Mock,
    BinanceSandbox,
    BinanceMainnet,
}

impl MarketMode {
    pub fn from_env(key: &str, default_mode: MarketMode) -> MarketMode {
        match env::var(key).unwrap_or_default().to_ascii_lowercase().as_str() {
            "mock"             => MarketMode::Mock,
            "binance_sandbox"  => MarketMode::BinanceSandbox,
            "binance_mainnet"  => MarketMode::BinanceMainnet,
            _ => default_mode,
        }
    }

    // Endpoint default per mode
    pub fn default_ws_url(&self) -> &'static str {
        match self {
            MarketMode::Mock            => "wss://testnet.binance.vision/ws", // tidak dipakai saat mock
            MarketMode::BinanceSandbox  => "wss://testnet.binance.vision/ws",
            MarketMode::BinanceMainnet  => "wss://stream.binance.com:9443/ws",
        }
    }

    pub fn default_rest_url(&self) -> &'static str {
        match self {
            MarketMode::Mock            => "https://testnet.binance.vision", // placeholder
            MarketMode::BinanceSandbox  => "https://testnet.binance.vision",
            MarketMode::BinanceMainnet  => "https://api.binance.com",
        }
    }
}

// ===== Strategi =====
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StrategyMode {
    MeanReversion,
    MACrossover,
    VolBreakout,
}

impl StrategyMode {
    pub fn parse_one(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "mean_reversion" | "meanreversion" | "mr" => Some(StrategyMode::MeanReversion),
            "ma_crossover"  | "macrossover"  | "ma"  => Some(StrategyMode::MACrossover),
            "vol_breakout"  | "volbreakout"  | "vb"  => Some(StrategyMode::VolBreakout),
            _ => None,
        }
    }

    /// Baca daftar strategi dari `STRATEGIES` (comma separated) atau fallback `STRATEGY` (single).
    pub fn parse_many(env_key_list: &str, env_key_single: &str, default_list: Vec<Self>) -> Vec<Self> {
        // STRATEGIES=mean_reversion,ma_crossover
        if let Ok(val) = env::var(env_key_list) {
            let mut out: Vec<Self> = val
                .split(',')
                .filter_map(|t| Self::parse_one(t))
                .collect();
            out.dedup();
            if !out.is_empty() {
                return out;
            }
        }
        // Fallback STRATEGY=mean_reversion
        if let Ok(one) = env::var(env_key_single) {
            if let Some(mode) = Self::parse_one(&one) {
                return vec![mode];
            }
        }
        default_list
    }
}

#[derive(Clone, Debug)]
pub struct Args {
    // symbol
    pub data_source: String, // legacy; tidak wajib digunakan
    pub symbol: String,      // primary symbol (untuk snapshot router)
    pub symbols: Vec<String>, // multi-symbol feed/positions

    // files/metrics
    pub record_file: Option<String>,
    pub metrics_port: u16,

    // market mode
    pub feed_mode: MarketMode,
    pub venue_mode: MarketMode,
    pub binance_ws_url: String,
    pub binance_rest_url: String,

    // strategy selection
    pub strategy_modes: Vec<StrategyMode>, // bisa lebih dari satu
    pub strategy_workers: u32,             // worker per strategi
}

#[derive(Clone, Debug)]
pub struct Limits {
    pub max_notional: i64,
    pub px_min: i64,
    pub px_max: i64,
    pub max_qps: u32,
}

pub fn load() -> (Args, Limits) {
    // Pastikan .env dibaca (agar RECORD_FILE, SYMBOLS, dll ter-load)
    let _ = dotenv();

    // ===== Basic =====
    let data_source = env::var("DATA_SOURCE").unwrap_or_else(|_| "mock".to_string());
    let symbol      = env::var("SYMBOL").unwrap_or_else(|_| "BTCUSDT".to_string());

    // Multi-symbol: SYMBOLS=BTCUSDT,ETHUSDT,SOLUSDT
    let symbols: Vec<String> = env::var("SYMBOLS")
        .ok()
        .map(|s| {
            s.split(',')
                .map(|x| x.trim())
                .filter(|x| !x.is_empty())
                .map(|x| x.to_ascii_uppercase())
                .collect()
        })
        .filter(|v: &Vec<String>| !v.is_empty())
        .unwrap_or_else(|| vec![symbol.clone()]);

    let record_file  = env::var("RECORD_FILE").ok();
    let metrics_port = env::var("METRICS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(9898);

    // ===== Mode =====
    let feed_mode  = MarketMode::from_env("FEED_MODE",  MarketMode::Mock);
    let venue_mode = MarketMode::from_env("VENUE_MODE", MarketMode::Mock);

    let binance_ws_url = env::var("BINANCE_WS_URL")
        .unwrap_or_else(|_| feed_mode.default_ws_url().to_string());
    let binance_rest_url = env::var("BINANCE_REST_URL")
        .unwrap_or_else(|_| venue_mode.default_rest_url().to_string());

    // ===== Strategy selection =====
    // Contoh:
    //   STRATEGY=ma_crossover
    //   STRATEGIES=mean_reversion,vol_breakout
    //   STRATEGY_WORKERS=2
    let strategy_modes = StrategyMode::parse_many(
        "STRATEGIES",
        "STRATEGY",
        vec![StrategyMode::MeanReversion], // default
    );
    let strategy_workers = env::var("STRATEGY_WORKERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);

    let args = Args {
        data_source,
        symbol,
        symbols,
        record_file,
        metrics_port,
        feed_mode,
        venue_mode,
        binance_ws_url,
        binance_rest_url,
        strategy_modes,
        strategy_workers,
    };

    // ===== Limits =====
    let max_notional = env::var("MAX_NOTIONAL")
        .ok()
        .and_then(|x| x.parse().ok())
        .unwrap_or(2_000_000_000);
    let px_min  = env::var("PX_MIN").ok().and_then(|x| x.parse().ok()).unwrap_or(1_000);
    let px_max  = env::var("PX_MAX").ok().and_then(|x| x.parse().ok()).unwrap_or(200_000);
    let max_qps = env::var("MAX_QPS").ok().and_then(|x| x.parse().ok()).unwrap_or(50);

    let limits = Limits { max_notional, px_min, px_max, max_qps };
    (args, limits)
}
