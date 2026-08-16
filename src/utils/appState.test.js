import assert from "node:assert/strict";
import test from "node:test";

import {
  createEmptyDashboardSnapshot,
  formatUpdateMessage,
  getUpdateMessageTone,
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
