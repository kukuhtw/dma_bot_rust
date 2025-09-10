// ===============================
// src/gateway_binance.rs
// ===============================
use chrono::Utc;
use futures_util::StreamExt;
use tokio::{
    sync::mpsc,
    time::{sleep, Duration},
};
use tokio_tungstenite::connect_async;
use url::Url;

use crate::binance::{sign_query, timestamp_ms, WsEnvelope};
use crate::domain::{ExecReport, ExecStatus, Side, VenueOrder};
use crate::metrics::EXECS;

/// Binance gateway (REST + User Data Stream).
/// PoC: submit LIMIT GTC orders only; fills/updates come from userDataStream WS.
pub async fn run_venue_binance(
    mut rx: mpsc::Receiver<VenueOrder>,
    exec_tx: mpsc::Sender<ExecReport>,
    venue: String,
) {
    // ENV
    let rest_base =
        std::env::var("BINANCE_REST_URL").unwrap_or_else(|_| "https://testnet.binance.vision".to_string());
    let ws_base =
        std::env::var("BINANCE_WS_URL").unwrap_or_else(|_| "wss://testnet.binance.vision/ws".to_string());
    let api_key = std::env::var("BINANCE_API_KEY").expect("BINANCE_API_KEY missing");
    let api_sec = std::env::var("BINANCE_API_SECRET").expect("BINANCE_API_SECRET missing");
    let recv_window = std::env::var("BINANCE_RECV_WINDOW")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(5000);

    let http = reqwest::Client::new();

    // 1) Get listenKey
    let listen_key: String = match create_listen_key(&http, &rest_base, &api_key).await {
        Ok(k) => k,
        Err(e) => {
            tracing::error!(?e, "create listenKey failed");
            return;
        }
    };

    // 2) Spawn WS user data stream
    let exec_tx_ws = exec_tx.clone();
    let venue_ws = venue.clone();
    tokio::spawn(async move { user_stream_ws_loop(&ws_base, &listen_key, exec_tx_ws, venue_ws).await });

    // 3) Consume orders from router
    while let Some(vord) = rx.recv().await {
        let o = vord.order;

        // Immediate ACK (gateway received)
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

        // Build LIMIT GTC params
        let ts = timestamp_ms();
        let symbol_up = o.symbol.to_ascii_uppercase();
        let price = (o.px as f64) / 100.0;
        let qty = o.qty as f64;

        let side = match o.side {
            Side::Buy => "BUY",
            Side::Sell => "SELL",
        };

        let params = vec![
            ("symbol".to_string(), symbol_up.clone()),
            ("side".to_string(), side.to_string()),
            ("type".to_string(), "LIMIT".to_string()),
            ("timeInForce".to_string(), "GTC".to_string()),
            ("quantity".to_string(), format!("{qty}")),
            ("price".to_string(), format!("{price}")),
            ("timestamp".to_string(), ts.to_string()),
            ("recvWindow".to_string(), recv_window.to_string()),
            ("newClientOrderId".to_string(), o.cl_id.clone()),
        ];

        let query = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        let sig = sign_query(&api_sec, &query);
        let url = format!("{}/api/v3/order?{}&signature={}", rest_base, query, sig);

        // Send order
        let resp = http.post(url).header("X-MBX-APIKEY", &api_key).send().await;

        match resp {
            Ok(rsp) if rsp.status().is_success() => {
                tracing::info!("order sent OK: cl_id={}", o.cl_id);
                // Fills/partial fills will arrive via WS ORDER_TRADE_UPDATE
            }
            Ok(rsp) => {
                let code = rsp.status();
                let body = rsp.text().await.unwrap_or_default();
                tracing::error!(%code, %body, "order send failed");
                let rej = ExecReport {
                    cl_id: o.cl_id.clone(),
                    symbol: o.symbol.clone(),
                    status: ExecStatus::Rejected(body),
                    filled_qty: 0,
                    avg_px: 0,
                    ts_ns: Utc::now().timestamp_nanos_opt().unwrap_or(0) as i128,
                };
                let _ = exec_tx.send(rej).await;
                EXECS.with_label_values(&["rejected", &venue]).inc();
            }
            Err(e) => {
                tracing::error!(?e, "order send err");
                let rej = ExecReport {
                    cl_id: o.cl_id.clone(),
                    symbol: o.symbol.clone(),
                    status: ExecStatus::Rejected(format!("{e}")),
                    filled_qty: 0,
                    avg_px: 0,
                    ts_ns: Utc::now().timestamp_nanos_opt().unwrap_or(0) as i128,
                };
                let _ = exec_tx.send(rej).await;
                EXECS.with_label_values(&["rejected", &venue]).inc();
            }
        }

        // small pacing to avoid rate limits in PoC
        sleep(Duration::from_millis(50)).await;
    }
}

