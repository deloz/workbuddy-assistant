//! 连签相关：用补登卡补救当月断登，再兑换已解锁的连登奖励。
//!
//! 补登日期的口径对齐官方成长中心：只补当月、今天之前、活动上线之后且活跃分数为
//! 零的日期；`makeup_dates` 是「已补登」记录，不能当作待办清单。

use std::collections::{BTreeMap, HashSet};

use chrono::{Datelike, NaiveDate};
use serde_json::Value;

use super::{GrowthRun, dig, text_of, u32_of};

/// 补登卡每轮最多用掉的数量。
const MAKEUP_MAX_PER_RUN: usize = 1;
/// 活动最早可补的日期：早于它的零分日期不属于断登。
const ACTIVITY_FLOOR: &str = "2026-06-17";
/// 连登兑换三档：(档位标识, summary 状态字段前缀, 展示名, 天数)。
const REDEEM_TIERS: [(&str, &str, &str, u32); 3] = [
    ("7d", "starter", "入门", 7),
    ("14d", "advanced", "进阶", 14),
    ("28d", "legendary", "巅峰", 28),
];

impl GrowthRun<'_> {
    #[allow(
        clippy::too_many_lines,
        reason = "补登是一条线性流程（查状态→查日历→逐日补登），拆开会割断步骤记账"
    )]
    pub(super) async fn run_makeup(&mut self) {
        if !self.budget_left() {
            self.note_skipped("makeup", "断登补登", "时间预算耗尽，补登跳过");
            return;
        }

        let streak = match self.get_json("/streak").await {
            Ok(streak) => streak,
            Err(failure) => {
                self.note_call_failure("makeup", "断登补登", "查询连登状态", &failure);
                return;
            }
        };
        if !super::api_success(&streak) {
            let detail = super::business_detail(&streak);
            self.note_failure_by_severity(
                "makeup",
                "断登补登",
                format!("查询连登状态失败：{detail}"),
                true,
            );
            return;
        }

        self.read_streak_summary(&streak);
        let cards = makeup_cards(&streak);
        self.overview.makeup_cards = Some(cards);
        if cards == 0 {
            return;
        }

        let heatmap = match self.get_json("/heatmap").await {
            Ok(heatmap) => heatmap,
            Err(failure) => {
                self.note_call_failure("makeup", "断登补登", "查询补登日历", &failure);
                return;
            }
        };
        let candidates = match makeup_candidates(&streak, &heatmap) {
            Ok(candidates) => candidates,
            Err(reason) => {
                self.note_failure_by_severity(
                    "makeup",
                    "断登补登",
                    format!("补登日历不可用（{reason}），本轮不补登"),
                    true,
                );
                return;
            }
        };

        let limit = (cards as usize).min(MAKEUP_MAX_PER_RUN);
        let mut used = 0_u32;
        for date in candidates.iter().take(limit) {
            if !self.budget_left() {
                self.note_skipped("makeup", "断登补登", "时间预算耗尽，剩余补登下次再做");
                return;
            }
            let payload = serde_json::json!({ "target_date": date });
            match self.post_json("/makeup-cards/use", payload).await {
                Ok(body) if super::api_success(&body) => {
                    used += 1;
                    self.streak_stale = true;
                    // 优先用服务端给的余额，接口没给才按本地递减兜底。
                    let local_left = cards.saturating_sub(used);
                    let left = makeup_cards_opt(&body)
                        .unwrap_or(local_left)
                        .min(local_left);
                    self.overview.makeup_cards = Some(left);
                    self.note_ok(
                        "makeup",
                        "断登补登",
                        format!("补登 {date}（剩 {left} 张卡）"),
                    );
                }
                Ok(body) => {
                    let message = super::body_message(&body);
                    if is_no_makeup_needed(&message) {
                        self.note_normal(
                            "makeup",
                            "断登补登",
                            format!("{date} 已活跃或已补登，无需再次补登"),
                        );
                    } else {
                        let detail = super::business_detail(&body);
                        self.note_failure_by_severity(
                            "makeup",
                            "断登补登",
                            format!("补登 {date} 失败：{detail}"),
                            true,
                        );
                    }
                    break;
                }
                Err(failure) => {
                    self.note_call_failure("makeup", "断登补登", "补登", &failure);
                    return;
                }
            }
        }

        let used_count = used as usize;
        if used_count > 0 {
            let remaining = self.overview.makeup_cards.unwrap_or(0);
            if candidates.len() > used_count && remaining > 0 {
                self.note_normal(
                    "makeup",
                    "断登补登",
                    format!(
                        "另有 {} 天可补、剩 {remaining} 张卡，下轮继续",
                        candidates.len() - used_count
                    ),
                );
            }
        }
    }

    pub(super) async fn run_redeem(&mut self) {
        if !self.budget_left() {
            self.note_skipped("redeem", "连登兑换", "时间预算耗尽，连登兑换跳过");
            return;
        }

        let summary = match self.get_json("/redeem/summary").await {
            Ok(summary) => summary,
            Err(failure) => {
                self.note_call_failure("redeem", "连登兑换", "查询连登兑换", &failure);
                return;
            }
        };
        if !super::api_success(&summary) {
            let detail = super::business_detail(&summary);
            self.note_failure_by_severity(
                "redeem",
                "连登兑换",
                format!("查询连登兑换失败：{detail}"),
                true,
            );
            return;
        }

        for (tier, status_key, label, days) in REDEEM_TIERS {
            let status = text_of(dig(&summary, &format!("{status_key}_status")));
            record_redeem_status(&mut self.overview, tier, status.as_deref());
            // 字段缺失或已领/未解锁都跳过；接口改版时不对三档无脑发起写入。
            let Some(status) = status else { continue };
            if status == "claimed" || status == "locked" {
                continue;
            }
            if !self.budget_left() {
                self.note_skipped("redeem", "连登兑换", "时间预算耗尽，剩余连登兑换下次再领");
                return;
            }

            let payload =
                serde_json::json!({ "tier": tier, "client_token": super::client_token() });
            let first = self.post_json("/redeem", payload).await;
            let outcome = match first {
                // 档位标识被判为未知时退回天数再试一次：这类 400 发生在参数校验阶段，
                // 服务端没有兑换任何东西，重试不会重复领取。
                Err(super::CallFailure::Http { status, body })
                    if is_unknown_tier(status, &body) =>
                {
                    let payload =
                        serde_json::json!({ "tier": days, "client_token": super::client_token() });
                    self.post_json("/redeem", payload).await
                }
                other => other,
            };

            match outcome {
                Ok(body) if super::api_success(&body) => {
                    let credit =
                        super::decimal_of(dig(&body, "credit_granted")).unwrap_or_default();
                    self.credits_gained += credit;
                    self.note_ok(
                        "redeem",
                        "连登兑换",
                        format!("连登兑换「{label}」{}", redeem_reward_desc(&body, tier)),
                    );
                }
                Ok(body) => {
                    let detail = super::business_detail(&body);
                    self.note_failure_by_severity(
                        "redeem",
                        "连登兑换",
                        format!("连登兑换「{label}」失败：{detail}"),
                        false,
                    );
                }
                Err(super::CallFailure::Http { status, body }) if is_tier_locked(status, &body) => {
                    // 「连登天数不足」是常态，不能当成权限拒绝中止整轮。
                    self.note_normal(
                        "redeem",
                        "连登兑换",
                        format!("连登兑换「{label}」未解锁（连登天数不足）"),
                    );
                }
                Err(failure) => {
                    self.note_call_failure("redeem", "连登兑换", "连登兑换", &failure);
                }
            }
        }
    }

    /// 连签展示值：补登成功后必须重新取，其余情况复用本轮已取的数据。
    pub(super) async fn refresh_streak_if_stale(&mut self) {
        if self.streak_stale {
            if !self.budget_left() {
                return;
            }
            if let Ok(body) = self.get_json("/streak").await {
                self.read_streak_summary(&body);
            }
        }
    }

    fn read_streak_summary(&mut self, body: &Value) {
        let days = dig(body, "streak")
            .and_then(|streak| u32_of(dig(streak, "days")))
            .or_else(|| u32_of(dig(body, "streak_days")));
        if days.is_some() {
            self.streak_days = days;
        }
        if let Some(cards) = makeup_cards_opt(body) {
            self.overview.makeup_cards = Some(cards);
        }
    }
}

