import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  findPreviousTag,
  groupCommits,
  parseArguments,
  parseCommit,
  parseGitLog,
  readReleaseData,
  renderReleaseNotes,
  repositoryUrlFromRemote,
  writeReleaseNotes,
} from "./release-notes.mjs";

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const commit = (subject, shortSha = "abc1234") => ({
  sha: shortSha.padEnd(40, "0"),
  shortSha,
  subject,
});

test("parses CLI arguments and rejects incomplete input", () => {
  assert.deepEqual(parseArguments(["--tag", "v1.2.3", "--output", "notes.md"]), {
    tag: "v1.2.3",
    output: "notes.md",
  });
  assert.throws(() => parseArguments(["--tag", "1.2.3", "--output", "notes.md"]), /vX\.Y\.Z/);
  assert.throws(() => parseArguments(["--tag", "v1.2.3"]), /--output/);
  assert.throws(() => parseArguments(["--unknown", "value"]), /未知参数/);
});

test("parses and groups Conventional Commits in stable sections", () => {
  assert.deepEqual(parseCommit(commit("feat(account)!: 增加账号切换")), {
    ...commit("feat(account)!: 增加账号切换"),
    subject: "feat(account)!: 增加账号切换",
    type: "feat",
    scope: "account",
    description: "增加账号切换",
  });

  const groups = groupCommits([
    commit("docs: update README", "1111111"),
    commit("fix(update): use public feed", "2222222"),
    commit("feat: add check-in", "3333333"),
    commit("perf: reduce startup work", "4444444"),
    commit("chore: bump version to 1.0.0", "5555555"),
    commit("unstructured subject", "6666666"),
  ]);

  assert.deepEqual([...groups.keys()], ["feat", "fix", "perf", "other"]);
  assert.deepEqual(groups.get("feat").map(({ shortSha }) => shortSha), ["3333333"]);
  assert.deepEqual(groups.get("fix").map(({ shortSha }) => shortSha), ["2222222"]);
  assert.deepEqual(groups.get("perf").map(({ shortSha }) => shortSha), ["4444444"]);
  assert.deepEqual(groups.get("other").map(({ shortSha }) => shortSha), ["1111111", "6666666"]);
});

test("renders UTF-8 Markdown, escaped subjects, downloads, and compare links", () => {
  const markdown = renderReleaseNotes({
    tag: "v1.1.0",
    previousTag: "v1.0.0",
    commits: [commit("feat(ui): 支持 [多个] *账号*", "1234567")],
    repositoryUrl: "https://github.com/deloz/workbuddy-assistant/",
  });

  assert.match(markdown, /## 本次更新/);
  assert.match(markdown, /### 新增功能/);
  assert.ok(markdown.includes("支持 \\[多个\\] \\*账号\\*"));
  assert.match(markdown, /workbuddy-assistant\.exe/);
  assert.match(markdown, /SHA256SUMS\.txt/);
  assert.match(markdown, /compare\/v1\.0\.0\.\.\.v1\.1\.0/);
});

test("renders maintenance text without empty section headings", () => {
  const markdown = renderReleaseNotes({
    tag: "v1.0.0",
    previousTag: null,
    commits: [commit("chore: release version 1.0.0")],
    repositoryUrl: "https://github.com/deloz/workbuddy-assistant",
  });

  assert.match(markdown, /本版本仅包含发布与维护调整/);
  assert.doesNotMatch(markdown, /### 其他调整/);
  assert.match(markdown, /commits\/v1\.0\.0/);
});

test("renders an entirely empty release range", () => {
  const markdown = renderReleaseNotes({
    tag: "v1.0.1",
    previousTag: "v1.0.0",
    commits: [],
    repositoryUrl: "https://github.com/deloz/workbuddy-assistant",
  });

  assert.match(markdown, /本版本仅包含发布与维护调整/);
  assert.doesNotMatch(markdown, /### (新增功能|问题修复|性能优化|其他调整)/);
  assert.match(markdown, /compare\/v1\.0\.0\.\.\.v1\.0\.1/);
});

test("parses git records and removes credentials from repository remotes", () => {
  const records = parseGitLog(
    `a`.repeat(40) + "\u001faaaaaaa\u001ffix: one\u001e" +
    "\n" + `b`.repeat(40) + "\u001fbbbbbbb\u001ffeat: two\u001e",
  );
  assert.equal(records.length, 2);
  assert.equal(records[1].subject, "feat: two");
  assert.match(records[1].sha, /^[a-f0-9]{40}$/);
  assert.equal(
    repositoryUrlFromRemote("https://token@example.com/owner/repo.git"),
    "https://example.com/owner/repo",
  );
  assert.equal(
    repositoryUrlFromRemote("git@github.com:deloz/workbuddy-assistant.git"),
    "https://github.com/deloz/workbuddy-assistant",
  );
});

test("reads real first and incremental release ranges", () => {
  assert.equal(findPreviousTag("v0.0.1", projectRoot), null);
  assert.equal(findPreviousTag("v0.0.2", projectRoot), "v0.0.1");

  const firstRelease = readReleaseData({ tag: "v0.0.1", cwd: projectRoot, environment: {} });
  assert.equal(firstRelease.previousTag, null);
  assert.deepEqual(firstRelease.commits.map(({ shortSha }) => shortSha), ["9b52f5c", "fffa062", "9640f44"]);
  assert.ok(firstRelease.commits.every(({ sha }) => /^[a-f0-9]{40}$/.test(sha)));

  const secondRelease = readReleaseData({ tag: "v0.0.2", cwd: projectRoot, environment: {} });
  assert.equal(secondRelease.previousTag, "v0.0.1");
  assert.deepEqual(secondRelease.commits.map(({ shortSha }) => shortSha), ["446a21d"]);
});

test("rejects missing tags and output directories", () => {
  assert.throws(
    () => readReleaseData({ tag: "v9.9.9", cwd: projectRoot, environment: {} }),
    /Git 命令执行失败/,
  );

  const missingDirectory = path.join(os.tmpdir(), `missing-release-notes-${Date.now()}`, "notes.md");
  assert.throws(
    () => writeReleaseNotes({ tag: "v0.0.2", output: missingDirectory, cwd: projectRoot }),
    /输出目录不存在/,
  );
});

test("writes UTF-8 release notes to an existing directory", () => {
  const temporaryDirectory = fs.mkdtempSync(path.join(os.tmpdir(), "workbuddy-release-notes-"));
  const output = path.join(temporaryDirectory, "v0.0.2.md");

  try {
    const result = writeReleaseNotes({ tag: "v0.0.2", output, cwd: projectRoot, environment: {} });
    assert.equal(result.previousTag, "v0.0.1");
    assert.equal(fs.readFileSync(output, "utf8"), result.markdown);
    assert.match(result.markdown, /fix\(update\): use public releases feed/);
  } finally {
    fs.rmSync(temporaryDirectory, { recursive: true, force: true });
  }
});
