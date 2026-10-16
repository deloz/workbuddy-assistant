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
    nextGrowthRunAt: null,
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

export function isSigninBusy(state) {
  return state === "opening" || state === "waiting";
}

export function formatSigninStatus(state) {
  switch (state) {
    case "opening":
      return "正在打开登录页面…";
    case "waiting":
      return "已在浏览器打开登录页面，等待完成…最多 5 分钟";
    case "done":
      return "登录成功，正在加载账号额度…";
    case "error":
      return "登录未完成，可重新点击浏览器登录";
    default:
      return "";
  }
}

export function accountTypeLabel(accountType) {
  return String(accountType).toLowerCase() === "personal" ? "个人账号" : "企业账号";
}

export function accountStatusLabel(account) {
  if (account?.needsSignin) {
    return "需重新登录";
  }
  return accountTypeLabel(account?.accountType);
}

export function growthTriggerLabel(trigger) {
  switch (trigger) {
    case "daily":
      return "每日自动";
    case "poll":
      return "定时轮询";
    default:
      return "手动运行";
  }
}

export function growthStepTone(status) {
  switch (status) {
    case "ok":
      return "ok";
    case "failed":
      return "danger";
    case "skipped":
      return "warn";
    default:
      return "muted";
  }
}

export function formatGrowthCredits(value) {
  const text = String(value ?? "").trim();
  if (!text || Number(text) === 0) {
    return "—";
  }
  return `+${text}`;
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
