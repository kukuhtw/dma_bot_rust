// ===============================
// src/domain.rs
// ===============================
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Side { Buy, Sell }
impl Side { pub fn sign(&self) -> i64 { match self { Side::Buy => 1, Side::Sell => -1 } } }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MdTick { pub ts_ns: i128, pub symbol: String, pub best_bid: i64, pub best_ask: i64 }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal { pub ts_ns: i128, pub symbol: String, pub side: Side, pub px: i64, pub qty: i64 }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order { pub cl_id: String, pub ts_ns: i128, pub symbol: String, pub side: Side, pub px: i64, pub qty: i64 }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VenueOrder { pub venue: String, pub order: Order }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecReport { pub cl_id: String, pub symbol: String, pub status: ExecStatus, pub filled_qty: i64, pub avg_px: i64, pub ts_ns: i128 }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecStatus { Ack, PartialFill, Filled, Rejected(String) }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event { Md(MdTick), Sig(Signal), Ord(Order), Exec(ExecReport), Note(String) }

// Inventory structures
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VenuePosition { pub qty: i64, pub avg_cost_px: i64, pub realized_pnl: i64 }
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SymbolState {
    pub last_mid: i64,
    pub total_qty: i64,
    pub realized_pnl: i64,
    pub unrealized_pnl: i64,
    pub by_venue: std::collections::HashMap<String, VenuePosition>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InvSnapshot { pub ts_ns: i128, pub symbol: String, pub state: SymbolState }
