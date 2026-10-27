//! 网络请求的共享退避策略：把「只重试传输类错误」的规则集中在一处。
//!
//! 开机瞬间网络还没就绪是自动签到最常见的失败原因，因此连接失败与超时会
//! 短暂重试；HTTP 4xx/5xx 属于服务端语义，交回调用方按原有文案处理，不重试。

use std::time::Duration;

use reqwest::{RequestBuilder, Response};
use tokio::time::sleep;

/// 传输类失败后的退避间隔，最多额外尝试两次。
pub(crate) const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(3), Duration::from_secs(10)];

/// 是否属于值得重试的传输类错误：连接失败或请求超时。
pub(crate) fn is_transient(error: &reqwest::Error) -> bool {
    error.is_connect() || error.is_timeout()
}

/// 发送请求，遇到传输类错误时按 [`RETRY_DELAYS`] 退避重试。
///
/// 每轮都重新克隆请求，保证请求体可重放；请求体不可重放时只发送一次，
/// 保持与调用方原本 `send()` 完全一致的语义与错误。
pub(crate) async fn send_with_retry(builder: RequestBuilder) -> Result<Response, reqwest::Error> {
    if builder.try_clone().is_none() {
        return builder.send().await;
    }

    let mut attempt = 0usize;
    loop {
        // 上一轮判断可重放，这里必然拿到克隆；拿不到时退回单次发送。
        let Some(request) = builder.try_clone() else {
            return builder.send().await;
        };
        match request.send().await {
            Err(error) if is_transient(&error) && attempt < RETRY_DELAYS.len() => {
                sleep(RETRY_DELAYS[attempt]).await;
                attempt += 1;
            }
            outcome => return outcome,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_delays_match_the_growth_backoff() {
        assert_eq!(
            RETRY_DELAYS,
            [Duration::from_secs(3), Duration::from_secs(10)]
        );
    }

    #[test]
    fn json_requests_can_be_replayed() {
        let client = reqwest::Client::new();
        let builder = client
            .post("https://copilot.tencent.com/v2/billing/meter/daily-checkin")
            .json(&serde_json::json!({}));

        assert!(builder.try_clone().is_some(), "JSON body 应可重放");
    }
}
