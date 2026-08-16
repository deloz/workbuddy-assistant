<template>
  <section
    class="view-panel"
    aria-labelledby="credit-title"
    aria-live="polite"
    :aria-busy="refreshing"
  >
    <div class="view-heading">
      <div>
        <p class="section-kicker">额度使用</p>
        <h1 id="credit-title">额度概览</h1>
      </div>
      <div class="view-heading__actions">
        <span v-if="account?.credit?.updatedAt" class="updated-at">更新于 {{ formatDate(account.credit.updatedAt) }}</span>
        <button class="button button--secondary" type="button" :disabled="disabled" @click="$emit('refresh')">
          {{ refreshing ? "刷新中…" : "刷新额度" }}
        </button>
      </div>
    </div>

    <div v-if="!account" class="inline-empty">
      <strong>还没有可用账号</strong>
      <span>请先完成 WorkBuddy 登录，再点击“重新检测”。</span>
    </div>
    <template v-else>
      <div v-if="account.credit.status === 'loading'" class="loading-line" role="status">正在加载 {{ account.nickname }} 的额度…</div>
      <div v-else-if="account.credit.status === 'error'" class="inline-error" role="alert">
        <strong>额度暂时无法读取</strong>
        <span>{{ account.credit.error }}</span>
      </div>
      <template v-else-if="account.credit.data">
        <div class="summary-strip">
          <div><span>总额度</span><strong>{{ formatAmount(account.credit.data.totalCredits) }}</strong></div>
          <div><span>已使用</span><strong>{{ formatAmount(account.credit.data.usedCredits) }}</strong></div>
          <div class="summary-strip__remaining"><span>剩余额度</span><strong>{{ formatAmount(account.credit.data.remainingCredits) }}</strong></div>
        </div>

        <div class="table-heading">
          <h2>资源明细</h2>
          <span>{{ account.credit.data.resources.length }} 项资源</span>
        </div>
        <div v-if="account.credit.data.resources.length" class="table-wrap">
          <table>
            <thead><tr><th scope="col">套餐</th><th scope="col">状态</th><th scope="col">总量</th><th scope="col">已使用</th><th scope="col">剩余</th><th scope="col">到期 / 刷新</th></tr></thead>
            <tbody>
              <tr v-for="(resource, index) in account.credit.data.resources" :key="`${resource.packageName}-${index}`">
                <th scope="row">{{ resource.packageName }}</th>
                <td :class="statusClass(resource.status)">{{ statusLabel(resource.status) }}</td>
                <td>{{ formatAmount(resource.totalCredits) }}</td>
                <td>{{ formatAmount(resource.usedCredits) }}</td>
                <td class="remaining-cell">{{ formatAmount(resource.remainingCredits) }}</td>
                <td><span>{{ formatDate(resource.expiresAt) }}</span><small v-if="resource.refreshAt">刷新：{{ formatDate(resource.refreshAt) }}</small></td>
              </tr>
            </tbody>
          </table>
        </div>
        <div v-else class="inline-empty inline-empty--small"><strong>当前没有可显示的资源</strong><span>额度服务返回了空资源列表。</span></div>
      </template>
      <div v-else class="loading-line" role="status">额度会在选择账号后自动加载。</div>
    </template>
  </section>
</template>

<script setup>
defineProps({
  account: { type: Object, default: null },
  refreshing: { type: Boolean, default: false },
  disabled: { type: Boolean, default: false },
});

defineEmits(["refresh"]);

const amountFormatter = new Intl.NumberFormat("zh-CN", {
  maximumFractionDigits: 2,
});

function formatAmount(value) {
  if (value === null || value === undefined || value === "") return "—";
  const numericValue = Number(value);
  return Number.isFinite(numericValue) ? amountFormatter.format(numericValue) : String(value);
}

function formatDate(value) {
  if (!value) return "未提供";
  const text = String(value).trim();
  const epochValue = /^\d{10,13}$/.test(text)
    ? Number(text) * (text.length === 10 ? 1000 : 1)
    : value;
  const parsed = new Date(epochValue);
  if (Number.isNaN(parsed.getTime())) return String(value);
  return new Intl.DateTimeFormat("zh-CN", { dateStyle: "medium", timeStyle: "short" }).format(parsed);
}

function statusLabel(status) {
  if (status === "valid") return "有效";
  if (status === "usedUp") return "已用完";
  return "其他";
}

function statusClass(status) {
  if (status === "valid") return "status-cell status-cell--ok";
  if (status === "usedUp") return "status-cell status-cell--danger";
  return "status-cell";
}
</script>
