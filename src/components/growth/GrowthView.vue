<template>
  <section
    class="view-panel"
    aria-labelledby="growth-title"
    aria-live="polite"
    :aria-busy="loading"
  >
    <div class="view-heading">
      <div>
        <p class="section-kicker">成长中心</p>
        <h1 id="growth-title">成长中心</h1>
      </div>
      <div class="view-heading__actions">
        <span v-if="snapshot" class="updated-at">最近运行 {{ formatDate(snapshot.ranAt) }}</span>
        <button
          class="button button--secondary"
          type="button"
          :disabled="loading || !account"
          @click="$emit('run')"
        >
          {{ loading ? "运行中…" : "运行当前账号" }}
        </button>
        <button
          class="button button--primary"
          type="button"
          :disabled="loading || !hasAccounts"
          @click="$emit('run-all')"
        >
          运行全部账号
        </button>
      </div>
    </div>

    <div class="scheduler-bar">
      <div>
        <strong>{{ automationEnabled ? "成长中心自动化已开启" : "成长中心自动化已关闭" }}</strong>
        <span>
          {{
            automationEnabled
              ? "应用运行期间，每天 00:00 全量运行一次，并每 4 小时轮询补领；可在设置中关闭。"
              : "自动运行已暂停，手动运行仍然可用；可在设置中重新开启。"
          }}
        </span>
      </div>
      <div class="scheduler-next">
        <span>下一次自动运行</span>
        <strong>{{ formatDate(nextGrowthRunAt) }}</strong>
      </div>
    </div>

    <p v-if="error" class="growth-alert growth-alert--danger" role="status">
      {{ error }}
    </p>
    <p
      v-else-if="snapshot && snapshot.needsAttention"
      class="growth-alert growth-alert--danger"
      role="status"
    >
      最近一次运行有需要处理的问题，详情见下方逐项结果。
    </p>

    <template v-if="snapshot">
      <section class="growth-overview" aria-labelledby="growth-overview-title">
        <h2 id="growth-overview-title" class="visually-hidden">当前状态</h2>
        <div class="growth-overview__grid">
          <div>
            <span>Buddy 旅行</span>
            <strong>{{ travelLabel }}</strong>
            <small v-if="snapshot.overview?.travelLocation">{{ snapshot.overview.travelLocation }}</small>
          </div>
          <div>
            <span>能量</span>
            <strong>{{ formatAmount(snapshot.overview?.energy) }}</strong>
          </div>
          <div>
            <span>连签</span>
            <strong>{{ formatDays(snapshot.overview?.streakDays) }}</strong>
          </div>
          <div>
            <span>补登卡</span>
            <strong>{{ formatCount(snapshot.overview?.makeupCards) }}</strong>
          </div>
          <div>
            <span>抽奖机会</span>
            <strong>{{ formatCount(snapshot.overview?.lotteryChances) }}</strong>
          </div>
          <div>
            <span>任务（可接/进行/可领）</span>
            <strong>{{ taskCounts }}</strong>
          </div>
        </div>
        <div class="growth-overview__redeem">
          <span>连登兑换</span>
          <ul>
            <li v-for="tier in redeemTiers" :key="tier.key">
              {{ tier.label }}：{{ redeemLabel(snapshot.overview?.[tier.key]) }}
            </li>
          </ul>
        </div>
      </section>

      <section class="growth-steps" aria-labelledby="growth-steps-title">
        <div class="table-heading growth-steps__heading">
          <h2 id="growth-steps-title">最近一次运行结果</h2>
          <span>{{ triggerLabel(snapshot.trigger) }} · 本次获得 {{ formatGrowthCredits(snapshot.creditsGained) }}</span>
        </div>
        <ul class="growth-steps__list">
          <li v-for="(step, index) in snapshot.steps" :key="`${step.id}-${index}`">
            <span class="growth-step__label">{{ step.label }}</span>
            <span class="growth-step__detail">{{ step.detail }}</span>
            <span class="status-cell" :class="stepClass(step.status)">{{ stepLabel(step.status) }}</span>
          </li>
        </ul>
        <p v-if="!snapshot.steps?.length" class="inline-empty inline-empty--small">
          <strong>本次没有需要执行的动作</strong>
          <span>可能是活动未开放，或今日奖励已全部领取。</span>
        </p>
      </section>
    </template>

    <div v-else-if="loading" class="loading-line" role="status">
      <strong>正在运行成长中心…</strong>
      <span>会依次处理旅行、任务、补登、兑换与盲盒，请稍候。</span>
    </div>
    <div v-else-if="account" class="inline-empty">
      <strong>还没有运行记录</strong>
      <span>点击「运行当前账号」立即领取成长中心奖励，或等待下一次自动运行。</span>
    </div>
    <div v-else class="inline-empty">
      <strong>请先登录 WorkBuddy</strong>
      <span>登录后即可查看并运行成长中心。</span>
    </div>

    <section v-if="account" class="growth-history" aria-labelledby="growth-history-title">
      <div class="table-heading growth-history__heading">
        <h2 id="growth-history-title">运行历史</h2>
        <span>当前账号最近 90 次</span>
      </div>
      <div v-if="history.length" class="table-wrap">
        <table class="growth-history__table">
          <thead>
            <tr>
              <th scope="col">时间</th>
              <th scope="col">触发</th>
              <th scope="col">结果</th>
              <th scope="col">获得积分</th>
              <th scope="col">能量</th>
              <th scope="col">摘要</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="entry in history" :key="`${entry.ranAt}-${entry.trigger}`">
              <th scope="row"><time :datetime="entry.ranAt">{{ formatDate(entry.ranAt) }}</time></th>
              <td>{{ triggerLabel(entry.trigger) }}</td>
              <td :class="historyStatusClass(entry)">
                {{ historyStatusLabel(entry) }}
              </td>
              <td :class="{ 'growth-history__credit': hasCredits(entry) }">
                {{ formatGrowthCredits(entry.creditsGained) }}
              </td>
              <td>{{ formatAmount(entry.energy) }}</td>
              <td class="growth-history__report">{{ entry.report || "—" }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <div v-else class="inline-empty inline-empty--small growth-history__empty">
        <strong>还没有运行历史</strong>
        <span>历史会从本版本开始记录；轮询没有收获时不写入，避免淹没重要记录。</span>
      </div>
    </section>
  </section>
</template>

<script setup>
import { computed } from "vue";

import {
  formatGrowthCredits,
  growthStepTone,
  growthTriggerLabel,
} from "../../utils/appState";

const props = defineProps({
  account: { type: Object, default: null },
  hasAccounts: { type: Boolean, default: false },
  nextGrowthRunAt: { type: String, default: null },
  automationEnabled: { type: Boolean, default: true },
  loading: { type: Boolean, default: false },
});
defineEmits(["run", "run-all"]);

const amountFormatter = new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 2 });
const redeemTiers = [
  { key: "redeemStarter", label: "入门 7 天" },
  { key: "redeemAdvanced", label: "进阶 14 天" },
  { key: "redeemLegendary", label: "巅峰 28 天" },
];

