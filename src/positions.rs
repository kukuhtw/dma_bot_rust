// ===============================
// src/positions.rs (PnL & Inventory tracker)
// ===============================

use tokio::sync::{broadcast, watch};
use crate::domain::{ExecReport, InvSnapshot, MdTick, Side, SymbolState, VenuePosition};
use crate::metrics::{INV_QTY, INV_TOTAL_QTY, PNL_REALIZED, PNL_UNREALIZED};

pub struct PositionsTask {
    symbol: String,
    state: SymbolState,
}

impl PositionsTask {
    pub fn new(symbol: String) -> Self { Self { symbol, state: SymbolState::default() } }

    fn on_fill(&mut self, er: &ExecReport, side: Side) {
        // venue diambil dari suffix cl_id: ...-A / ...-B
        let venue = er.cl_id.split('-').last().unwrap_or("?").to_string();
        let entry = self.state.by_venue.entry(venue.clone()).or_insert(VenuePosition::default());
        let signed_qty = side.sign() * er.filled_qty;

        let prev_qty = entry.qty;
        let new_qty = prev_qty + signed_qty;
        if prev_qty == 0 || (prev_qty.signum() == signed_qty.signum()) {
            // arah sama -> update avg cost
            entry.avg_cost_px = if entry.qty == 0 {
                er.avg_px
            } else {
                ((entry.avg_cost_px * entry.qty) + (er.avg_px * signed_qty.abs())) / (entry.qty + signed_qty.abs())
            };
            entry.qty = new_qty;
        } else {
            // arah berlawanan -> realize PnL
            let qty_closed = signed_qty.abs().min(prev_qty.abs());
            let pnl = (er.avg_px - entry.avg_cost_px) as i64 * (if prev_qty > 0 { qty_closed } else { -qty_closed });
            entry.realized_pnl += pnl;
            entry.qty = new_qty;
            if entry.qty == 0 { entry.avg_cost_px = 0; }
        }

        // agregat
        self.state.total_qty = self.state.by_venue.values().map(|v| v.qty).sum();
        self.state.realized_pnl = self.state.by_venue.values().map(|v| v.realized_pnl).sum();

        // metrics
        INV_TOTAL_QTY.set(self.state.total_qty);
        for (v, pos) in self.state.by_venue.iter() {
            INV_QTY.with_label_values(&[&self.symbol, v]).set(pos.qty);
        }
        PNL_REALIZED.set(self.state.realized_pnl);
    }

    fn mark_to_market(&mut self, mid: i64) {
        self.state.last_mid = mid;
        let mut u = 0_i64;
        for pos in self.state.by_venue.values() {
            if pos.qty != 0 && pos.avg_cost_px != 0 {
                u += (mid - pos.avg_cost_px) * pos.qty;
            }
        }
        self.state.unrealized_pnl = u;
        PNL_UNREALIZED.set(u);
    }
}

pub async fn run(
    symbol: String,
    mut md_rx: broadcast::Receiver<MdTick>,
    mut exec_rx: tokio::sync::mpsc::Receiver<ExecReport>,
    snap_tx: watch::Sender<InvSnapshot>,
) {
    let mut task = PositionsTask::new(symbol.clone());
    loop {
        tokio::select! {
            Ok(md) = md_rx.recv() => {
                let mid = (md.best_bid + md.best_ask)/2;
                task.mark_to_market(mid);
                let _ = snap_tx.send(InvSnapshot { ts_ns: md.ts_ns, symbol: symbol.clone(), state: task.state.clone() });
            }
            Some(er) = exec_rx.recv() => {
                // Sementara infer side dari harga relatif mid
                let side = if task.state.last_mid <= er.avg_px { Side::Buy } else { Side::Sell };
                task.on_fill(&er, side);
                let _ = snap_tx.send(InvSnapshot { ts_ns: er.ts_ns, symbol: symbol.clone(), state: task.state.clone() });
            }
        }
    }
}
