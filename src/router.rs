// ===============================
// src/router.rs (SOR + inventory bias)
// ===============================
use ahash::AHashMap as HashMap;
use tokio::sync::{mpsc, watch};
use crate::domain::{InvSnapshot, Order, VenueOrder};
use crate::metrics::VENUE_SCORE;

#[derive(Debug, Clone)]
pub struct VenueCfg { pub fee_bps: i32, pub est_latency_ms: u32, pub liq_score: u32 }

#[derive(Debug, Clone)]
pub struct RouterCfg {
    pub venues: HashMap<String, VenueCfg>,
    pub top_n: usize,
    pub min_child_qty: i64,
    pub inv_target: i64,
    pub inv_bias_weight: i64,
}

impl Default for RouterCfg {
    fn default() -> Self {
        let mut venues = HashMap::new();
        venues.insert("A".into(), VenueCfg { fee_bps: 5, est_latency_ms: 3, liq_score: 70 });
        venues.insert("B".into(), VenueCfg { fee_bps: 7, est_latency_ms: 2, liq_score: 50 });
        venues.insert("C".into(), VenueCfg { fee_bps: 2, est_latency_ms: 6, liq_score: 90 });
        Self { venues, top_n: 2, min_child_qty: 2, inv_target: 0, inv_bias_weight: 5 }
    }
}

fn score_base(v: &VenueCfg, px: i64) -> i64 {
    let fee_ticks = (v.fee_bps as i64) * px / 10_000;
    let lat_penalty = v.est_latency_ms as i64;
    (v.liq_score as i64) - fee_ticks - lat_penalty
}

pub async fn run(
    mut ord_rx: mpsc::Receiver<Order>,
    gw_txs: HashMap<String, mpsc::Sender<VenueOrder>>,
    cfg: RouterCfg,
    mut inv_snap_rx: watch::Receiver<InvSnapshot>,
) {
    let mut last_inv: Option<InvSnapshot> = inv_snap_rx.borrow().clone().into();

    loop {
        tokio::select! {
            _ = inv_snap_rx.changed() => { last_inv = Some(inv_snap_rx.borrow().clone()); }
            Some(o) = ord_rx.recv() => {
                let px = o.px;
                // 1) skor dasar
                let mut ranked: Vec<(String, i64)> =
                    cfg.venues.iter().map(|(k,v)| (k.clone(), score_base(v, px))).collect();

                // 2) bias inventory (mendekati target)
                if let Some(inv) = &last_inv {
                    for (venue, s) in ranked.iter_mut() {
                        let cur_qty = inv.state.by_venue.get(venue).map(|vp| vp.qty).unwrap_or(0);
                        let bias = -cur_qty.signum() as i64 * cfg.inv_bias_weight;
                        *s += bias;
                        VENUE_SCORE.with_label_values(&[venue]).set(*s);
                    }
                }

                // 3) top-N
                ranked.sort_by_key(|(_,s)| -s);
                let top = ranked.into_iter().take(cfg.top_n).collect::<Vec<_>>();

                // 4) bagi qty berdasar likuiditas
                let total_liq: u32 = top.iter().map(|(k,_)| cfg.venues.get(k).unwrap().liq_score).sum();
                let mut remaining = o.qty;

                for (i,(k,_)) in top.iter().enumerate() {
                    let liq = cfg.venues.get(k).unwrap().liq_score as i64;
                    let share = if i == top.len()-1 {
                        remaining
                    } else {
                        (o.qty as i64 * liq / total_liq as i64).max(cfg.min_child_qty)
                    };
                    remaining -= share;
                    if share <= 0 { continue; }

                    if let Some(tx) = gw_txs.get(k) {
                        let child = Order { qty: share, cl_id: format!("{}-{}", o.cl_id, k), ..o.clone() };
                        let _ = tx.send(VenueOrder { venue: k.clone(), order: child }).await;
                    }
                }
            }
        }
    }
}
