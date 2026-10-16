//! 抽奖盲盒、Buddy 盲盒与能量读数。抽奖每轮只开一次，剩余机会留给下一轮。

use super::{GrowthRun, buddy_open_count, dig, text_of, u32_of};

impl GrowthRun<'_> {
    pub(super) async fn run_lottery(&mut self) {
        if !self.budget_left() {
            self.note_skipped("lottery", "开盲盒", "时间预算耗尽，盲盒跳过");
            return;
        }

        let chances = match self.get_json("/lottery/chances").await {
            Ok(body) if super::api_success(&body) => u32_of(dig(&body, "balance")).unwrap_or(0),
            Ok(body) => {
                let detail = super::business_detail(&body);
                self.note_failure_by_severity(
                    "lottery",
                    "开盲盒",
                    format!("查询抽奖机会失败：{detail}"),
                    true,
                );
                return;
            }
            Err(failure) => {
                self.note_call_failure("lottery", "开盲盒", "查询抽奖机会", failure);
                return;
            }
        };
        self.overview.lottery_chances = Some(chances);
        if chances == 0 {
            return;
        }

        let payload = serde_json::json!({ "client_token": super::client_token() });
        match self.post_json("/lottery/draw", payload).await {
            Ok(body) if super::api_success(&body) => {
                let mut prize = text_of(dig(&body, "prize_name"))
                    .or_else(|| text_of(dig(&body, "prize")))
                    .unwrap_or_else(|| "未知".to_owned());
                // 实物奖需要用户自己去成长中心填收件信息，必须提醒。
                let needs_address = super::bool_of(dig(&body, "need_address")) == Some(true)
                    || super::bool_of(dig(&body, "require_address")) == Some(true);
                if needs_address {
                    prize.push_str("（实物奖，需到成长中心填写收件信息）");
                }
                self.overview.lottery_chances = Some(chances.saturating_sub(1));
                self.note_ok("lottery", "开盲盒", format!("开盲盒获得：{prize}"));
                if chances > 1 {
                    self.note_normal(
                        "lottery",
                        "开盲盒",
                        format!("还剩 {} 次抽奖机会，下轮继续", chances - 1),
                    );
                }
            }
            Ok(body) => {
                if is_no_chance(&super::body_message(&body)) {
                    self.note_normal("lottery", "开盲盒", "没有可用的抽奖机会".to_owned());
                } else {
                    let detail = super::business_detail(&body);
                    self.note_failure_by_severity(
                        "lottery",
                        "开盲盒",
                        format!("开盲盒失败：{detail}"),
                        false,
                    );
                }
            }
            Err(failure) => {
                self.note_call_failure("lottery", "开盲盒", "开盲盒", failure);
            }
        }
    }

    pub(super) async fn run_buddy_open(&mut self) {
        if !self.budget_left() {
            self.note_skipped("buddy", "Buddy 盲盒", "时间预算耗尽，Buddy 盲盒跳过");
            return;
        }

        let quota = match self.get_json("/buddy/quota").await {
            Ok(body) if super::api_success(&body) => body,
            Ok(body) => {
                let detail = super::business_detail(&body);
                self.note_failure_by_severity(
                    "buddy",
                    "Buddy 盲盒",
                    format!("查询 Buddy 能量失败：{detail}"),
                    true,
                );
                return;
            }
            Err(failure) => {
                self.note_call_failure("buddy", "Buddy 盲盒", "查询 Buddy 能量", failure);
                return;
            }
        };

        let affordable = u32_of(dig(&quota, "affordable")).unwrap_or(0);
        let max_open = u32_of(dig(&quota, "max_open_count"));
        let Some(count) = buddy_open_count(affordable, max_open) else {
            return;
        };

        let payload = serde_json::json!({ "count": count, "client_token": super::client_token() });
        match self.post_json("/buddy/open", payload).await {
            Ok(body) if super::api_success(&body) => {
                let name = text_of(dig(&body, "buddy"))
                    .or_else(|| text_of(dig(&body, "name")))
                    .unwrap_or_else(|| "新 Buddy".to_owned());
                self.note_ok(
                    "buddy",
                    "Buddy 盲盒",
                    format!("开 Buddy 盲盒 ×{count}（{name}）"),
                );
            }
            Ok(body) => {
                let detail = super::business_detail(&body);
                self.note_failure_by_severity(
                    "buddy",
                    "Buddy 盲盒",
                    format!("开 Buddy 盲盒失败：{detail}"),
                    false,
                );
            }
            Err(failure) => {
                self.note_call_failure("buddy", "Buddy 盲盒", "开 Buddy 盲盒", failure);
            }
        }
    }

    /// 能量与连签是纯展示值：取不到不算失败，也不影响退出状态。
    pub(super) async fn run_status_readout(&mut self) {
        self.refresh_streak_if_stale().await;
        if !self.budget_left() {
            return;
        }
        if let Ok(body) = self.get_json("/energy").await {
            self.energy = text_of(dig(&body, "balance"));
        }
    }
}

/// 抽奖次数为 0 是常态（次数来自连登兑换），不是故障。
fn is_no_chance(message: &str) -> bool {
    let message = message.to_lowercase();
    if message.is_empty() {
        return false;
    }
    if message.contains("insufficient") || message.contains("not enough") {
        return message.contains("chance") || message.contains("balance");
    }
    message.contains("no chance")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_chance_detection_matches_the_server_wording() {
        assert!(is_no_chance("insufficient lottery chance balance"));
        assert!(is_no_chance("not enough balance"));
        assert!(is_no_chance("no chance"));
        assert!(!is_no_chance("invalid request"));
        assert!(!is_no_chance(""));
    }
}
