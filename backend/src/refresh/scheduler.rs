use std::time::Duration;

use tokio::time::MissedTickBehavior;

use crate::service::Service;

/// One sequential timer per server process; missed ticks never create a refresh backlog.
pub(crate) async fn run(service: Service) {
    let mut timer = tokio::time::interval(Duration::from_secs(service.config.refresh.poll_seconds));
    timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        timer.tick().await;
        if let Err(error) = service.update_due().await {
            eprintln!("Automatic subscription refresh: {error}");
        }
    }
}
