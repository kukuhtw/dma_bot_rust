// ===============================
// src/feed.rs
// ===============================
//
// Market Data adapters:
// - run_mock      : random-walk generator (~200 ticks/s)
// - run_binance   : Binance WS bookTicker (works for Sandbox & Mainnet)
//                    pass the WS base URL from config (no hardcoded ENV)
//
// Notes:
// - Domain price scale: we use 2 decimals (px * 100) for PoC consistency.
//   For production, derive tickSize/stepSize from exchangeInfo and scale properly.
//

use chrono::Utc;
use futures_util::StreamExt; // for .next()
use rand::Rng;
use std::time::Duration;
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tracing::{error, info, warn};
use url::Url;

use crate::domain::MdTick;
use crate::metrics::TICKS;

/// Generator market data mock (random walk) ~200 ticks/s
pub async fn run_mock(md_tx: tokio::sync::broadcast::Sender<MdTick>, symbol: String) {
    let mut px_bid: i64 = 100_00; // 100.00 (2 desimal)
    loop {
        // jangan simpan ThreadRng melewati .await
        let step = rand::thread_rng().gen_range(-3..=3);
        px_bid = (px_bid + step).max(50_00);
        let tick = MdTick {
            ts_ns: Utc::now().timestamp_nanos_opt().unwrap_or(0) as i128,
            symbol: symbol.clone(),
            best_bid: px_bid,
            best_ask: px_bid + 1,
        };
        let _ = md_tx.send(tick);
        TICKS.inc();
        sleep(Duration::from_millis(5)).await; // ~200 ticks/s
    }
}

/// Adapter ke Binance WS (read-only) untuk best bid/ask (`bookTicker`)
///
/// - `ws_base` diteruskan dari config:
///     * Sandbox: wss://testnet.binance.vision/ws
///     * Mainnet: wss://stream.binance.com:9443/ws
/// - `symbol` adalah domain symbol (mis. "BTCUSDT") — kita lower-case saat susun topic.
/// - Skala harga: 2 desimal (PoC). Untuk produksi, gunakan tickSize dari `exchangeInfo`.
pub async fn run_binance(
    md_tx: tokio::sync::broadcast::Sender<MdTick>,
    symbol: String,
    ws_base: String,
) {
    let topic = format!("{}@bookTicker", symbol.to_lowercase());
    let ws_url = format!("{}/{}", ws_base.trim_end_matches('/'), topic);

    let mut attempt: u32 = 0;
    loop {
        let url = match Url::parse(&ws_url) {
            Ok(u) => u,
            Err(e) => {
                error!(?e, %ws_url, "bad ws url");
                return;
            }
        };

        info!(%ws_url, "connecting binance bookTicker");
        match connect_async(url).await {
            Ok((mut ws, _resp)) => {
                info!("connected to bookTicker for {}", symbol);
                attempt = 0; // reset backoff

                while let Some(frame) = ws.next().await {
                    match frame {
                        Ok(m) if m.is_text() => {
                            // Contoh payload:
                            // {"u":400900217,"s":"BNBUSDT","b":"25.35190000","B":"31.21000000","a":"25.36520000","A":"40.66000000"}
                            let txt = match m.into_text() {
                                Ok(t) => t,
                                Err(e) => {
                                    warn!(?e, "failed to read text frame");
                                    continue;
                                }
                            };
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
                                let b = v.get("b").and_then(|x| x.as_str());
                                let a = v.get("a").and_then(|x| x.as_str());
                                if let (Some(b), Some(a)) = (b, a) {
                                    // NOTE: PoC scale 2 decimals
                                    let bid = (b.parse::<f64>().unwrap_or(0.0) * 100.0).round() as i64;
                                    let ask = (a.parse::<f64>().unwrap_or(0.0) * 100.0).round() as i64;
                                    if bid > 0 && ask > 0 {
                                        let tick = MdTick {
                                            ts_ns: Utc::now().timestamp_nanos_opt().unwrap_or(0) as i128,
                                            symbol: symbol.clone(),
                                            best_bid: bid,
                                            best_ask: ask,
                                        };
                                        let _ = md_tx.send(tick);
                                        TICKS.inc();
                                    }
                                }
                            }
                        }
                        Ok(_) => {
                            // ignore non-text frames
                        }
                        Err(e) => {
                            error!(?e, "ws read error");
                            break;
                        }
                    }
                }
                info!("bookTicker disconnected, will reconnect…");
            }
            Err(e) => {
                error!(?e, "connect failed");
            }
        }

        // Exponential backoff + jitter
        attempt = attempt.saturating_add(1);
        let shift = attempt.min(6) as u32;           // 0..=6
        let factor = 1u64 << shift;                  // 1,2,4,...,64
        let base_ms = 500u64.saturating_mul(factor); // 0.5s..32s
        let jitter = rand::thread_rng().gen_range(0..=250);
        sleep(Duration::from_millis(base_ms + jitter)).await;
    }
}
