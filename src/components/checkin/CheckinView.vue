<template>
  <section
    class="view-panel"
    aria-labelledby="checkin-title"
    aria-live="polite"
    :aria-busy="loading"
  >
    <div class="view-heading">
      <div>
        <p class="section-kicker">每日任务</p>
        <h1 id="checkin-title">签到状态</h1>
      </div>
      <button class="button button--secondary" type="button" :disabled="loading || !account" @click="$emit('run')">
        {{ loading ? "检查中…" : "检查当前账号" }}
      </button>
    </div>

    <div class="scheduler-bar">
      <div><strong>自动签到已开启</strong><span>应用运行期间，每天本地时间 00:00 会检查全部账号。</span></div>
      <div class="scheduler-next"><span>下一次计划</span><strong>{{ formatDate(nextAutoCheckinAt) }}</strong></div>
    </div>

    <section v-if="taskData" class="checkin-task" aria-labelledby="checkin-task-title">
      <div class="checkin-task__main">
        <div class="checkin-task__heading">
          <p>当前签到任务</p>
          <h2 id="checkin-task-title">{{ taskTitle }}</h2>
          <span>{{ taskPeriod }}</span>
        </div>
        <div class="checkin-task__reward" :aria-label="`每日可领 ${formatAmount(taskData.todayCredit)} 通用积分`">
          <span>每日可领</span>
          <div><strong>{{ formatAmount(taskData.todayCredit) }}</strong><span>通用积分</span></div>
        </div>
      </div>

      <dl class="checkin-task__stats">
        <div>
          <dt>今日状态</dt>
          <dd :class="taskStatusClass(taskData)">{{ taskStatusLabel(taskData) }}</dd>
        </div>
        <div>
          <dt>连续签到</dt>
          <dd>{{ formatAmount(taskData.streakDays) }} 天</dd>
        </div>
        <div>
          <dt>累计领取</dt>
          <dd>{{ formatAmount(taskData.totalCredits) }} 积分</dd>
        </div>
      </dl>

      <div class="checkin-task__period">
        <span>活动时间</span>
        <strong>{{ formatDateRange(taskData.startAt, taskData.endAt) }}</strong>
      </div>
    </section>

    <section v-if="account" class="checkin-history" aria-labelledby="checkin-history-title">
      <div class="table-heading checkin-history__heading">
        <h2 id="checkin-history-title">签到历史</h2>
        <span>当前账号最近 90 天</span>
      </div>
      <div v-if="checkinHistory.length" class="table-wrap">
        <table class="checkin-history__table">
          <thead>
            <tr>
              <th scope="col">日期</th>
              <th scope="col">任务</th>
              <th scope="col">结果</th>
              <th scope="col">获得积分</th>
              <th scope="col">连续签到</th>
              <th scope="col">检查时间</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="entry in checkinHistory" :key="`${entry.localDate}-${entry.checkedAt}`">
              <th scope="row"><time :datetime="entry.localDate">{{ formatHistoryDate(entry.localDate) }}</time></th>
              <td>{{ entry.activityName || "每日签到" }}</td>
              <td :class="historyStatusClass(entry.status)">
                {{ historyStatusLabel(entry.status) }}
                <small v-if="entry.error">{{ entry.error }}</small>
              </td>
              <td :class="{ 'checkin-history__credit': entry.creditEarned }">{{ historyCredit(entry) }}</td>
              <td>{{ historyStreak(entry) }}</td>
              <td>{{ formatDate(entry.checkedAt) }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <div v-else class="inline-empty inline-empty--small checkin-history__empty">
        <strong>还没有签到历史</strong>
        <span>历史会从本版本开始记录，每个账号每天最多保留一条。</span>
      </div>
    </section>
  </section>
</template>

<script setup>
import { computed } from "vue";

const props = defineProps({
  account: { type: Object, default: null },
  nextAutoCheckinAt: { type: String, default: null },
  loading: { type: Boolean, default: false },
});
defineEmits(["run"]);

const amountFormatter = new Intl.NumberFormat("zh-CN", {
  maximumFractionDigits: 2,
});
const taskData = computed(() => props.account?.checkin?.data ?? null);
const checkinHistory = computed(() => props.account?.checkinHistory ?? []);
const taskTitle = computed(() => cleanText(taskData.value?.activityName) || "每日签到");
const taskPeriod = computed(() => {
  const themeName = cleanText(taskData.value?.themeName);
  const season = taskData.value?.season;
  const seasonLabel = season === null || season === undefined ? "" : `第 ${season} 期`;
  return [themeName, seasonLabel].filter(Boolean).join(" · ") || "签到活动";
});

function cleanText(value) {
  return typeof value === "string" ? value.trim() : "";
}

function formatAmount(value) {
  if (value === null || value === undefined || value === "") return "—";
  const numericValue = Number(value);
  return Number.isFinite(numericValue) ? amountFormatter.format(numericValue) : String(value);
}

function parseDate(value) {
  if (!value) return null;
  const text = String(value).trim();
  const epochValue = /^\d{10,13}$/.test(text)
    ? Number(text) * (text.length === 10 ? 1000 : 1)
    : text.replace(/^(\d{4}-\d{2}-\d{2}) (\d{2}:\d{2}:\d{2})$/, "$1T$2");
  const parsed = new Date(epochValue);
  return Number.isNaN(parsed.getTime()) ? null : parsed;
}

function formatDate(value) {
  if (!value) return "等待登录";
  const parsed = parseDate(value);
  if (!parsed) return String(value);
  return new Intl.DateTimeFormat("zh-CN", { dateStyle: "medium", timeStyle: "short" }).format(parsed);
}

function formatDateRange(startAt, endAt) {
  const formatter = new Intl.DateTimeFormat("zh-CN", { dateStyle: "medium" });
  const start = parseDate(startAt);
  const end = parseDate(endAt);
  if (start && end) return `${formatter.format(start)} 至 ${formatter.format(end)}`;
  if (start) return `${formatter.format(start)} 起`;
  if (end) return `至 ${formatter.format(end)}`;
  return "未提供";
}

function taskStatusLabel(task) {
  if (!task.active) return "活动未开放";
  return task.todayCheckedIn ? "今日已完成" : "今日待签到";
}

function taskStatusClass(task) {
  if (!task.active) return "checkin-task__status--muted";
  return task.todayCheckedIn ? "checkin-task__status--ok" : "";
}

function formatHistoryDate(value) {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(String(value));
  if (!match) return String(value);
  return `${match[1]}年${Number(match[2])}月${Number(match[3])}日`;
}

function historyStatusLabel(status) {
  if (status === "claimedNow") return "领取成功";
  if (status === "alreadyClaimed") return "当日已领取";
  if (status === "inactive") return "活动未开放";
  if (status === "error") return "检查失败";
  return "已检查";
}

function historyStatusClass(status) {
  if (["claimedNow", "alreadyClaimed"].includes(status)) return "status-cell status-cell--ok";
  if (status === "error") return "status-cell status-cell--danger";
  return "status-cell";
}

function historyCredit(entry) {
  return entry.creditEarned ? `+${formatAmount(entry.creditEarned)}` : "—";
}

function historyStreak(entry) {
  return entry.streakDays === null || entry.streakDays === undefined
    ? "—"
    : `${formatAmount(entry.streakDays)} 天`;
}
</script>
