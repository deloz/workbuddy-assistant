<template>
  <section class="view-panel settings-view" aria-labelledby="settings-title">
    <header class="view-heading">
      <div>
        <p class="section-kicker">应用偏好</p>
        <h1 id="settings-title">设置</h1>
      </div>
    </header>

    <section class="settings-section" aria-labelledby="accounts-title">
      <header class="settings-section__heading">
        <h2 id="accounts-title">WorkBuddy 账号</h2>
        <button
          class="button button--secondary"
          type="button"
          :disabled="dashboard.loading || dashboard.signinBusy"
          @click="dashboard.beginSignin"
        >
          {{ dashboard.signinBusy ? "等待登录完成…" : "添加账号" }}
        </button>
      </header>

      <p
        v-if="dashboard.signinStatus"
        class="settings-message settings-message--neutral"
        role="status"
        aria-live="polite"
      >
        {{ dashboard.signinStatus }}
      </p>
      <p
        v-if="dashboard.error"
        class="settings-message settings-message--error"
        role="status"
        aria-live="polite"
      >
        {{ dashboard.error }}
      </p>

      <div v-if="dashboard.accounts.length" class="account-list">
        <article
          v-for="account in dashboard.accounts"
          :key="account.accountId"
          class="account-list__item"
        >
          <div class="account-list__identity">
            <strong>{{ account.nickname }}</strong>
            <span class="account-list__id">{{ account.maskedAccountId }}</span>
          </div>
          <div class="account-list__meta">
            <span>{{ accountTypeLabel(account.accountType) }}</span>
            <span
              class="account-list__state"
              :class="{ 'account-list__state--warn': account.needsSignin }"
            >
              {{ accountStatusLabel(account) }}
            </span>
          </div>
          <div class="account-list__actions">
            <button
              class="button button--secondary settings-link"
              type="button"
              :disabled="dashboard.signinBusy"
              @click="dashboard.beginSignin"
            >
              重新登录
            </button>
            <button
              class="button button--secondary settings-link"
              type="button"
              :disabled="dashboard.busy"
              @click="dashboard.signOutAccount(account.accountId)"
            >
              移除
            </button>
          </div>
        </article>
      </div>
      <p v-else class="settings-message settings-message--neutral" role="status">
        尚未登录，请先完成 WorkBuddy 登录。
      </p>
    </section>

    <section class="settings-section" aria-labelledby="automation-title">
      <h2 id="automation-title">自动化</h2>
      <div class="about-list">
        <div class="about-row">
          <dt>成长中心</dt>
          <dd>
            <button
              class="button button--secondary settings-link"
              type="button"
              :disabled="store.savingGrowthAutomation"
              :aria-pressed="store.growthAutomationEnabled"
              @click="store.toggleGrowthAutomation"
            >
              {{ store.growthAutomationEnabled ? "已开启，点击关闭" : "已关闭，点击开启" }}
            </button>
            <p class="settings-message settings-message--neutral">
              开启后，应用运行期间会在每天 00:00 与每 4 小时自动领取旅行礼物、任务奖励、补登与盲盒；关闭后仅保留自动签到。
            </p>
            <p
              v-if="settingsError"
              class="settings-message settings-message--error"
              role="status"
            >
              {{ settingsError }}
            </p>
          </dd>
        </div>
        <div class="about-row">
          <dt>开机启动</dt>
          <dd>
            <button
              class="button button--secondary settings-link"
              type="button"
              :disabled="store.savingLaunchAtStartup"
              :aria-pressed="store.launchAtStartup"
              @click="store.toggleLaunchAtStartup"
            >
              {{ store.launchAtStartup ? "已开启，点击关闭" : "已关闭，点击开启" }}
            </button>
            <p class="settings-message settings-message--neutral">
              开启后，登录 Windows 时会在后台自动启动并常驻托盘，按原计划继续自动签到；关闭后需要手动启动应用。安装版卸载时会清理由此写入的启动项，便携版或 Scoop 用户需要自行移除。
            </p>
            <p
              v-if="launchAtStartupError"
              class="settings-message settings-message--error"
              role="status"
            >
              {{ launchAtStartupError }}
            </p>
          </dd>
        </div>
      </div>
    </section>

    <section class="settings-section" aria-labelledby="about-title">
      <h2 id="about-title">关于</h2>
      <dl class="about-list">
        <div class="about-row about-row--version">
          <dt>版本</dt>
          <dd>
            <span class="settings-version">{{ store.version ? `v${store.version}` : "—" }}</span>
            <button
              class="button button--secondary"
              type="button"
              :disabled="store.loading || store.checking"
              @click="store.checkForUpdates"
            >
              <span v-if="store.checking" class="button-spinner" aria-hidden="true"></span>
              {{ store.checking ? "检查中…" : "检查更新" }}
            </button>
            <p
              v-if="updateMessage"
              class="settings-message"
              :class="`settings-message--${updateMessageTone}`"
              role="status"
              aria-live="polite"
            >
              {{ updateMessage }}
            </p>
            <button
              v-if="store.actionableUpdate"
              class="button button--primary"
              type="button"
              @click="store.openUpdateGuide"
            >
              查看更新指导
            </button>
          </dd>
        </div>
        <div class="about-row">
          <dt>作者</dt>
          <dd>Deloz</dd>
        </div>
        <div class="about-row about-row--links">
          <dt>相关链接</dt>
          <dd>
            <button
              v-for="link in links"
              :key="link.url"
              class="button button--secondary settings-link"
              type="button"
              @click="store.openExternalLink(link.url)"
            >
              {{ link.label }}
            </button>
          </dd>
        </div>
        <div v-if="store.linkError" class="about-row about-row--message">
          <dt>链接状态</dt>
          <dd>
            <p class="settings-message settings-message--error" role="status">
              {{ store.linkError }}
            </p>
          </dd>
        </div>
      </dl>
    </section>
  </section>
</template>

<script setup>
import { computed, onMounted } from "vue";

import { useDashboardStore } from "../../stores/dashboard";
import { EXTERNAL_LINKS, useSettingsStore } from "../../stores/settings";
import {
  accountStatusLabel,
  accountTypeLabel,
  formatUpdateMessage,
  getUpdateMessageTone,
} from "../../utils/appState";

const store = useSettingsStore();
const dashboard = useDashboardStore();
const links = [
  { label: "项目仓库", url: EXTERNAL_LINKS.repository },
];

const updateMessage = computed(() =>
  formatUpdateMessage({
    checking: store.checking,
    error: store.updateError,
    result: store.updateResult,
  }),
);
const updateMessageTone = computed(() =>
  getUpdateMessageTone({
    error: store.updateError,
    result: store.updateResult,
  }),
);
const settingsError = computed(() => store.settingsError);
const launchAtStartupError = computed(() => store.launchAtStartupError);

onMounted(() => {
  store.initialize();
  store.loadAppSettings();
});
</script>
