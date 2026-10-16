import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import {
  createEmptyDashboardSnapshot,
  formatSigninStatus,
  isSigninBusy,
  normalizeError,
  resolveSelectedAccountId,
} from "../utils/appState";

export const useDashboardStore = defineStore("dashboard", () => {
  const snapshot = ref(createEmptyDashboardSnapshot());
  const activeView = ref("credit");
  const selectedAccountId = ref(null);
  const loading = ref(false);
  const refreshingAccountId = ref(null);
  const checkinInvoking = ref(false);
  const growthInvoking = ref(false);
  const error = ref(null);
  const revealedAccountIds = ref({});
  const signinState = ref("idle");
  const signinUrl = ref(null);
  let stopListening = null;

  const accounts = computed(() => snapshot.value.accounts || []);
  const currentAccount = computed(() =>
    accounts.value.find((account) => account.accountId === selectedAccountId.value) ||
    accounts.value.find((account) => account.isCurrent) ||
    accounts.value[0] ||
    null,
  );
  const checkinLoading = computed(
    () =>
      checkinInvoking.value ||
      accounts.value.some((account) => account.checkin?.status === "loading"),
  );
  const growthLoading = computed(
    () =>
      growthInvoking.value ||
      accounts.value.some((account) => account.growth?.status === "running"),
  );
  const busy = computed(
    () =>
      loading.value ||
      refreshingAccountId.value !== null ||
      checkinLoading.value ||
      growthLoading.value,
  );
  const signinBusy = computed(() => isSigninBusy(signinState.value));
  const signinStatus = computed(() => formatSigninStatus(signinState.value));

  function applySnapshot(nextSnapshot) {
    snapshot.value = nextSnapshot || createEmptyDashboardSnapshot();
    selectedAccountId.value = resolveSelectedAccountId(
      snapshot.value,
      selectedAccountId.value,
    );
  }

  function updateAccount(accountId, patch) {
    snapshot.value = {
      ...snapshot.value,
      accounts: accounts.value.map((account) =>
        account.accountId === accountId ? { ...account, ...patch } : account,
      ),
    };
  }

  async function ensureListener() {
    if (stopListening) {
      return;
    }
    const listeners = await Promise.all([
      listen("dashboard://state-changed", (event) => {
        applySnapshot(event.payload);
      }),
      listen("dashboard://open-checkin", () => {
        activeView.value = "checkin";
      }),
    ]);
    stopListening = () => listeners.forEach((unlisten) => unlisten());
  }

  async function loadState(command = "get_dashboard_state") {
    await ensureListener();
    loading.value = true;
    error.value = null;
    try {
      applySnapshot(await invoke(command));
      return true;
    } catch (reason) {
      error.value = normalizeError(reason);
      return false;
    } finally {
      loading.value = false;
    }
  }

  async function initialize() {
    const loaded = await loadState();
    const initial = currentAccount.value;
    if (loaded && snapshot.value.loggedIn && initial) {
      await refreshAccountCredit(initial.accountId);
    }
    return loaded;
  }

  async function reloadAccounts() {
    revealedAccountIds.value = {};
    const loaded = await loadState("reload_accounts");
    const initial = currentAccount.value;
    if (loaded && snapshot.value.loggedIn && initial && initial.credit.status === "idle") {
      await refreshAccountCredit(initial.accountId);
    }
    return loaded;
  }

  async function selectAccount(accountId) {
    if (!accounts.value.some((account) => account.accountId === accountId)) {
      return;
    }
    selectedAccountId.value = accountId;
    revealedAccountIds.value = {};
    const account = accounts.value.find((item) => item.accountId === accountId);
    if (snapshot.value.loggedIn && account?.credit.status === "idle") {
      await refreshAccountCredit(accountId);
    }
  }

  async function refreshAccountCredit(accountId = currentAccount.value?.accountId) {
    if (!accountId || refreshingAccountId.value !== null) {
      return false;
    }
    refreshingAccountId.value = accountId;
    error.value = null;
    try {
      const result = await invoke("refresh_account_credit", { accountId });
      updateAccount(accountId, { credit: result });
      return true;
    } catch (reason) {
      const message = normalizeError(reason);
      updateAccount(accountId, {
        credit: {
          accountId,
          status: "error",
          data: null,
          error: message,
          updatedAt: new Date().toISOString(),
        },
      });
      error.value = message;
      return false;
    } finally {
      refreshingAccountId.value = null;
    }
  }

  async function runCheckinCurrent(accountId = currentAccount.value?.accountId) {
    if (!accountId || checkinLoading.value) {
      return false;
    }
    checkinInvoking.value = true;
    error.value = null;
    try {
      await invoke("run_checkin_account", { accountId });
      return true;
    } catch (reason) {
      error.value = normalizeError(reason);
      return false;
    } finally {
      checkinInvoking.value = false;
    }
  }

  async function runGrowthCurrent(accountId = currentAccount.value?.accountId) {
    if (!accountId || growthLoading.value) {
      return false;
    }
    growthInvoking.value = true;
    error.value = null;
    try {
      await invoke("run_growth_account", { accountId });
      return true;
    } catch (reason) {
      error.value = normalizeError(reason);
      return false;
    } finally {
      growthInvoking.value = false;
    }
  }

  async function runGrowthAll() {
    if (growthLoading.value) {
      return false;
    }
    growthInvoking.value = true;
    error.value = null;
    try {
      await invoke("run_growth_all");
      return true;
    } catch (reason) {
      error.value = normalizeError(reason);
      return false;
    } finally {
      growthInvoking.value = false;
    }
  }

  async function revealAccountIdentifier(accountId) {
    if (revealedAccountIds.value[accountId]) {
      const next = { ...revealedAccountIds.value };
      delete next[accountId];
      revealedAccountIds.value = next;
      return;
    }
    try {
      const identifier = await invoke("reveal_account_identifier", { accountId });
      revealedAccountIds.value = {
        ...revealedAccountIds.value,
        [accountId]: identifier,
      };
    } catch (reason) {
      error.value = normalizeError(reason);
    }
  }

  // 先取登录地址并在系统浏览器打开，然后等待用户在浏览器里完成登录。
  async function beginSignin() {
    if (signinBusy.value) {
      return false;
    }
    signinState.value = "opening";
    error.value = null;
    try {
      const start = await invoke("begin_workbuddy_signin");
      signinUrl.value = start?.url || null;
      signinState.value = "waiting";
    } catch (reason) {
      signinState.value = "error";
      error.value = normalizeError(reason);
      return false;
    }
    return completeSignin();
  }

  // 等待浏览器登录完成，成功后装载账号并加载当前账号额度。
  async function completeSignin() {
    try {
      applySnapshot(await invoke("complete_workbuddy_signin"));
      signinState.value = "done";
      signinUrl.value = null;
      revealedAccountIds.value = {};
      const account = currentAccount.value;
      if (account && account.credit.status === "idle") {
        await refreshAccountCredit(account.accountId);
      }
      return true;
    } catch (reason) {
      signinState.value = "error";
      error.value = normalizeError(reason);
      return false;
    }
  }

  async function openSigninPage() {
    error.value = null;
    try {
      await invoke("open_workbuddy_signin_page");
      return true;
    } catch (reason) {
      error.value = normalizeError(reason);
      return false;
    }
  }

  async function signOutAccount(accountId) {
    error.value = null;
    try {
      applySnapshot(await invoke("sign_out_account", { accountId }));
      revealedAccountIds.value = {};
      signinState.value = "idle";
      signinUrl.value = null;
      return true;
    } catch (reason) {
      error.value = normalizeError(reason);
      return false;
    }
  }

  return {
    snapshot,
    accounts,
    currentAccount,
    selectedAccountId,
    activeView,
    loading,
    refreshingAccountId,
    checkinLoading,
    growthLoading,
    busy,
    signinBusy,
    signinState,
    signinStatus,
    signinUrl,
    error,
    revealedAccountIds,
    initialize,
    reloadAccounts,
    selectAccount,
    refreshAccountCredit,
    runCheckinCurrent,
    runGrowthCurrent,
    runGrowthAll,
    revealAccountIdentifier,
    beginSignin,
    completeSignin,
    openSigninPage,
    signOutAccount,
  };
});