fn record_redeem_status(overview: &mut super::GrowthOverview, tier: &str, status: Option<&str>) {
    let status = status.map(str::to_owned);
    match tier {
        "7d" => overview.redeem_starter = status,
        "14d" => overview.redeem_advanced = status,
        "28d" => overview.redeem_legendary = status,
        _ => {}
    }
}

/// 补登卡余额：兼容 `{"makeup_cards": {"balance": 2}}` 与 `{"makeup_cards": 2}`。
fn makeup_cards_opt(body: &Value) -> Option<u32> {
    let cards = dig(body, "makeup_cards")?;
    match cards {
        Value::Object(_) => u32_of(dig(cards, "balance")),
        other => u32_of(Some(other)),
    }
}

fn makeup_cards(body: &Value) -> u32 {
    makeup_cards_opt(body).unwrap_or(0)
}

/// 从活跃日历挑出可补的断登日期，最近的优先。
pub(super) fn makeup_candidates(streak: &Value, heatmap: &Value) -> Result<Vec<String>, String> {
    let today =
        parse_date(text_of(dig(heatmap, "today").and_then(|today| dig(today, "date"))).as_deref())
            .map_err(|()| "活跃日历缺少服务端日期".to_owned())?;
    let launch = parse_date(text_of(dig(streak, "launch_date")).as_deref())
        .map_err(|()| "连登状态缺少活动上线日期".to_owned())?;
    let floor = parse_date(Some(ACTIVITY_FLOOR)).expect("活动起始日应为合法日期");
    let start = today
        .with_day(1)
        .expect("当月第一天应存在")
        .max(launch)
        .max(floor);

    let made_up = dig(streak, "streak")
        .and_then(|streak| streak.get("makeup_dates"))
        .and_then(Value::as_array)
        .map(|dates| {
            dates
                .iter()
                .map(|date| parse_date(date.as_str()))
                .collect::<Result<HashSet<_>, _>>()
        })
        .transpose()
        .map_err(|()| "已补登记录格式无效".to_owned())?
        .unwrap_or_default();

    let cells = dig(heatmap, "cells")
        .and_then(Value::as_array)
        .ok_or_else(|| "活跃日历缺少日期列表".to_owned())?;
    let mut scores: BTreeMap<NaiveDate, f64> = BTreeMap::new();
    for cell in cells {
        let day = parse_date(cell.get("date").and_then(Value::as_str))
            .map_err(|()| "活跃日历记录格式无效".to_owned())?;
        let score = cell
            .get("score")
            .and_then(Value::as_f64)
            .filter(|score| score.is_finite() && *score >= 0.0)
            .ok_or_else(|| "活跃日历分数无效".to_owned())?;
        #[allow(
            clippy::float_cmp,
            reason = "同一日期出现两条记录要逐位比较，容忍误差会把真实冲突当成重复"
        )]
        let conflicted = scores
            .insert(day, score)
            .is_some_and(|previous| previous != score);
        if conflicted {
            return Err("活跃日历同一日期的记录冲突".to_owned());
        }
    }

    let mut candidates: Vec<String> = scores
        .into_iter()
        .filter(|(day, score)| {
            *day >= start && *day < today && *score == 0.0 && !made_up.contains(day)
        })
        .map(|(day, _)| day.to_string())
        .collect();
    candidates.reverse();
    Ok(candidates)
}

