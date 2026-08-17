export function normalizeError(error) {
  if (typeof error === "string" && error.trim()) {
    return error;
  }
  if (error && typeof error.message === "string" && error.message.trim()) {
    return error.message;
  }
  return "操作失败，请稍后重试";
}

export function createEmptyDashboardSnapshot() {
  return {
    loggedIn: false,
    loginError: null,
    currentAccountId: null,
    accounts: [],
    nextAutoCheckinAt: null,
    schedulerRunning: false,
  };
}

export function resolveSelectedAccountId(snapshot, selectedAccountId) {
  const accounts = snapshot?.accounts || [];
  const preferred = selectedAccountId || snapshot?.currentAccountId;
  if (accounts.some((account) => account.accountId === preferred)) {
    return preferred;
  }
  return accounts[0]?.accountId || null;
}

export function formatUpdateMessage({ checking, error, result }) {
  if (checking) {
    return "";
  }
  if (error) {
    return error;
  }
  if (!result) {
    return "";
  }
  if (result.status === "updateAvailable") {
    return `发现新版本 v${result.latestVersion}`;
  }
  if (result.status === "upToDate") {
    return "当前已是最新版本";
  }
  if (result.status === "ahead") {
    return `当前版本高于最新公开版本 v${result.latestVersion}`;
  }
  return "尚未发布公开版本";
}

export function isActionableUpdate(result) {
  return (
    result?.status === "updateAvailable" &&
    typeof result.currentVersion === "string" &&
    result.currentVersion.trim().length > 0 &&
    typeof result.latestVersion === "string" &&
    result.latestVersion.trim().length > 0 &&
    typeof result.releaseUrl === "string" &&
    result.releaseUrl.trim().length > 0
  );
}

export function getUpdateMessageTone({ error, result }) {
  if (error) {
    return "error";
  }
  if (result?.status === "updateAvailable") {
    return "available";
  }
  return "neutral";
}
