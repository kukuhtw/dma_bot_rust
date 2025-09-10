// ===============================
// src/binance/mod.rs
// ===============================
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn timestamp_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

pub fn sign_query(secret: &str, query: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC can take key");
    mac.update(query.as_bytes());
    let sig = mac.finalize().into_bytes();
    hex::encode(sig)
}

// --- Model minimal untuk user data stream ---
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct WsEnvelope {
    pub e: Option<String>,
    #[serde(rename = "E")] pub E: Option<u64>,
    #[serde(rename = "o")] pub o: Option<OrderTradeUpdate>,
}

#[derive(Debug, Deserialize)]
pub struct OrderTradeUpdate {
    #[serde(rename = "s")] pub s: String,  // symbol
    #[serde(rename = "c")] pub c: String,  // clientOrderId
    #[serde(rename = "X")] pub X: String,  // current order status: NEW, PARTIALLY_FILLED, FILLED, CANCELED, REJECTED, EXPIRED
    #[serde(rename = "x")] pub x: String,  // execution type: NEW, TRADE, etc
    #[serde(rename = "L")] pub L: Option<String>, // last filled price
    #[serde(rename = "l")] pub l: Option<String>, // last filled qty
    #[serde(rename = "z")] pub z: Option<String>, // cum filled qty
    #[serde(rename = "ap")] pub ap: Option<String>, // avg price (sometimes 0 if not provided)
}