fn parse_date(value: Option<&str>) -> Result<NaiveDate, ()> {
    let value = value.ok_or(())?;
    NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").map_err(|_| ())
}

/// 「天数不足」的 403 是未解锁档位的常态，不是权限问题。
fn is_tier_locked(status: reqwest::StatusCode, body: &Value) -> bool {
    status == reqwest::StatusCode::FORBIDDEN && super::body_message(body).contains("不足")
}

/// 「tier 不认识」用于判断是否退回天数重试。
fn is_unknown_tier(status: reqwest::StatusCode, body: &Value) -> bool {
    if status != reqwest::StatusCode::BAD_REQUEST {
        return false;
    }
    let message = super::body_message(body).to_lowercase();
    message.contains("tier")
        && (message.contains("unknown")
            || message.contains("unsupported")
            || message.contains("invalid"))
}

/// 查询后被客户端或另一轮补上，属于正常竞争。
fn is_no_makeup_needed(message: &str) -> bool {
    message.trim().to_lowercase() == "date is not broken, no makeup needed"
}

/// 兑换成功的奖励描述：优先用服务端实发明细，挖不到才回落到官方文案。
fn redeem_reward_desc(body: &Value, tier: &str) -> String {
    let mut bits = Vec::new();
    for (key, suffix) in [
        ("credit_granted", "积分"),
        ("energy_granted", "能量"),
        ("cards_granted", "补登卡"),
        ("chances_granted", "次抽奖"),
    ] {
        if let Some(value) = u32_of(dig(body, key)).filter(|value| *value > 0) {
            bits.push(format!("+{value} {suffix}"));
        }
    }
    if !bits.is_empty() {
        return format!("（{}）", bits.join(" "));
    }
    let fallback = match tier {
        "7d" => "+2 能量 +1 补登卡 +1 次抽奖",
        "14d" => "+50 积分 +3 能量 +1 补登卡 +1 次抽奖",
        "28d" => "+150 积分 +5 能量 +1 补登卡 +1 次抽奖",
        _ => "奖励已到账",
    };
    format!("（{fallback}）")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn streak_fixture() -> Value {
        json!({
            "launch_date": "2026-06-17",
            "streak": {"days": 1, "makeup_dates": ["2026-09-01"]},
            "makeup_cards": {"balance": 2}
        })
    }

    fn heatmap_fixture() -> Value {
        json!({
            "today": {"date": "2026-09-30"},
            "cells": [
                {"date": "2026-09-01", "score": 10},
                {"date": "2026-09-28", "score": 0},
                {"date": "2026-09-29", "score": 12},
                {"date": "2026-09-30", "score": 0},
                {"date": "2026-08-30", "score": 0}
            ]
        })
    }

    #[test]
    fn candidates_skip_today_other_months_and_made_up_dates() {
        let candidates = makeup_candidates(&streak_fixture(), &heatmap_fixture()).unwrap();

        // 9-30 是今天、8-30 属于上月、9-01 已补登，只剩 9-28。
        assert_eq!(candidates, vec!["2026-09-28".to_owned()]);
    }

    #[test]
    fn candidates_prefer_the_most_recent_break() {
        let heatmap = json!({
            "today": {"date": "2026-09-30"},
            "cells": [
                {"date": "2026-09-10", "score": 0},
                {"date": "2026-09-20", "score": 0},
                {"date": "2026-09-25", "score": 0}
            ]
        });

        let candidates = makeup_candidates(&streak_fixture(), &heatmap).unwrap();

        assert_eq!(
            candidates,
            vec![
                "2026-09-25".to_owned(),
                "2026-09-20".to_owned(),
                "2026-09-10".to_owned()
            ]
        );
    }

    #[test]
    fn candidates_respect_the_activity_floor() {
        let heatmap = json!({
            "today": {"date": "2026-06-30"},
            "cells": [
                {"date": "2026-06-16", "score": 0},
                {"date": "2026-06-17", "score": 0}
            ]
        });
        let streak = json!({
            "launch_date": "2026-06-01",
            "streak": {"days": 1, "makeup_dates": []},
            "makeup_cards": {"balance": 1}
        });

        let candidates = makeup_candidates(&streak, &heatmap).unwrap();

        assert_eq!(candidates, vec!["2026-06-17".to_owned()]);
    }

    #[test]
    fn candidates_stop_on_broken_calendars() {
        assert!(makeup_candidates(&streak_fixture(), &json!({"cells": []})).is_err());
        assert!(
            makeup_candidates(&streak_fixture(), &json!({"today": {"date": "2026-09-30"}}))
                .is_err()
        );
        assert!(
            makeup_candidates(
                &streak_fixture(),
                &json!({"today": {"date": "2026-09-30"}, "cells": [{"date": "bad", "score": 0}]})
            )
            .is_err()
        );
        assert!(
            makeup_candidates(
                &streak_fixture(),
                &json!({"today": {"date": "2026-09-30"}, "cells": [{"date": "2026-09-10", "score": -1}]})
            )
            .is_err()
        );
    }

    #[test]
    fn conflicting_calendar_records_are_rejected() {
        let heatmap = json!({
            "today": {"date": "2026-09-30"},
            "cells": [
                {"date": "2026-09-10", "score": 0},
                {"date": "2026-09-10", "score": 5}
            ]
        });

        assert!(makeup_candidates(&streak_fixture(), &heatmap).is_err());
    }

    #[test]
    fn makeup_card_balance_accepts_both_shapes() {
        assert_eq!(makeup_cards(&json!({"makeup_cards": {"balance": 3}})), 3);
        assert_eq!(makeup_cards(&json!({"makeup_cards": 2})), 2);
        assert_eq!(makeup_cards(&json!({"makeup_cards": {"balance": "1"}})), 1);
        assert_eq!(makeup_cards(&json!({})), 0);
    }

    #[test]
    fn tier_locked_and_unknown_tier_are_classified_separately() {
        let locked = json!({"msg": "连续登录天数不足"});
        assert!(is_tier_locked(reqwest::StatusCode::FORBIDDEN, &locked));
        assert!(!is_unknown_tier(reqwest::StatusCode::FORBIDDEN, &locked));

        let unknown = json!({"msg": "unknown tier"});
        assert!(is_unknown_tier(reqwest::StatusCode::BAD_REQUEST, &unknown));
        assert!(!is_tier_locked(reqwest::StatusCode::BAD_REQUEST, &unknown));

        assert!(!is_unknown_tier(
            reqwest::StatusCode::BAD_REQUEST,
            &json!({"msg": "invalid request"})
        ));
    }

    #[test]
    fn no_makeup_needed_matches_the_exact_server_message() {
        assert!(is_no_makeup_needed("date is not broken, no makeup needed"));
        assert!(is_no_makeup_needed(
            "  DATE IS NOT BROKEN, NO MAKEUP NEEDED  "
        ));
        assert!(!is_no_makeup_needed("insufficient makeup cards"));
        assert!(!is_no_makeup_needed(""));
    }

    #[test]
    fn redeem_description_prefers_server_values() {
        let body = json!({
            "data": {
                "credit_granted": 50,
                "energy_granted": 3,
                "cards_granted": 1,
                "chances_granted": 1
            }
        });
        assert_eq!(
            redeem_reward_desc(&body, "14d"),
            "（+50 积分 +3 能量 +1 补登卡 +1 次抽奖）"
        );

        assert_eq!(
            redeem_reward_desc(&json!({"data": {}}), "14d"),
            "（+50 积分 +3 能量 +1 补登卡 +1 次抽奖）"
        );
        assert_eq!(
            redeem_reward_desc(&json!({"data": {}}), "unknown"),
            "（奖励已到账）"
        );
    }
}
