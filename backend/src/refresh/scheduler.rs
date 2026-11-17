//! 后台订阅自动刷新轮询调度器。

use std::time::Duration;

use tokio::time::MissedTickBehavior;

use crate::service::Service;

/// 启动全局后台定时轮询任务，按配置周期检查并执行到期的自动更新任务。
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