const snapshot = computed(() => props.account?.growth?.data ?? null);
const history = computed(() => props.account?.growthHistory ?? []);
const error = computed(() => props.account?.growth?.error ?? null);
const travelLabel = computed(() => {
  const overview = snapshot.value?.overview;
  if (!overview?.travelState) return "—";
  if (overview.travelState === "traveling") {
    return overview.travelEta ? `旅行中（${overview.travelEta}）` : "旅行中";
  }
  if (overview.travelState === "arrived") return "已到达，待领取";
  if (overview.travelState === "idle") {
    return overview.travelDailyLimitReached ? "空闲（今日名额已用完）" : "空闲";
  }
  return overview.travelState;
});
const taskCounts = computed(() => {
  const overview = snapshot.value?.overview;
  const format = (value) => (value === null || value === undefined ? "—" : String(value));
  return `${format(overview?.tasksAvailable)} / ${format(overview?.tasksInProgress)} / ${format(overview?.tasksClaimable)}`;
});

function formatAmount(value) {
  if (value === null || value === undefined || value === "") return "—";
  const numericValue = Number(value);
  return Number.isFinite(numericValue) ? amountFormatter.format(numericValue) : String(value);
}

function formatCount(value) {
  return value === null || value === undefined ? "—" : String(value);
}

function formatDays(value) {
  return value === null || value === undefined ? "—" : `${value} 天`;
}

function formatDate(value) {
  if (!value) return "尚未计划";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return String(value);
  return new Intl.DateTimeFormat("zh-CN", { dateStyle: "medium", timeStyle: "short" }).format(parsed);
}

function triggerLabel(trigger) {
  return growthTriggerLabel(trigger);
}

function stepClass(status) {
  const tone = growthStepTone(status);
  if (tone === "ok") return "status-cell--ok";
  if (tone === "danger") return "status-cell--danger";
  return "";
}

function stepLabel(status) {
  if (status === "ok") return "已领取";
  if (status === "failed") return "失败";
  if (status === "skipped") return "已跳过";
  return "正常";
}

function redeemLabel(status) {
  if (status === "claimed") return "已兑换";
  if (status === "locked") return "未解锁";
  if (status) return "可兑换";
  return "—";
}

function hasCredits(entry) {
  return Number(entry.creditsGained) > 0;
}

function historyStatusLabel(entry) {
  if (entry.status === "needsAttention") return "需要处理";
  if (entry.status === "claimed") return "有领取";
  if (entry.status === "idle") return "无变化";
  if (entry.status === "error") return "运行失败";
  return "已检查";
}

function historyStatusClass(entry) {
  if (entry.status === "needsAttention" || entry.status === "error") {
    return "status-cell status-cell--danger";
  }
  if (entry.status === "claimed") return "status-cell status-cell--ok";
  return "status-cell";
}
</script>
