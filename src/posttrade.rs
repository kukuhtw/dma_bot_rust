// ===============================
// src/posttrade.rs
// ===============================
use tokio::sync::mpsc;
use tracing::{info, warn};
use crate::domain::{ExecReport, ExecStatus};


pub async fn run(mut exec_rx: mpsc::Receiver<ExecReport>) {
while let Some(er) = exec_rx.recv().await {
match &er.status {
ExecStatus::Ack => info!(cl_id=?er.cl_id, symbol=?er.symbol, "ACK"),
ExecStatus::Filled => info!(cl_id=?er.cl_id, qty=?er.filled_qty, px=?er.avg_px, "FILLED"),
ExecStatus::PartialFill => info!(cl_id=?er.cl_id, qty=?er.filled_qty, px=?er.avg_px, "PARTIAL"),
ExecStatus::Rejected(r) => warn!(cl_id=?er.cl_id, reason=%r, "REJECT"),
}
}
}