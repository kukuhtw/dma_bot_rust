// ===============================
// src/recorder.rs
// ===============================
//
// JSONL recorder yang ringan & tahan banting:
// - Tulis setiap Event ke file .jsonl (append).
// - Bufer dengan BufWriter agar hemat syscall.
// - Flush periodik tiap 1s dan/atau tiap 1000 event.
// - Otomatis membuat parent directory jika belum ada.
// - Jika tulis gagal, coba reopen file dan lanjut.
//
// ENV: set `RECORD_FILE=/path/to/events.jsonl` agar aktif (lihat main.rs).
//
use std::path::Path;
use tokio::{
    fs::{self, OpenOptions},
    io::{AsyncWriteExt, BufWriter},
    sync::mpsc,
    time::{interval, Duration, MissedTickBehavior},
};
use tracing::{error, info};

use crate::domain::Event;

async fn open_writer(path: &str) -> BufWriter<tokio::fs::File> {
    // Pastikan parent directory ada (kalau ada)
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = fs::create_dir_all(parent).await {
                error!(?e, %path, "recorder: create_dir_all failed");
            }
        }
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .unwrap_or_else(|e| panic!("recorder: open {} failed: {}", path, e));

    BufWriter::new(file)
}

pub async fn run(mut rx: mpsc::Receiver<Event>, path: String) {
    info!(%path, "recorder: started");
    let mut writer = open_writer(&path).await;

    // Flush periodik (tiap 1 detik) + flush berbasis jumlah event
    let mut tick = interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let mut since_last_flush: u32 = 0;
    const FLUSH_EVERY_N_EVENTS: u32 = 1000;

    loop {
        tokio::select! {
            maybe_ev = rx.recv() => {
                match maybe_ev {
                    Some(ev) => {
                        // Serialize event
                        let line = match serde_json::to_string(&ev) {
                            Ok(s) => s,
                            Err(e) => {
                                error!(?e, "recorder: serialize error, skip event");
                                continue;
                            }
                        };

                        // Tulis + newline
                        if let Err(e) = writer.write_all(line.as_bytes()).await {
                            error!(?e, "recorder: write_all failed, attempting reopen");
                            writer = open_writer(&path).await;
                            // coba lagi sekali setelah reopen
                            if let Err(e2) = writer.write_all(line.as_bytes()).await {
                                error!(?e2, "recorder: write_all failed again after reopen, drop event");
                                continue;
                            }
                        }
                        if let Err(e) = writer.write_all(b"\n").await {
                            error!(?e, "recorder: write newline failed, attempting reopen");
                            writer = open_writer(&path).await;
                            // newline setelah reopen (opsional)
                            let _ = writer.write_all(b"\n").await;
                        }

                        since_last_flush += 1;
                        if since_last_flush >= FLUSH_EVERY_N_EVENTS {
                            let _ = writer.flush().await;
                            since_last_flush = 0;
                        }
                    }
                    None => {
                        // Channel closed: flush dan keluar
                        let _ = writer.flush().await;
                        info!("recorder: channel closed, stopped");
                        break;
                    }
                }
            }

            _ = tick.tick() => {
                // Flush periodik
                let _ = writer.flush().await;
                since_last_flush = 0;
            }
        }
    }
}
