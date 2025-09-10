// ===============================
// src/strategy.rs
// ===============================
//
// Disediakan 3 strategi:
// 1) Mean-Reversion (default)          -> function: run (alias run_mean_reversion)
// 2) MA Crossover (Trend-Following)    -> function: run_ma_crossover
// 3) Volatility Breakout (Range Break) -> function: run_vol_breakout
//
// Cara pakai cepat (tanpa ubah main.rs):
// - Strategi default yang dipanggil main.rs adalah `run()` = mean-reversion.
// - Untuk mencoba strategi lain, panggil fungsi `run_ma_crossover()` atau
//   `run_vol_breakout()` dari main.rs (atau spawn tambahan sebagai parallel strategy).
//
// Catatan domain harga:
// - MdTick.best_bid/best_ask dalam skala "tick" internal (di contoh PoC = 2 desimal).
// - Signal.qty, px, dan side menyesuaikan domain kamu.
//
// Remarks ringkas setiap strategi ada di komentar di atas state struct masing-masing.
//

use std::collections::VecDeque;
use tokio::sync::{broadcast, mpsc};
use tracing::{error, warn};
use crate::domain::{MdTick, Signal, Side};
use crate::metrics::SIGNALS;

fn mid_price(md: &MdTick) -> i64 {
    (md.best_bid + md.best_ask) / 2
}

// -----------------------------------------------------------------------------
// 1) MEAN-REVERSION (default)
//    Ide: jika harga saat ini (ask) < rata-rata N-bar - edge  -> Buy
//         jika harga saat ini (bid) > rata-rata N-bar + edge  -> Sell
//    Kapan cocok:
//      - Pasar cenderung sideways / reversion ke mean.
//    Risiko:
//      - Saat trending kuat, bisa melawan arus (perlu risk guard di modul risk).
// -----------------------------------------------------------------------------
pub struct StratState {
    window: VecDeque<i64>,
    sum: i64,
    edge: i64,
    w: usize,
}
impl StratState {
    pub fn new(w: usize, edge: i64) -> Self {
        Self { window: VecDeque::with_capacity(w), sum: 0, edge, w }
    }
    fn fair(&self) -> Option<i64> {
        if self.window.len() >= self.w { Some(self.sum / self.w as i64) } else { None }
    }
    pub fn on_tick(&mut self, md: &MdTick) -> Option<Signal> {
        if self.window.len() == self.w {
            if let Some(x) = self.window.pop_front() { self.sum -= x; }
        }
        let mid = mid_price(md);
        self.window.push_back(mid);
        self.sum += mid;

        if let Some(fair) = self.fair() {
            if md.best_ask < fair - self.edge {
                return Some(Signal { ts_ns: md.ts_ns, symbol: md.symbol.clone(), side: Side::Buy,  px: md.best_ask, qty: 10 });
            }
            if md.best_bid > fair + self.edge {
                return Some(Signal { ts_ns: md.ts_ns, symbol: md.symbol.clone(), side: Side::Sell, px: md.best_bid, qty: 10 });
            }
        }
        None
    }
}

pub async fn run(mut md_rx: broadcast::Receiver<MdTick>, sig_tx: mpsc::Sender<Signal>) {
    // Parameter default: MA window 64, edge 3 tick
    let mut st = StratState::new(64, 3);
    loop {
        match md_rx.recv().await {
            Ok(md) => {
                if let Some(sig) = st.on_tick(&md) {
                    if let Err(e) = sig_tx.send(sig).await { error!(?e, "signal send failed"); }
                    else { SIGNALS.inc(); }
                }
            },
            Err(e) => warn!(?e, "md channel closed"),
        }
    }
}

