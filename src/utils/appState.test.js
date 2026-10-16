import assert from "node:assert/strict";
import test from "node:test";

import {
  accountStatusLabel,
  accountTypeLabel,
  createEmptyDashboardSnapshot,
  formatGrowthCredits,
  formatSigninStatus,
  formatUpdateMessage,
  getUpdateMessageTone,
  growthStepTone,
  growthTriggerLabel,
  isSigninBusy,
  isActionableUpdate,
  normalizeError,
  resolveSelectedAccountId,
} from "./appState.js";

test("formats every update state without creating a separate row", () => {
  assert.equal(formatUpdateMessage({ checking: true }), "");
  assert.equal(formatUpdateMessage({ error: "网络错误" }), "网络错误");
  assert.equal(
    formatUpdateMessage({ result: { status: "updateAvailable", latestVersion: "0.0.2" } }),
    "发现新版本 v0.0.2",
  );
  assert.equal(formatUpdateMessage({ result: { status: "upToDate" } }), "当前已是最新版本");
  assert.equal(
    formatUpdateMessage({ result: { status: "ahead", latestVersion: "0.0.1" } }),
    "当前版本高于最新公开版本 v0.0.1",
  );
  assert.equal(formatUpdateMessage({ result: { status: "unpublished" } }), "尚未发布公开版本");
});

test("selects the preferred account and falls back predictably", () => {
  const snapshot = {
    currentAccountId: "enterprise",
    accounts: [{ accountId: "personal" }, { accountId: "enterprise" }],
  };

  assert.equal(resolveSelectedAccountId(snapshot, "personal"), "personal");
  assert.equal(resolveSelectedAccountId(snapshot, "missing"), "personal");
  assert.equal(resolveSelectedAccountId(createEmptyDashboardSnapshot(), null), null);
});

test("normalizes errors and update message tones", () => {
  assert.equal(normalizeError("请求失败"), "请求失败");
  assert.equal(normalizeError(new Error("连接超时")), "连接超时");
  assert.equal(normalizeError(null), "操作失败，请稍后重试");
  assert.equal(getUpdateMessageTone({ error: "失败" }), "error");
  assert.equal(getUpdateMessageTone({ result: { status: "updateAvailable" } }), "available");
  assert.equal(getUpdateMessageTone({ result: { status: "upToDate" } }), "neutral");
});

test("describes the browser sign-in progress", () => {
  assert.equal(formatSigninStatus("idle"), "");
  assert.equal(formatSigninStatus("opening"), "正在打开登录页面…");
  assert.equal(
    formatSigninStatus("waiting"),
    "已在浏览器打开登录页面，等待完成…最多 5 分钟",
  );
  assert.equal(formatSigninStatus("error"), "登录未完成，可重新点击浏览器登录");
  assert.equal(isSigninBusy("opening"), true);
  assert.equal(isSigninBusy("waiting"), true);
  assert.equal(isSigninBusy("done"), false);
  assert.equal(isSigninBusy("idle"), false);
});

test("labels account types and expired sign-ins", () => {
  assert.equal(accountTypeLabel("personal"), "个人账号");
  assert.equal(accountTypeLabel("enterprise"), "企业账号");
  assert.equal(accountStatusLabel({ accountType: "personal" }), "个人账号");
  assert.equal(accountStatusLabel({ accountType: "enterprise" }), "企业账号");
  assert.equal(
    accountStatusLabel({ accountType: "enterprise", needsSignin: true }),
    "需重新登录",
  );
});

test("only exposes complete update results as actionable", () => {
  const update = {
    status: "updateAvailable",
    currentVersion: "0.0.3",
    latestVersion: "0.0.4",
    releaseUrl: "https://github.com/deloz/workbuddy-assistant/releases/latest",
  };

  assert.equal(isActionableUpdate(update), true);
  assert.equal(isActionableUpdate({ ...update, releaseUrl: null }), false);
  assert.equal(isActionableUpdate({ ...update, latestVersion: "" }), false);
  assert.equal(isActionableUpdate({ ...update, status: "upToDate" }), false);
  assert.equal(isActionableUpdate(null), false);
});

test("labels growth triggers and step tones", () => {
  assert.equal(growthTriggerLabel("daily"), "每日自动");
  assert.equal(growthTriggerLabel("poll"), "定时轮询");
  assert.equal(growthTriggerLabel("manual"), "手动运行");
  assert.equal(growthTriggerLabel(undefined), "手动运行");

  assert.equal(growthStepTone("ok"), "ok");
  assert.equal(growthStepTone("failed"), "danger");
  assert.equal(growthStepTone("skipped"), "warn");
  assert.equal(growthStepTone("normal"), "muted");
});

test("formats growth credits without showing a bare zero", () => {
  assert.equal(formatGrowthCredits("0"), "—");
  assert.equal(formatGrowthCredits(""), "—");
  assert.equal(formatGrowthCredits(null), "—");
  assert.equal(formatGrowthCredits("100"), "+100");
  assert.equal(formatGrowthCredits("0.5"), "+0.5");
});

test("empty snapshot carries the growth schedule field", () => {
  assert.equal(createEmptyDashboardSnapshot().nextGrowthRunAt, null);
});
