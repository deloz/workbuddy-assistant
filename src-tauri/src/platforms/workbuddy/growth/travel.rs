//! Buddy 旅行：先领回已到达的礼物，再把空闲的 Buddy 派出去。

use super::{GrowthRun, dig, text_of, travel_eta, u32_of};

impl GrowthRun<'_> {
    pub(super) async fn run_travel(&mut self) {
        if !self.budget_left() {
            self.note_skipped("travel", "Buddy 旅行", "时间预算耗尽，旅行模块跳过");
            return;
        }

        let status = match self.get_json("/buddy/travel/status").await {
            Ok(status) => status,
            Err(failure) => {
                self.note_call_failure("travel", "Buddy 旅行", "查询旅行状态", failure);
                return;
            }
        };

        let state = text_of(dig(&status, "state"));
        let daily_limit = super::bool_of(dig(&status, "daily_limit_reached"));
        let location = dig(&status, "location").and_then(|location| text_of(dig(location, "name")));
        self.overview.travel_state = state.clone();
        self.overview.travel_location = location.clone();
        self.overview.travel_daily_limit_reached = daily_limit;

        if state.as_deref() == Some("traveling") {
            let eta = travel_eta(dig(&status, "arrive_at"), dig(&status, "server_now"));
            self.overview.travel_eta =
                (!eta.is_empty()).then(|| eta.trim_start_matches('，').to_owned());
            let place = location.clone().unwrap_or_else(|| "未知地点".to_owned());
            self.note_normal(
                "travel",
                "Buddy 旅行",
                format!("Buddy 正在{place}旅行{eta}"),
            );
            return;
        }

        let mut travel_state = state;
        if travel_state.as_deref() == Some("arrived") {
            let record_id = dig(&status, "record_id").cloned();
            let payload = serde_json::json!({ "record_id": record_id });
            match self.post_json("/buddy/travel/claim", payload).await {
                Ok(body) if body_reward_credit(&body).is_some() => {
                    let credit = body_reward_credit(&body).unwrap_or_default();
                    self.credits_gained += credit;
                    self.note_ok(
                        "travel",
                        "Buddy 旅行",
                        format!("领旅行礼物 +{} 积分", super::format_decimal(credit)),
                    );
                    travel_state = Some("idle".to_owned());
                }
                Ok(body) => {
                    let detail = super::business_detail(&body);
                    self.note_failure_by_severity(
                        "travel",
                        "Buddy 旅行",
                        format!("领旅行礼物失败：{detail}"),
                        true,
                    );
                    return;
                }
                Err(failure) => {
                    self.note_call_failure("travel", "Buddy 旅行", "领取旅行礼物", failure);
                    return;
                }
            }
        }

        if travel_state.as_deref() != Some("idle") {
            // 非 idle 且非 traveling/arrived 的未知状态不猜测，留待下轮。
            return;
        }
        if daily_limit == Some(true) {
            self.note_normal("travel", "Buddy 旅行", "今日旅行名额已用完".to_owned());
            return;
        }

        let config = match self.get_json("/buddy/travel/config").await {
            Ok(config) => config,
            Err(failure) => {
                self.note_call_failure("travel", "Buddy 旅行", "查询旅行地点", failure);
                return;
            }
        };
        let Some(location_id) = first_location_id(&config) else {
            self.note_failure_by_severity(
                "travel",
                "Buddy 旅行",
                "查询旅行地点失败：服务端未返回可用地点".to_owned(),
                false,
            );
            return;
        };

        let payload = serde_json::json!({ "location_id": location_id });
        match self.post_json("/buddy/travel/depart", payload).await {
            Ok(body) => {
                let place = dig(&body, "location")
                    .and_then(|location| text_of(dig(location, "name")))
                    .or_else(|| location.clone())
                    .unwrap_or_else(|| "目的地".to_owned());
                let duration = depart_duration_label(&body);
                self.note_ok(
                    "travel",
                    "Buddy 旅行",
                    format!("派 Buddy 去{place}{duration}"),
                );
            }
            Err(failure) => {
                self.note_call_failure("travel", "Buddy 旅行", "派出 Buddy", failure);
            }
        }
    }
}

fn body_reward_credit(body: &serde_json::Value) -> Option<rust_decimal::Decimal> {
    super::decimal_of(dig(body, "reward_credit"))
}

fn first_location_id(config: &serde_json::Value) -> Option<serde_json::Value> {
    let locations = dig(config, "locations")?.as_array()?;
    let first = locations.first()?;
    first.get("id").filter(|id| !id.is_null()).cloned()
}

/// 出发时长文案；服务端偶尔回 0，展示成「0 小时后回」会误导，直接省略。
fn depart_duration_label(body: &serde_json::Value) -> String {
    u32_of(dig(body, "duration_hours"))
        .or_else(|| {
            dig(body, "location").and_then(|location| u32_of(dig(location, "duration_hours")))
        })
        .filter(|hours| *hours > 0)
        .map(|hours| format!("（{hours} 小时后回）"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn location_id_requires_a_non_null_first_entry() {
        let config = json!({"data": {"locations": [{"id": "loc-1", "name": "海边"}]}});
        assert_eq!(first_location_id(&config), Some(json!("loc-1")));

        assert_eq!(first_location_id(&json!({"locations": []})), None);
        assert_eq!(
            first_location_id(&json!({"locations": [{"name": "海边"}]})),
            None
        );
        assert_eq!(first_location_id(&json!({})), None);
    }

    #[test]
    fn reward_credit_reads_wrapped_values() {
        assert_eq!(
            body_reward_credit(&json!({"data": {"reward_credit": 25}}))
                .map(super::super::format_decimal),
            Some("25".to_owned())
        );
        assert_eq!(body_reward_credit(&json!({"data": {}})), None);
    }

    #[test]
    fn depart_duration_labels_omit_zero_hours() {
        assert_eq!(
            depart_duration_label(&json!({"duration_hours": 3})),
            "（3 小时后回）"
        );
        assert_eq!(
            depart_duration_label(&json!({"location": {"duration_hours": 1}})),
            "（1 小时后回）"
        );
        assert_eq!(depart_duration_label(&json!({"duration_hours": 0})), "");
        assert_eq!(depart_duration_label(&json!({})), "");
    }
}
