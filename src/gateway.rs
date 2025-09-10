// ===============================
// src/gateway.rs (per-venue)
// ===============================
use chrono::Utc;
use tokio::{sync::mpsc, time::{sleep, Duration}};
use crate::domain::{ExecReport, ExecStatus, VenueOrder};
use crate::metrics::EXECS;

pub async fn run_venue(
    mut rx: mpsc::Receiver<VenueOrder>,
    exec_tx: mpsc::Sender<ExecReport>,
    venue: String,
    fill_ms: u64,
) {
    while let Some(vord) = rx.recv().await {
        let o = vord.order;

        let ack = ExecReport {
            cl_id: o.cl_id.clone(),
            symbol: o.symbol.clone(),
            status: ExecStatus::Ack,
            filled_qty: 0,
            avg_px: 0,
            ts_ns: Utc::now().timestamp_nanos_opt().unwrap_or(0) as i128,
        };
        let _ = exec_tx.send(ack).await;
        EXECS.with_label_values(&["ack", &venue]).inc();

        sleep(Duration::from_millis(fill_ms)).await;

        let fill = ExecReport {
            cl_id: o.cl_id.clone(),
            symbol: o.symbol.clone(),
            status: ExecStatus::Filled,
            filled_qty: o.qty,
            avg_px: o.px,
            ts_ns: Utc::now().timestamp_nanos_opt().unwrap_or(0) as i128,
        };
        let _ = exec_tx.send(fill).await;
        EXECS.with_label_values(&["filled", &venue]).inc();
    }
}
