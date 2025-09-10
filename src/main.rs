// ===============================
// src/main.rs
// ===============================
/*
 cd /home/kukuhtw/rust/dma_bot_rust

 # konfigurasi yang aktif
curl -s localhost:9898/metrics | egrep '^config_(feed_mode|venue_mode|symbol|strategy_active)'

# aktivitas per symbol & strategi (harus ada kalau snippet increment sudah dipasang)
curl -s localhost:9898/metrics | grep '^ticks_total_by_symbol'
curl -s localhost:9898/metrics | grep '^signals_total_by'

*/
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
mod domain;
mod config;
mod metrics;
mod recorder;
mod feed;
mod strategy;
mod risk;
mod router;
mod gateway;          // mock gateway (ACK -> Filled after delay)
mod posttrade;
mod positions;
mod binance;          // helper (signer/types) for Binance
mod gateway_binance;  // real Binance Spot (REST + User Data Stream)

use ahash::AHashMap as HashMap;
use tokio::{
    select,
    sync::{broadcast, mpsc, watch},
    time::Duration,
};
use tracing::info;

use crate::domain::{Event, InvSnapshot, VenueOrder};

#[tokio::main]
async fn main() {
    // ---- Logging ----
    tracing_subscriber::fmt().with_env_filter("info").init();

    // ---- Load config & limits ----
    let (args, limits) = config::load();

    // ---- Metrics ----
    metrics::init();
    tokio::spawn(metrics::serve_metrics(args.metrics_port));

    // ---- Human-friendly startup info + export config to metrics ----
    let feed_mode_str = match args.feed_mode {
        config::MarketMode::Mock => "mock",
        config::MarketMode::BinanceSandbox => "binance_sandbox",
        config::MarketMode::BinanceMainnet => "binance_mainnet",
    };
    let venue_mode_str = match args.venue_mode {
        config::MarketMode::Mock => "mock",
        config::MarketMode::BinanceSandbox => "binance_sandbox",
        config::MarketMode::BinanceMainnet => "binance_mainnet",
    };
    let strategy_names: Vec<&'static str> = args
        .strategy_modes
        .iter()
        .map(|m| match m {
            config::StrategyMode::MeanReversion => "mean_reversion",
            config::StrategyMode::MACrossover => "ma_crossover",
            config::StrategyMode::VolBreakout => "vol_breakout",
        })
        .collect();

    info!(
        feed_mode = %feed_mode_str,
        venue_mode = %venue_mode_str,
        symbols = ?args.symbols,
        strategies = ?strategy_names,
        workers_per_strategy = args.strategy_workers,
        binance_ws = %args.binance_ws_url,
        binance_rest = %args.binance_rest_url,
        "startup config"
    );

    crate::metrics::CONFIG_FEED_MODE
        .with_label_values(&[feed_mode_str])
        .set(1);
    crate::metrics::CONFIG_VENUE_MODE
        .with_label_values(&[venue_mode_str])
        .set(1);
    for s in &args.symbols {
        crate::metrics::CONFIG_SYMBOL.with_label_values(&[s]).set(1);
    }
    for m in &args.strategy_modes {
        let label = match m {
            config::StrategyMode::MeanReversion => "mean_reversion",
            config::StrategyMode::MACrossover => "ma_crossover",
            config::StrategyMode::VolBreakout => "vol_breakout",
        };
        crate::metrics::CONFIG_STRATEGY_ACTIVE
            .with_label_values(&[label])
            .set(args.strategy_workers as i64);
    }

    // ---- Buses ----
    let (md_tx, _md_rx) = broadcast::channel::<domain::MdTick>(4096);
    let (sig_tx, sig_rx) = mpsc::channel::<domain::Signal>(2048);
    let (ord_tx, ord_rx) = mpsc::channel::<domain::Order>(2048);

    // Fan-out ExecReport: gateway -> central -> (posttrade, positions dispatcher)
    let (exec_central_tx, exec_central_rx) = mpsc::channel::<domain::ExecReport>(4096);
    let (exec_to_post_tx, exec_to_post_rx) = mpsc::channel::<domain::ExecReport>(4096);
    let (exec_to_pos_tx, exec_to_pos_rx) = mpsc::channel::<domain::ExecReport>(4096);
    tokio::spawn(async move {
        let mut rx = exec_central_rx;
        while let Some(er) = rx.recv().await {
            let _ = exec_to_post_tx.send(er.clone()).await;
            let _ = exec_to_pos_tx.send(er).await;
        }
    });

    // ---- Recorder (optional) ----
    let (rec_tx, rec_rx) = mpsc::channel::<Event>(8192);
    if let Some(path) = args.record_file.clone() {
        tokio::spawn(recorder::run(rec_rx, path));
    }

    // ---- FEED (Market Data) ----
    // Multi-symbol feed: args.symbols (fallback ke args.symbol jika SYMBOLS kosong)
    match args.feed_mode {
        config::MarketMode::Mock => {
            for sym in args.symbols.iter().cloned() {
                let tx = md_tx.clone();
                tokio::spawn(async move {
                    feed::run_mock(tx, sym).await;
                });
            }
        }
        config::MarketMode::BinanceSandbox | config::MarketMode::BinanceMainnet => {
            for sym in args.symbols.iter().cloned() {
                let tx = md_tx.clone();
                let base = args.binance_ws_url.clone();
                tokio::spawn(async move {
                    feed::run_binance(tx, sym, base).await;
                });
            }
        }
    };

    // ---- Strategy workers ----
    // Pilih via ENV:
    //   STRATEGY=mean_reversion|ma_crossover|vol_breakout  (single)
    //   atau STRATEGIES=mean_reversion,ma_crossover        (multi)
    //   STRATEGY_WORKERS=N                                 (default 2)
    for mode in &args.strategy_modes {
        for _ in 0..args.strategy_workers {
            let rx = md_tx.subscribe();
            let sig = sig_tx.clone();
            match mode {
                config::StrategyMode::MeanReversion => {
                    tokio::spawn(strategy::run(rx, sig));
                }
                config::StrategyMode::MACrossover => {
                    tokio::spawn(strategy::run_ma_crossover(rx, sig));
                }
                config::StrategyMode::VolBreakout => {
                    tokio::spawn(strategy::run_vol_breakout(rx, sig));
                }
            }
        }
    }

    // ---- Risk ----
    tokio::spawn(risk::run(sig_rx, ord_tx.clone(), limits));

    // ---- SOR Multi-Venue ----
    let cfg = router::RouterCfg::default();

    // Salin parameter venue agar 'static
    let venue_params: Vec<(String, u32)> = cfg
        .venues
        .iter()
        .map(|(name, vcfg)| (name.clone(), vcfg.est_latency_ms))
        .collect();

    // Buat gateway per-venue
    let mut gw_txs: HashMap<String, mpsc::Sender<VenueOrder>> = HashMap::new();
    for (venue_name, est_latency_ms) in venue_params {
        let (tx, rx) = mpsc::channel::<VenueOrder>(1024);
        gw_txs.insert(venue_name.clone(), tx);
        let exec_tx = exec_central_tx.clone();

        let venue_mode = args.venue_mode.clone();
        let rest_base = args.binance_rest_url.clone();

        tokio::spawn({
            let venue_name_spawn = venue_name.clone();
            async move {
                match venue_mode {
                    // Semua venue mock
                    config::MarketMode::Mock => {
                        crate::gateway::run_venue(
                            rx,
                            exec_tx,
                            venue_name_spawn,
                            est_latency_ms as u64,
                        )
                        .await;
                    }
                    // Sandbox/Mainnet: venue "binance"/"binance_testnet" pakai gateway_binance, lainnya mock
                    config::MarketMode::BinanceSandbox | config::MarketMode::BinanceMainnet => {
                        match venue_name_spawn.to_ascii_lowercase().as_str() {
                            "binance" | "binance_testnet" => {
                                // pass REST base ke gateway_binance via ENV (dipakai internal)
                                std::env::set_var("BINANCE_REST_URL", rest_base.clone());
                                crate::gateway_binance::run_venue_binance(
                                    rx,
                                    exec_tx,
                                    venue_name_spawn,
                                )
                                .await;
                            }
                            _ => {
                                crate::gateway::run_venue(
                                    rx,
                                    exec_tx,
                                    venue_name_spawn,
                                    est_latency_ms as u64,
                                )
                                .await;
                            }
                        }
                    }
                }
            }
        });
    }

    // ---- Positions / PnL watcher (multi-symbol dengan dispatcher) ----
    // Snapshot utama untuk symbol "primary" (dipakai router)
    let (snap_tx_primary, snap_rx) = watch::channel::<InvSnapshot>(InvSnapshot {
        ts_ns: 0,
        symbol: args.symbol.clone(),
        state: Default::default(),
    });

    // Channel positions per symbol
    let mut pos_txs: HashMap<String, mpsc::Sender<crate::domain::ExecReport>> = HashMap::new();

    for sym in args.symbols.iter().cloned() {
        let (pos_tx, pos_rx) = mpsc::channel::<crate::domain::ExecReport>(2048);
        pos_txs.insert(sym.clone(), pos_tx);

        let md_rx_pos = md_tx.subscribe();
        if sym == args.symbol {
            // symbol utama -> gunakan snap_tx_primary (agar router tetap dapat snapshot)
            let snap_tx = snap_tx_primary.clone();
            tokio::spawn(positions::run(sym.clone(), md_rx_pos, pos_rx, snap_tx));
        } else {
            // symbol lain -> snapshot sendiri (tidak dipakai router saat ini)
            let (snap_tx_other, _snap_rx_unused) = watch::channel::<InvSnapshot>(InvSnapshot {
                ts_ns: 0,
                symbol: sym.clone(),
                state: Default::default(),
            });
            tokio::spawn(positions::run(sym.clone(), md_rx_pos, pos_rx, snap_tx_other));
        }
    }

    // Dispatcher: fanout ExecReport ke positions per symbol
    tokio::spawn({
        let mut pos_map = pos_txs;
        let mut rx = exec_to_pos_rx;
        async move {
            while let Some(er) = rx.recv().await {
                if let Some(tx) = pos_map.get(&er.symbol) {
                    let _ = tx.send(er).await;
                } else {
                    // Tak ada channel untuk symbol tsb (belum dikonfigurasi)
                    tracing::debug!(symbol = %er.symbol, "no positions channel for symbol");
                }
            }
        }
    });

    // ---- Router ----
    tokio::spawn(router::run(ord_rx, gw_txs, cfg, snap_rx));

    // ---- Post-Trade ----
    tokio::spawn(posttrade::run(exec_to_post_rx));

    // ---- Heartbeat + record MD ----
    let mut md_rx_metrics = md_tx.subscribe();
    let rec_tx2 = rec_tx.clone();
    let mut tick_count: u64 = 0;

    loop {
        select! {
            Ok(md) = md_rx_metrics.recv() => {
                tick_count += 1;
                let _ = rec_tx2.try_send(Event::Md(md));
            },
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                info!(ticks=tick_count, "heartbeat");
                tick_count = 0;
            }
        }
    }
}
