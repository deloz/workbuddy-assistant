//! 成长中心任务：先接单（进度从接单开始计），再领已完成任务的奖励。

use serde_json::Value;

use super::{GrowthRun, TASK_BATCH_SIZE, dig, text_of, u32_of};

impl GrowthRun<'_> {
    pub(super) async fn run_tasks(&mut self) {
        if !self.budget_left() {
            self.note_skipped("tasks", "每日任务", "时间预算耗尽，任务模块跳过");
            return;
        }

        let body = match self.get_json("/tasks").await {
            Ok(body) => body,
            Err(failure) => {
                self.note_call_failure("tasks", "每日任务", "查询任务列表", &failure);
                return;
            }
        };
        if !super::api_success(&body) {
            let detail = super::business_detail(&body);
            self.note_failure_by_severity(
                "tasks",
                "每日任务",
                format!("查询任务列表失败：{detail}"),
                true,
            );
            return;
        }

        let tasks = dig(&body, "tasks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        self.overview.tasks_available = Some(count_status(&tasks, "not_accepted"));
        self.overview.tasks_in_progress =
            Some(count_status(&tasks, "accepted") + count_status(&tasks, "in_progress"));
        self.overview.tasks_claimable = Some(count_status(&tasks, "completed"));

        self.accept_pending(&tasks).await;
        self.claim_completed(&tasks).await;
    }

    async fn accept_pending(&mut self, tasks: &[Value]) {
        let pending: Vec<String> = tasks
            .iter()
            .filter(|task| {
                !task.get("locked").and_then(Value::as_bool).unwrap_or(false)
                    && text_of(task.get("accept_status")).as_deref() == Some("not_accepted")
            })
            .filter_map(|task| text_of(task.get("task_code")))
            .collect();
        if pending.is_empty() {
            return;
        }

        for batch in pending.chunks(TASK_BATCH_SIZE) {
            if !self.budget_left() {
                self.note_skipped("tasks", "每日任务", "时间预算耗尽，剩余任务下次再接单");
                return;
            }
            let payload = serde_json::json!({ "task_codes": batch });
            match self.post_json("/tasks/accept", payload).await {
                Ok(body) => self.note_accept_results(tasks, batch, &body),
                Err(failure) => {
                    self.note_call_failure("tasks", "每日任务", "领取任务", &failure);
                    return;
                }
            }
        }
    }

    fn note_accept_results(&mut self, tasks: &[Value], batch: &[String], body: &Value) {
        let results = dig(body, "results").and_then(Value::as_array);
        match results {
            Some(results) => {
                for result in results {
                    let code = text_of(result.get("task_code"));
                    let title = code
                        .as_deref()
                        .map_or_else(|| "未知任务".to_owned(), |code| task_title(tasks, code));
                    if text_of(result.get("status")).as_deref() == Some("error") {
                        let detail = text_of(result.get("message"))
                            .unwrap_or_else(|| "服务端未说明原因".to_owned());
                        self.note_failure_by_severity(
                            "tasks",
                            "每日任务",
                            format!("领取任务「{title}」失败：{detail}"),
                            true,
                        );
                    } else {
                        self.note_ok(
                            "tasks",
                            "每日任务",
                            format!("领取任务「{title}」（进度开始计）"),
                        );
                    }
                }
            }
            None if super::api_success(body) => {
                for code in batch {
                    self.note_ok(
                        "tasks",
                        "每日任务",
                        format!("领取任务「{}」（进度开始计）", task_title(tasks, code)),
                    );
                }
            }
            None => {
                let detail = super::business_detail(body);
                self.note_failure_by_severity(
                    "tasks",
                    "每日任务",
                    format!("领取任务失败：{detail}"),
                    true,
                );
            }
        }
    }

    async fn claim_completed(&mut self, tasks: &[Value]) {
        for task in tasks {
            if task.get("locked").and_then(Value::as_bool).unwrap_or(false)
                || text_of(task.get("accept_status")).as_deref() != Some("completed")
            {
                continue;
            }
            let Some(code) = text_of(task.get("task_code")) else {
                continue;
            };
            if !self.budget_left() {
                self.note_skipped("tasks", "每日任务", "时间预算耗尽，剩余任务奖下次再领");
                return;
            }
            let title = task_title(tasks, &code);
            match self
                .post_json(&format!("/tasks/{code}/claim"), serde_json::json!({}))
                .await
            {
                Ok(body) if super::api_success(&body) => {
                    if super::bool_of(dig(&body, "already_claimed")) == Some(true) {
                        self.note_normal("tasks", "每日任务", format!("任务奖「{title}」已领过"));
                        continue;
                    }
                    // 优先用服务端实发数值，挖不到才回落到列表里的活动配置值。
                    let credit = super::decimal_of(dig(&body, "credit"))
                        .or_else(|| super::decimal_of(task.get("reward_credit")))
                        .unwrap_or_default();
                    let energy = u32_of(dig(&body, "energy"))
                        .or_else(|| u32_of(task.get("reward_energy")))
                        .unwrap_or_default();
                    self.credits_gained += credit;
                    self.note_ok(
                        "tasks",
                        "每日任务",
                        format!(
                            "领任务奖「{title}」+credit{}+energy{}",
                            super::format_decimal(credit),
                            energy
                        ),
                    );
                }
                Ok(body) => {
                    let detail = super::business_detail(&body);
                    self.note_failure_by_severity(
                        "tasks",
                        "每日任务",
                        format!("领任务奖「{title}」失败：{detail}"),
                        false,
                    );
                }
                Err(failure) => {
                    self.note_call_failure("tasks", "每日任务", "领取任务奖", &failure);
                    return;
                }
            }
        }
    }
}

fn count_status(tasks: &[Value], status: &str) -> u32 {
    let count = tasks
        .iter()
        .filter(|task| {
            !task.get("locked").and_then(Value::as_bool).unwrap_or(false)
                && text_of(task.get("accept_status")).as_deref() == Some(status)
        })
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn task_title(tasks: &[Value], code: &str) -> String {
    tasks
        .iter()
        .find(|task| text_of(task.get("task_code")).as_deref() == Some(code))
        .and_then(|task| text_of(task.get("title")))
        .unwrap_or_else(|| code.to_owned())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn fixture() -> Vec<Value> {
        vec![
            json!({"task_code": "a", "title": "每日一问", "accept_status": "not_accepted", "locked": false}),
            json!({"task_code": "b", "title": "分享任务", "accept_status": "completed", "locked": false}),
            json!({"task_code": "c", "title": "锁住的", "accept_status": "not_accepted", "locked": true}),
            json!({"task_code": "d", "title": "进行中", "accept_status": "in_progress", "locked": false}),
        ]
    }

    #[test]
    fn counts_ignore_locked_and_split_by_status() {
        let tasks = fixture();

        assert_eq!(count_status(&tasks, "not_accepted"), 1);
        assert_eq!(count_status(&tasks, "in_progress"), 1);
        assert_eq!(count_status(&tasks, "completed"), 1);
        assert_eq!(count_status(&tasks, "claimed"), 0);
    }

    #[test]
    fn task_title_falls_back_to_the_code() {
        let tasks = fixture();

        assert_eq!(task_title(&tasks, "a"), "每日一问");
        assert_eq!(task_title(&tasks, "missing"), "missing");
    }
}
