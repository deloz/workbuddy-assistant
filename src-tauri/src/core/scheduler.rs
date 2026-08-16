use std::time::Duration;

use tauri::{AppHandle, Manager};

use super::dashboard::{self, DashboardRuntime};

const RECALIBRATION_SECONDS: u64 = 60;

pub(crate) fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        run(app).await;
    });
}

async fn run(app: AppHandle) {
    let runtime = app.state::<DashboardRuntime>();
    runtime.set_scheduler(true, None).await;
    run_automatic_batch(&app, runtime.inner()).await;

    loop {
        let now = chrono::Local::now();
        let next = dashboard::next_local_midnight(now);
        runtime.set_scheduler(true, Some(next.to_rfc3339())).await;

        let until_midnight = (next - now)
            .to_std()
            .unwrap_or_else(|_| Duration::from_secs(1));
        tokio::time::sleep(until_midnight.min(Duration::from_secs(RECALIBRATION_SECONDS))).await;

        let today = dashboard::local_today();
        if runtime.auto_date_is_pending(today).await {
            run_automatic_batch(&app, runtime.inner()).await;
        }
    }
}

async fn run_automatic_batch(app: &AppHandle, runtime: &DashboardRuntime) {
    let today = dashboard::local_today();
    if !runtime.auto_date_is_pending(today).await {
        return;
    }

    if dashboard::run_checkin_all(runtime, app).await.is_ok() {
        let _ = runtime.mark_auto_date_if_needed(today).await;
    }
}
