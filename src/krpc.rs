use std::sync::Arc;

use anyhow::Result;
use krpc_client::{Client, services::space_center::SpaceCenter};
use tokio::sync::broadcast;

const STREAM_RATE_HZ: f32 = 5.0;

#[derive(Clone, Copy)]
pub struct Calendar {
    pub secs_per_day: f64,
    pub secs_per_year: f64,
}

const KERBIN_CALENDAR: Calendar = Calendar {
    secs_per_day: 21_600.0,
    secs_per_year: 9_201_600.0,
};

const EARTH_CALENDAR: Calendar = Calendar {
    secs_per_day: 86_400.0,
    secs_per_year: 31_536_000.0,
};

pub async fn detect_calendar(krpc: Arc<Client>) -> Result<Calendar> {
    let space_center = SpaceCenter::new(krpc);
    let bodies = space_center.get_bodies().await?;
    if bodies.contains_key("Kerbin") {
        Ok(KERBIN_CALENDAR)
    } else if bodies.contains_key("Earth") {
        Ok(EARTH_CALENDAR)
    } else {
        eprintln!(
            "warning: neither Kerbin nor Earth found in SpaceCenter.Bodies; \
             defaulting to Kerbin calendar"
        );
        Ok(KERBIN_CALENDAR)
    }
}

pub async fn run_ut_stream(krpc: Arc<Client>, tx: broadcast::Sender<f64>) {
    if let Err(e) = ut_stream_loop(krpc, tx).await {
        eprintln!("ut stream task ended: {e:#}");
    }
}

async fn ut_stream_loop(krpc: Arc<Client>, tx: broadcast::Sender<f64>) -> Result<()> {
    let space_center = SpaceCenter::new(krpc);
    let stream = space_center.get_ut_stream().await?;
    stream.set_rate(STREAM_RATE_HZ).await?;
    loop {
        stream.wait().await;
        let ut = stream.get().await?;
        let _ = tx.send(ut);
    }
}