// -----------------------------------------------------------------------------
// 2) MOVING AVERAGE CROSSOVER (Trend-Following)
//    Ide: MA cepat menembus ke atas MA lambat -> Buy (golden cross)
//         MA cepat menembus ke bawah MA lambat -> Sell (dead cross)
//    Kapan cocok:
//      - Pasar trending (uptrend/downtrend).
//    Cara kerja singkat:
//      - Hitung SMA fast (w_fast) dan SMA slow (w_slow).
//      - Deteksi pergantian sign (fast_minus_slow) untuk sinyal crossing.
//    Filtering:
//      - Tambahkan "min_edge" (dalam tick) agar tak sensitif pada noise kecil.
//    Risiko:
//      - Choppy/ranging market bisa menghasilkan whipsaw (perlu risk/cooldown).
// -----------------------------------------------------------------------------
pub struct MACrossState {
    fast_w: usize,
    slow_w: usize,
    fast_win: VecDeque<i64>,
    slow_win: VecDeque<i64>,
    fast_sum: i64,
    slow_sum: i64,
    prev_diff_sign: i8, // -1, 0, +1
    min_edge: i64,      // threshold selisih min agar dianggap valid cross
    cooldown_ticks: u32,
    since_last: u32,
}
impl MACrossState {
    pub fn new(fast_w: usize, slow_w: usize, min_edge: i64, cooldown_ticks: u32) -> Self {
        Self {
            fast_w,
            slow_w,
            fast_win: VecDeque::with_capacity(fast_w),
            slow_win: VecDeque::with_capacity(slow_w),
            fast_sum: 0,
            slow_sum: 0,
            prev_diff_sign: 0,
            min_edge,
            cooldown_ticks,
            since_last: cooldown_ticks, // mulai bisa sinyal
        }
    }
    fn push_window(win: &mut VecDeque<i64>, sum: &mut i64, cap: usize, v: i64) {
        if win.len() == cap {
            if let Some(x) = win.pop_front() { *sum -= x; }
        }
        win.push_back(v);
        *sum += v;
    }
    fn sma(sum: i64, len: usize) -> Option<i64> {
        if len > 0 { Some(sum / len as i64) } else { None }
    }
    pub fn on_tick(&mut self, md: &MdTick) -> Option<Signal> {
        let m = mid_price(md);
        Self::push_window(&mut self.fast_win, &mut self.fast_sum, self.fast_w, m);
        Self::push_window(&mut self.slow_win, &mut self.slow_sum, self.slow_w, m);

        self.since_last = self.since_last.saturating_add(1);

        if self.fast_win.len() < self.fast_w || self.slow_win.len() < self.slow_w {
            return None;
        }
        let fast = Self::sma(self.fast_sum, self.fast_w).unwrap();
        let slow = Self::sma(self.slow_sum, self.slow_w).unwrap();
        let diff = fast - slow;

        // Edge filter: abaikan diff terlalu kecil (noise)
        if diff.abs() < self.min_edge { return None; }

        // Hitung sign sekarang
        let cur_sign: i8 = if diff > 0 { 1 } else { -1 };

        // Detect crossing hanya jika sign berubah & cooldown lewat
        if cur_sign != self.prev_diff_sign && self.since_last >= self.cooldown_ticks {
            self.prev_diff_sign = cur_sign;
            self.since_last = 0;

            if cur_sign > 0 {
                // Golden cross -> Buy di best_ask
                return Some(Signal { ts_ns: md.ts_ns, symbol: md.symbol.clone(), side: Side::Buy,  px: md.best_ask, qty: 10 });
            } else {
                // Dead cross -> Sell di best_bid
                return Some(Signal { ts_ns: md.ts_ns, symbol: md.symbol.clone(), side: Side::Sell, px: md.best_bid, qty: 10 });
            }
        }

        // update prev_sign pertama kali ketika sudah punya dua MA penuh
        if self.prev_diff_sign == 0 {
            self.prev_diff_sign = cur_sign;
        }
        None
    }
}

