// ===============================
// src/metrics.rs
// ===============================
use once_cell::sync::Lazy;
use prometheus::{
    Encoder, Histogram, HistogramOpts, IntCounter, IntCounterVec, IntGauge, IntGaugeVec, Opts,
    Registry, TextEncoder,
};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

// Single custom registry (we register everything here)
pub static REGISTRY: Lazy<Registry> = Lazy::new(Registry::new);

// -------- Core trading metrics --------
pub static TICKS: Lazy<IntCounter> =
    Lazy::new(|| IntCounter::new("ticks_total", "market data ticks").unwrap());

pub static TICKS_BY_SYMBOL: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new("ticks_total_by_symbol", "market data ticks per symbol"),
        &["symbol"],
    )
    .unwrap()
});

pub static SIGNALS: Lazy<IntCounter> =
    Lazy::new(|| IntCounter::new("signals_total", "strategy signals").unwrap());

pub static SIGNALS_BY: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "signals_total_by",
            "strategy signals by strategy & symbol (labels: strategy, symbol)",
        ),
        &["strategy", "symbol"],
    )
    .unwrap()
});

pub static ORDERS: Lazy<IntCounter> =
    Lazy::new(|| IntCounter::new("orders_total", "orders accepted by risk").unwrap());

pub static EXECS: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new("exec_reports_total", "execution reports"),
        &["status", "venue"],
    )
    .unwrap()
});

// Latency from signal -> ack (milliseconds)
pub static LAT_SIG_ACK: Lazy<Histogram> = Lazy::new(|| {
    Histogram::with_opts(HistogramOpts::new(
        "latency_signal_to_ack_ms",
        "Latency from signal to ack (ms)",
    ))
    .unwrap()
});

// Router / venue scoring
pub static VENUE_SCORE: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(Opts::new("sor_venue_score", "router score"), &["venue"]).unwrap()
});

// Inventory & PnL
pub static INV_QTY: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("inventory_qty", "net qty per venue"),
        &["symbol", "venue"],
    )
    .unwrap()
});

pub static INV_TOTAL_QTY: Lazy<IntGauge> =
    Lazy::new(|| IntGauge::new("inventory_total_qty", "net qty total").unwrap());

pub static PNL_REALIZED: Lazy<IntGauge> =
    Lazy::new(|| IntGauge::new("pnl_realized", "realized PnL (ticks)").unwrap());

pub static PNL_UNREALIZED: Lazy<IntGauge> =
    Lazy::new(|| IntGauge::new("pnl_unrealized", "unrealized PnL (ticks)").unwrap());

// -------- Binance user-data stream health (optional, used by gateway_binance) --------
pub static BIN_WS_CONNECTED: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "binance_ws_connected",
            "1 if WS userDataStream connected, 0 otherwise",
        ),
        &["venue"],
    )
    .unwrap()
});

pub static BIN_WS_RECONNECTS: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "binance_ws_reconnects_total",
            "Number of reconnects to userDataStream WS",
        ),
        &["venue"],
    )
    .unwrap()
});

pub static BIN_WS_LAST_EVENT_TS: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "binance_ws_last_event_ts",
            "Unix seconds of the last received WS event",
        ),
        &["venue"],
    )
    .unwrap()
});

pub static BIN_WS_LAST_EVENT_AGE: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "binance_ws_last_event_age_seconds",
            "Age (seconds) since last WS event",
        ),
        &["venue"],
    )
    .unwrap()
});

pub static BIN_LISTEN_KEEPALIVE_OK: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "binance_listenkey_keepalive_ok_total",
            "Successful listenKey keepalive calls",
        ),
        &["venue"],
    )
    .unwrap()
});

pub static BIN_LISTEN_KEEPALIVE_ERR: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "binance_listenkey_keepalive_err_total",
            "Failed listenKey keepalive calls",
        ),
        &["venue"],
    )
    .unwrap()
});

