// ===============================
// src/risk.rs
// ===============================
use chrono::Utc;
use rand::Rng;
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::warn;

use crate::config::Limits;
use crate::domain::{Order, Signal};
use crate::metrics::ORDERS;

/// State throttle sederhana: batasi QPS berbasis interval waktu
#[derive(Debug, Default)]
pub struct ThrottleState {
    pub last_ns: i128,
    pub counter: u32,
}

/// Placeholder posisi (bisa dikembangkan)
#[derive(Debug, Default)]
pub struct Positions {
    pub qty: i64,
}

#[derive(Debug, Error)]
pub enum RiskError {
    #[error("Notional limit exceeded")]
    Notional,
    #[error("Price out of band")]
    PriceBand,
    #[error("Throttle exceeded")]
    Throttle,
}

/// Pre-trade checks -> jika lolos, konversi Signal menjadi Order
fn check(
    sig: &Signal,
    lim: &Limits,
    _pos: &Positions,
    thr: &mut ThrottleState,
) -> Result<Order, RiskError> {
    // 1) Notional limit (px * qty)
    let notional = sig.px.saturating_mul(sig.qty);
    if notional > lim.max_notional {
        return Err(RiskError::Notional);
    }

    // 2) Price band
    if sig.px < lim.px_min || sig.px > lim.px_max {
        return Err(RiskError::PriceBand);
    }

    // 3) Throttle (contoh: jika <20ms dari last_ns, hitung counter; jika >max_qps, reject)
    let now: i128 = Utc::now().timestamp_nanos_opt().unwrap_or(0) as i128;
    if now - thr.last_ns < 20_000_000i128 {
        // 20 ms
        thr.counter += 1;
        if thr.counter > lim.max_qps {
            return Err(RiskError::Throttle);
        }
    } else {
        thr.counter = 0;
        thr.last_ns = now;
    }

    // 4) Build order (cl_id unik)
    let cl_id = format!("CL-{}-{}", now, rand::thread_rng().gen::<u32>());
    Ok(Order {
        cl_id,
        ts_ns: sig.ts_ns,
        symbol: sig.symbol.clone(),
        side: sig.side,
        px: sig.px,
        qty: sig.qty,
    })
}

/// Task risk: menerima Signal, menjalankan check(), lalu mengirim Order valid
pub async fn run(
    mut sig_rx: mpsc::Receiver<Signal>,
    ord_tx: mpsc::Sender<Order>,
    lim: Limits,
) {
    let pos = Positions::default();
    let mut thr = ThrottleState::default();

    while let Some(sig) = sig_rx.recv().await {
        match check(&sig, &lim, &pos, &mut thr) {
            Ok(ord) => {
                let _ = ord_tx.send(ord).await;
                ORDERS.inc();
            }
            Err(e) => warn!(?e, "risk rejected"),
        }
    }
}
