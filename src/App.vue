<template>
  <div class="app-shell">
    <header class="app-header">
      <div class="topbar">
        <div class="brand-block" aria-label="WorkBuddy助手">
          <span class="brand-mark" aria-hidden="true">W</span>
          <strong>WorkBuddy助手</strong>
        </div>

        <AccountSwitcher
          v-if="store.snapshot.loggedIn"
          :accounts="store.accounts"
          :selected-account-id="store.selectedAccountId"
          :revealed-account-ids="store.revealedAccountIds"
          :disabled="store.busy"
          @select="store.selectAccount"
          @reveal="store.revealAccountIdentifier"
          @add="store.beginSignin"
        />

        <button
          class="settings-entry"
          :class="{ 'settings-entry--active': store.activeView === 'settings' }"
          type="button"
          aria-label="设置"
          title="设置"
          :aria-pressed="store.activeView === 'settings'"
          @click="store.activeView = 'settings'"
        >
          <svg aria-hidden="true" focusable="false" viewBox="0 0 24 24">
            <path
              d="M19.43 12.98c.04-.32.07-.65.07-.98s-.03-.66-.07-.98l2.11-1.65a.5.5 0 0 0 .12-.64l-2-3.46a.5.5 0 0 0-.6-.22l-2.49 1a7.3 7.3 0 0 0-1.69-.98l-.38-2.65A.5.5 0 0 0 14 2h-4a.5.5 0 0 0-.5.42l-.38 2.65c-.61.25-1.18.58-1.69.98l-2.49-1a.5.5 0 0 0-.6.22l-2 3.46a.5.5 0 0 0 .12.64l2.11 1.65a7.2 7.2 0 0 0 0 1.96l-2.11 1.65a.5.5 0 0 0-.12.64l2 3.46a.5.5 0 0 0 .6.22l2.49-1c.51.4 1.08.73 1.69.98l.38 2.65c.04.24.25.42.5.42h4a.5.5 0 0 0 .5-.42l.38-2.65c.61-.25 1.18-.58 1.69-.98l2.49 1a.5.5 0 0 0 .6-.22l2-3.46a.5.5 0 0 0-.12-.64l-2.11-1.65ZM12 15.5a3.5 3.5 0 1 1 0-7 3.5 3.5 0 0 1 0 7Z"
            />
          </svg>
        </button>
      </div>
    </header>

    <div class="view-navigation">
      <nav class="primary-nav" role="tablist" aria-label="功能导航">
        <button
          v-for="item in navigation"
          :key="item.id"
          type="button"
          role="tab"
          :data-view-id="item.id"
          :aria-selected="store.activeView === item.id"
          :class="{ 'primary-nav__item--active': store.activeView === item.id }"
          class="primary-nav__item"
          @click="store.activeView = item.id"
          @keydown.left.prevent="focusNavigation(item.id, -1, $event)"
          @keydown.right.prevent="focusNavigation(item.id, 1, $event)"
        >
          {{ item.label }}
        </button>
      </nav>
    </div>

    <main class="app-content">
      <SettingsPage v-if="store.activeView === 'settings'" />

      <section v-else-if="!store.snapshot.loggedIn" class="login-guide" aria-live="polite">
        <div class="login-guide__number" aria-hidden="true">1</div>
        <div>
          <p class="section-kicker">需要先登录</p>
          <h1>用系统浏览器完成 WorkBuddy 登录</h1>
          <p>点击「浏览器登录」会在系统浏览器打开 WorkBuddy 登录页面，正常登录后这里会自动读取账号列表、加载当前账号额度，并检查全部账号的今日签到。凭据只保存在本机，可以添加多个账号。</p>
          <p v-if="store.signinStatus" class="login-guide__progress" role="status">
            {{ store.signinStatus }}
          </p>
          <p v-if="store.snapshot.loginError || store.error" class="login-guide__reason" role="status">
            {{ store.snapshot.loginError || store.error }}
          </p>
        </div>
        <div class="login-guide__actions">
          <button
            class="button button--primary"
            type="button"
            :disabled="store.loading || store.signinBusy"
            @click="store.beginSignin"
          >
            {{ store.signinBusy ? "等待登录完成…" : "浏览器登录" }}
          </button>
          <button
            v-if="store.signinUrl"
            class="button button--secondary"
            type="button"
            @click="store.openSigninPage"
          >
            重新打开登录页面
          </button>
          <button
            class="button button--secondary"
            type="button"
            :disabled="store.loading"
            @click="store.reloadAccounts"
          >
            {{ store.loading ? "检测中…" : "重新检测" }}
          </button>
        </div>
      </section>

      <template v-else>
        <CreditView
          v-if="store.activeView === 'credit'"
          :account="store.currentAccount"
          :refreshing="Boolean(store.refreshingAccountId)"
          :disabled="store.busy || !store.currentAccount"
          @refresh="store.refreshAccountCredit()"
        />
        <CheckinView
          v-else-if="store.activeView === 'checkin'"
          :account="store.currentAccount"
          :next-auto-checkin-at="store.snapshot.nextAutoCheckinAt"
          :loading="store.checkinLoading"
          @run="store.runCheckinCurrent"
        />
        <GrowthView
          v-else
          :account="store.currentAccount"
          :has-accounts="store.accounts.length > 0"
          :next-growth-run-at="store.snapshot.nextGrowthRunAt"
          :automation-enabled="settingsStore.growthAutomationEnabled"
          :loading="store.growthLoading"
          @run="store.runGrowthCurrent"
          @run-all="store.runGrowthAll"
        />
      </template>
    </main>

    <footer class="app-footer">
      <span>关闭窗口后继续在系统托盘运行</span>
    </footer>

    <UpdateNotice />
  </div>
</template>

<script setup>
import { onMounted } from "vue";

import CheckinView from "./components/checkin/CheckinView.vue";
import AccountSwitcher from "./components/common/AccountSwitcher.vue";
import UpdateNotice from "./components/common/UpdateNotice.vue";
import CreditView from "./components/credit/CreditView.vue";
import GrowthView from "./components/growth/GrowthView.vue";
import SettingsPage from "./components/pages/SettingsPage.vue";
import { useDashboardStore } from "./stores/dashboard";
import { useSettingsStore } from "./stores/settings";

const store = useDashboardStore();
const settingsStore = useSettingsStore();
const navigation = [
  { id: "credit", label: "额度概览" },
  { id: "checkin", label: "签到状态" },
  { id: "growth", label: "成长中心" },
];

function focusNavigation(currentId, direction, event) {
  const currentIndex = navigation.findIndex((item) => item.id === currentId);
  const next = navigation[(currentIndex + direction + navigation.length) % navigation.length];
  store.activeView = next.id;
  event.currentTarget.parentElement.querySelector(`[data-view-id="${next.id}"]`)?.focus();
}

async function initializeUpdateCheck() {
  await settingsStore.initialize();
  await settingsStore.checkForUpdates();
}

onMounted(() => {
  void store.initialize();
  void settingsStore.loadAppSettings();
  void initializeUpdateCheck();
});
</script>
