use std::time::Duration;

use chrono::Local;
use tauri::{AppHandle, Manager};

use super::dashboard::{self, DashboardRuntime};

const RECALIBRATION_SECONDS: u64 = 60;
/// 成长中心轮询间隔：把旅行礼物及时领回，并兜住当天漏签。
const GROWTH_POLL_INTERVAL: chrono::Duration = chrono::Duration::hours(4);

pub(crate) fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        run(app).await;
    });
}

async fn run(app: AppHandle) {
    let runtime = app.state::<DashboardRuntime>();
    // 启动时先广播一次：调度计划只有写进快照，前端才看得到。
    runtime.set_scheduler(true, None).await;
    runtime.emit(&app).await;
    run_automatic_batch(&app, runtime.inner()).await;

    loop {
        let now = chrono::Local::now();
        let next = dashboard::next_local_midnight(now);
        let schedule_changed = runtime.set_scheduler(true, Some(next.to_rfc3339())).await;
        let growth_changed = runtime
            .set_next_growth_run(Some(
                next_growth_run_at(runtime.inner(), now).await.to_rfc3339(),
            ))
            .await;
        if schedule_changed || growth_changed {
            runtime.emit(&app).await;
        }

        let until_midnight = (next - now)
            .to_std()
            .unwrap_or_else(|_| Duration::from_secs(1));
        tokio::time::sleep(until_midnight.min(Duration::from_secs(RECALIBRATION_SECONDS))).await;

        let today = dashboard::local_today();
        if runtime.auto_date_is_pending(today).await {
            run_automatic_batch(&app, runtime.inner()).await;
            continue;
        }
        if growth_poll_is_due(runtime.inner()).await {
            run_poll_batch(&app, runtime.inner()).await;
        }
    }
}

async fn run_automatic_batch(app: &AppHandle, runtime: &DashboardRuntime) {
    let today = dashboard::local_today();
    if !runtime.auto_date_is_pending(today).await {
        return;
    }

    if dashboard::run_checkin_all(runtime, app).await.is_err() {
        return;
    }
    let _ = runtime.mark_auto_date_if_needed(today).await;
    runtime.mark_batch_ran(Local::now().to_rfc3339()).await;

    // 每日批次同时跑成长中心；开关关闭时自动运行不发成长请求。
    if runtime.app_settings().await.growth_automation_enabled {
        let _ = dashboard::run_growth_all(runtime, app, "daily").await;
    }
}

/// 轮询批次：先补签（幂等），再按开关决定是否跑成长中心。
async fn run_poll_batch(app: &AppHandle, runtime: &DashboardRuntime) {
    // 先记时间再干活：日常批次本身每 60 秒会重试补签，轮询不必叠加同一份重试，
    // 否则持续的网络故障会让应用每分钟打一次接口。
    runtime.mark_batch_ran(Local::now().to_rfc3339()).await;
    if dashboard::run_checkin_all(runtime, app).await.is_err() && !runtime.is_logged_in().await {
        // 补签失败且登录已被清空：没有可用凭据，成长中心不必再发一轮必然失败的请求。
        return;
    }
    if runtime.app_settings().await.growth_automation_enabled {
        let _ = dashboard::run_growth_all(runtime, app, "poll").await;
    }
}

async fn growth_poll_is_due(runtime: &DashboardRuntime) -> bool {
    if !runtime.is_logged_in().await {
        return false;
    }
    // 有登录但没有批次记录（启动时尚未登录、登录后才恢复）时立刻跑一轮。
    let Some(last) = runtime.last_batch_at().await else {
        return true;
    };
    match chrono::DateTime::parse_from_rfc3339(&last) {
        Ok(last) => last.with_timezone(&Local) + GROWTH_POLL_INTERVAL <= Local::now(),
        Err(_) => false,
    }
}

/// 下一次成长轮询时间：以上一轮运行时间为基准；没有记录时按下个间隔展示。
async fn next_growth_run_at(
    runtime: &DashboardRuntime,
    now: chrono::DateTime<Local>,
) -> chrono::DateTime<Local> {
    let Some(last) = runtime.last_batch_at().await else {
        return now + GROWTH_POLL_INTERVAL;
    };
    match chrono::DateTime::parse_from_rfc3339(&last) {
        Ok(last) => {
            let next = last.with_timezone(&Local) + GROWTH_POLL_INTERVAL;
            if next <= now { now } else { next }
        }
        Err(_) => now,
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn poll_interval_is_four_hours() {
        assert_eq!(GROWTH_POLL_INTERVAL, chrono::Duration::hours(4));
    }

    #[test]
    fn recalibration_stays_within_the_poll_interval() {
        const _: () = assert!(RECALIBRATION_SECONDS <= 60);
        let now = Local
            .with_ymd_and_hms(2026, 8, 15, 10, 0, 0)
            .single()
            .expect("local time should be valid");

        assert_eq!(
            now + GROWTH_POLL_INTERVAL,
            Local
                .with_ymd_and_hms(2026, 8, 15, 14, 0, 0)
                .single()
                .expect("local time should be valid")
        );
    }
}