async fn create_listen_key(
    http: &reqwest::Client,
    rest_base: &str,
    api_key: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!("{}/api/v3/userDataStream", rest_base);
    let rsp = http.post(url).header("X-MBX-APIKEY", api_key).send().await?;
    let v = rsp.json::<serde_json::Value>().await?;
    let lk = v
        .get("listenKey")
        .and_then(|x| x.as_str())
        .ok_or("no listenKey")?;
    Ok(lk.to_string())
}

async fn user_stream_ws_loop(
    ws_base: &str,
    listen_key: &str,
    exec_tx: mpsc::Sender<crate::domain::ExecReport>,
    venue: String,
) {
    let ws_url = format!("{}/{}", ws_base.trim_end_matches('/'), listen_key);
    loop {
        match Url::parse(&ws_url) {
            Ok(u) => {
                tracing::info!(%ws_url, "connecting userDataStream");
                match connect_async(u).await {
                    Ok((mut ws, _)) => {
                        while let Some(msg) = ws.next().await {
                            match msg {
                                Ok(m) if m.is_text() => {
                                    if let Ok(env) =
                                        serde_json::from_str::<WsEnvelope>(&m.into_text().unwrap_or_default())
                                    {
                                        if env.e.as_deref() == Some("ORDER_TRADE_UPDATE") {
                                            if let Some(ord) = env.o {
                                                // Map -> ExecReport
                                                let status = match ord.X.as_str() {
                                                    "NEW" => ExecStatus::Ack,
                                                    "PARTIALLY_FILLED" => ExecStatus::PartialFill,
                                                    "FILLED" => ExecStatus::Filled,
                                                    "CANCELED" | "EXPIRED" => ExecStatus::Rejected(ord.X.clone()),
                                                    "REJECTED" => ExecStatus::Rejected("REJECTED".to_string()),
                                                    _ => ExecStatus::Ack,
                                                };

                                                let cum_filled: i64 = ord
                                                    .z
                                                    .as_deref()
                                                    .and_then(|s| s.parse::<f64>().ok())
                                                    .unwrap_or(0.0) as i64;

                                                let avg_px: i64 = ord
                                                    .ap
                                                    .as_deref()
                                                    .and_then(|s| s.parse::<f64>().ok())
                                                    .map(|p| (p * 100.0).round() as i64)
                                                    .unwrap_or(0);

                                                // Derive metric label without moving `status`
                                                let label: &str = match &status {
                                                    ExecStatus::Ack => "ack",
                                                    ExecStatus::PartialFill => "partial",
                                                    ExecStatus::Filled => "filled",
                                                    ExecStatus::Rejected(_) => "rejected",
                                                };
                                                EXECS.with_label_values(&[label, &venue]).inc();

                                                // Now move status into the report
                                                let er = ExecReport {
                                                    cl_id: ord.c,
                                                    symbol: ord.s,
                                                    status,
                                                    filled_qty: cum_filled,
                                                    avg_px,
                                                    ts_ns: Utc::now().timestamp_nanos_opt().unwrap_or(0) as i128,
                                                };
                                                let _ = exec_tx.send(er).await;
                                            }
                                        }
                                    }
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    tracing::error!(?e, "userDataStream ws error");
                                    break;
                                }
                            }
                        }
                        tracing::warn!("userDataStream disconnected, reconnecting …");
                    }
                    Err(e) => {
                        tracing::error!(?e, "connect userDataStream failed");
                    }
                }
            }
            Err(e) => {
                tracing::error!(?e, "bad userDataStream url");
                return;
            }
        }
        sleep(Duration::from_secs(2)).await;
    }
}