// ---- Config visibility (feed / venue / strategies / symbols) ----
pub static CONFIG_FEED_MODE: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("config_feed_mode", "feed mode (label: mode)"),
        &["mode"],
    )
    .unwrap()
});

pub static CONFIG_VENUE_MODE: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("config_venue_mode", "venue mode (label: mode)"),
        &["mode"],
    )
    .unwrap()
});

pub static CONFIG_STRATEGY_ACTIVE: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "config_strategy_active",
            "active strategies (label: strategy) — value = workers",
        ),
        &["strategy"],
    )
    .unwrap()
});

pub static CONFIG_SYMBOL: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("config_symbol", "configured symbols (label: symbol)"),
        &["symbol"],
    )
    .unwrap()
});

pub fn init() {
    // Register all metrics to the custom registry
    for m in [
        REGISTRY.register(Box::new(TICKS.clone())),
        REGISTRY.register(Box::new(TICKS_BY_SYMBOL.clone())),
        REGISTRY.register(Box::new(SIGNALS.clone())),
        REGISTRY.register(Box::new(SIGNALS_BY.clone())),
        REGISTRY.register(Box::new(ORDERS.clone())),
        REGISTRY.register(Box::new(EXECS.clone())),
        REGISTRY.register(Box::new(LAT_SIG_ACK.clone())),
        REGISTRY.register(Box::new(VENUE_SCORE.clone())),
        REGISTRY.register(Box::new(INV_QTY.clone())),
        REGISTRY.register(Box::new(INV_TOTAL_QTY.clone())),
        REGISTRY.register(Box::new(PNL_REALIZED.clone())),
        REGISTRY.register(Box::new(PNL_UNREALIZED.clone())),
        // Binance WS health
        REGISTRY.register(Box::new(BIN_WS_CONNECTED.clone())),
        REGISTRY.register(Box::new(BIN_WS_RECONNECTS.clone())),
        REGISTRY.register(Box::new(BIN_WS_LAST_EVENT_TS.clone())),
        REGISTRY.register(Box::new(BIN_WS_LAST_EVENT_AGE.clone())),
        REGISTRY.register(Box::new(BIN_LISTEN_KEEPALIVE_OK.clone())),
        REGISTRY.register(Box::new(BIN_LISTEN_KEEPALIVE_ERR.clone())),
        // Config visibility
        REGISTRY.register(Box::new(CONFIG_FEED_MODE.clone())),
        REGISTRY.register(Box::new(CONFIG_VENUE_MODE.clone())),
        REGISTRY.register(Box::new(CONFIG_STRATEGY_ACTIVE.clone())),
        REGISTRY.register(Box::new(CONFIG_SYMBOL.clone())),
    ] {
        let _ = m;
    }
}

// Encode all metrics in Prometheus text format
fn encode_metrics() -> Vec<u8> {
    let encoder = TextEncoder::new();
    let families = REGISTRY.gather();
    let mut buf = Vec::new();
    if encoder.encode(&families, &mut buf).is_err() || buf.is_empty() {
        buf.extend_from_slice(b"# no metrics\n");
    }
    buf
}

// Serve one HTTP request (GET / or /metrics) — tiny HTTP 1.1 responder
fn handle_client(mut stream: TcpStream) {
    // Read a bit to consume headers (no full parse)
    let mut _req_buf = [0u8; 1024];
    let _ = stream.read(&mut _req_buf);

    let body = encode_metrics();
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );

    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(&body);
    let _ = stream.flush();
}

// Run the metrics server in a dedicated OS thread (keeps Tokio runtime clean)
pub async fn serve_metrics(port: u16) {
    thread::spawn(move || {
        let addr = format!("0.0.0.0:{port}");
        let listener = TcpListener::bind(&addr)
            .unwrap_or_else(|e| panic!("metrics bind {} failed: {}", addr, e));
        eprintln!("metrics listening on http://{addr}/ (and /metrics)");

        for conn in listener.incoming() {
            match conn {
                Ok(stream) => handle_client(stream),
                Err(e) => eprintln!("metrics accept error: {}", e),
            }
        }
    });
}