pub async fn run_ma_crossover(mut md_rx: broadcast::Receiver<MdTick>, sig_tx: mpsc::Sender<Signal>) {
    // Parameter default: fast=16, slow=64, min_edge=2 tick, cooldown=16 ticks
    let mut st = MACrossState::new(16, 64, 2, 16);
    loop {
        match md_rx.recv().await {
            Ok(md) => {
                if let Some(sig) = st.on_tick(&md) {
                    if let Err(e) = sig_tx.send(sig).await { error!(?e, "signal send failed"); }
                    else { SIGNALS.inc(); }
                }
            },
            Err(e) => warn!(?e, "md channel closed"),
        }
    }
}

// -----------------------------------------------------------------------------
// 3) VOLATILITY BREAKOUT (Range Break)
//    Ide: deteksi harga menembus rentang high/low rolling window + buffer
//         (buy ketika mid > rolling_high + edge, sell ketika mid < rolling_low - edge).
//    Kapan cocok:
//      - Saat ada expansion volatilitas / breakout dari konsolidasi.
//    Parameter:
//      - w: panjang window rolling high/low
//      - edge: buffer di atas/bawah level breakout untuk mengurangi false break.
//    Risiko:
//      - False breakout ketika market cepat kembali ke dalam range.
// -----------------------------------------------------------------------------
pub struct VolBreakoutState {
    w: usize,
    edge: i64,
    window: VecDeque<i64>,
    rolling_high: i64,
    rolling_low: i64,
    // Optional cooldown supaya tak spam sinyal
    cooldown_ticks: u32,
    since_last: u32,
}
impl VolBreakoutState {
    pub fn new(w: usize, edge: i64, cooldown_ticks: u32) -> Self {
        Self {
            w,
            edge,
            window: VecDeque::with_capacity(w),
            rolling_high: i64::MIN / 4,
            rolling_low: i64::MAX / 4,
            cooldown_ticks,
            since_last: cooldown_ticks,
        }
    }
    fn recompute_hilo(win: &VecDeque<i64>) -> (i64, i64) {
        let mut hi = i64::MIN / 4;
        let mut lo = i64::MAX / 4;
        for &v in win.iter() {
            if v > hi { hi = v; }
            if v < lo { lo = v; }
        }
        (hi, lo)
    }
    pub fn on_tick(&mut self, md: &MdTick) -> Option<Signal> {
        self.since_last = self.since_last.saturating_add(1);

        let m = mid_price(md);
        if self.window.len() == self.w {
            self.window.pop_front();
        }
        self.window.push_back(m);

        if self.window.len() < self.w {
            // butuh window penuh untuk level breakout
            return None;
        }

        // Recompute high/low ketika jendela lengkap
        let (hi, lo) = Self::recompute_hilo(&self.window);
        self.rolling_high = hi;
        self.rolling_low = lo;

        // Sinyal breakout + buffer edge + cooldown
        if self.since_last >= self.cooldown_ticks {
            if m > self.rolling_high + self.edge {
                self.since_last = 0;
                // Buy pada momentum break di best_ask
                return Some(Signal { ts_ns: md.ts_ns, symbol: md.symbol.clone(), side: Side::Buy,  px: md.best_ask, qty: 10 });
            }
            if m < self.rolling_low - self.edge {
                self.since_last = 0;
                // Sell pada momentum break di best_bid
                return Some(Signal { ts_ns: md.ts_ns, symbol: md.symbol.clone(), side: Side::Sell, px: md.best_bid, qty: 10 });
            }
        }
        None
    }
}

pub async fn run_vol_breakout(mut md_rx: broadcast::Receiver<MdTick>, sig_tx: mpsc::Sender<Signal>) {
    // Parameter default: window=100, edge=5 tick, cooldown=20 ticks
    let mut st = VolBreakoutState::new(100, 5, 20);
    loop {
        match md_rx.recv().await {
            Ok(md) => {
                if let Some(sig) = st.on_tick(&md) {
                    if let Err(e) = sig_tx.send(sig).await { error!(?e, "signal send failed"); }
                    else { SIGNALS.inc(); }
                }
            },
            Err(e) => warn!(?e, "md channel closed"),
        }
    }
}
