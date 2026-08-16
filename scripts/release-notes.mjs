import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

import { versionFromTag } from "./version.mjs";

const SECTION_ORDER = [
  ["feat", "新增功能"],
  ["fix", "问题修复"],
  ["perf", "性能优化"],
  ["other", "其他调整"],
];

const CONVENTIONAL_SUBJECT = /^([a-z][a-z0-9-]*)(?:\(([^)]+)\))?(!)?:\s+(.+)$/i;
const VERSION_BUMP = /^(?:bump(?:ed)?|release)\s+version\b|^version\s+bump\b/i;

export function parseArguments(argumentsList) {
  const options = {};

  for (let index = 0; index < argumentsList.length; index += 1) {
    const argument = argumentsList[index];
    if (argument !== "--tag" && argument !== "--output") {
      throw new Error(`未知参数：${argument}`);
    }

    const value = argumentsList[index + 1];
    if (!value || value.startsWith("--")) {
      throw new Error(`参数 ${argument} 缺少值`);
    }

    options[argument.slice(2)] = value;
    index += 1;
  }

  if (!options.tag) {
    throw new Error("缺少必填参数：--tag");
  }
  if (!options.output) {
    throw new Error("缺少必填参数：--output");
  }

  versionFromTag(options.tag);
  return options;
}

export function parseCommit(commit) {
  const subject = commit.subject.trim();
  const match = CONVENTIONAL_SUBJECT.exec(subject);
  if (!match) {
    return { ...commit, subject, type: null, scope: null, description: subject };
  }

  return {
    ...commit,
    subject,
    type: match[1].toLowerCase(),
    scope: match[2] || null,
    description: match[4].trim(),
  };
}

export function isReleaseOnlyCommit(commit) {
  return commit.type === "chore" && VERSION_BUMP.test(commit.description);
}

export function sectionForCommit(commit) {
  if (commit.type === "feat" || commit.type === "fix" || commit.type === "perf") {
    return commit.type;
  }
  return "other";
}

export function groupCommits(commits) {
  const groups = new Map(SECTION_ORDER.map(([key]) => [key, []]));

  for (const rawCommit of commits) {
    const commit = parseCommit(rawCommit);
    if (!isReleaseOnlyCommit(commit)) {
      groups.get(sectionForCommit(commit)).push(commit);
    }
  }

  return groups;
}

export function escapeMarkdown(text) {
  return text.replace(/[\\[\]*_`]/g, "\\$&");
}

export function renderReleaseNotes({ tag, previousTag, commits, repositoryUrl }) {
  versionFromTag(tag);
  const normalizedRepositoryUrl = repositoryUrl.replace(/\/$/, "");
  const groups = groupCommits(commits);
  const lines = ["## 本次更新", ""];
  let visibleCommitCount = 0;

  for (const [key, title] of SECTION_ORDER) {
    const sectionCommits = groups.get(key);
    if (!sectionCommits.length) {
      continue;
    }

    lines.push(`### ${title}`, "");
    for (const commit of sectionCommits) {
      const label = `${escapeMarkdown(commit.subject)} (${commit.shortSha})`;
      lines.push(`- [${label}](${normalizedRepositoryUrl}/commit/${commit.sha})`);
      visibleCommitCount += 1;
    }
    lines.push("");
  }

  if (visibleCommitCount === 0) {
    lines.push("本版本仅包含发布与维护调整。", "");
  }

  lines.push(
    "## 下载",
    "",
    "- Windows 安装版：下载名称以 `-setup.exe` 结尾的 NSIS 安装器。",
    "- Windows 便携版：下载 `workbuddy-assistant.exe`。",
    "- 完整性校验：使用 `SHA256SUMS.txt` 核对下载文件。",
    "",
    "## 完整变更",
    "",
  );

  const changelogUrl = previousTag
    ? `${normalizedRepositoryUrl}/compare/${encodeURIComponent(previousTag)}...${encodeURIComponent(tag)}`
    : `${normalizedRepositoryUrl}/commits/${encodeURIComponent(tag)}`;
  lines.push(`[查看 ${previousTag ? `${previousTag}...${tag}` : tag} 的完整提交记录](${changelogUrl})`, "");

  return lines.join("\n");
}

export function repositoryUrlFromRemote(remoteUrl) {
  const trimmed = remoteUrl.trim().replace(/\.git$/, "");
  const scpMatch = /^(?:[^@]+@)?([^:]+):(.+)$/.exec(trimmed);
  if (scpMatch && !trimmed.includes("://")) {
    return `https://${scpMatch[1]}/${scpMatch[2]}`;
  }

  const url = new URL(trimmed);
  url.username = "";
  url.password = "";
  url.protocol = "https:";
  return url.toString().replace(/\/$/, "");
}

export function parseGitLog(output) {
  if (!output) {
    return [];
  }

  return output
    .split("\u001e")
    .filter(Boolean)
    .map((record) => {
      const [sha, shortSha, ...subjectParts] = record.trimStart().split("\u001f");
      return { sha, shortSha, subject: subjectParts.join("\u001f") };
    });
}

function runGit(argumentsList, cwd) {
  try {
    return execFileSync("git", argumentsList, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  } catch (error) {
    const detail = error.stderr?.trim() || error.message;
    throw new Error(`Git 命令执行失败：git ${argumentsList.join(" ")}\n${detail}`);
  }
}

export function findPreviousTag(tag, cwd) {
  const parent = `${tag}^`;
  try {
    runGit(["rev-parse", "--verify", `${parent}^{commit}`], cwd);
  } catch {
    return null;
  }

  const candidateTags = runGit(["tag", "--merged", parent, "--list", "v*"], cwd)
    .split(/\r?\n/)
    .filter(Boolean)
    .filter((candidate) => {
      try {
        versionFromTag(candidate);
        return candidate !== tag;
      } catch {
        return false;
      }
    });

  if (!candidateTags.length) {
    return null;
  }

  return candidateTags
    .map((candidate) => ({
      tag: candidate,
      distance: Number(runGit(["rev-list", "--count", `${candidate}..${parent}`], cwd)),
    }))
    .sort((left, right) => left.distance - right.distance || left.tag.localeCompare(right.tag))[0].tag;
}

export function readReleaseData({ tag, cwd, environment = process.env }) {
  versionFromTag(tag);
  runGit(["rev-parse", "--verify", `${tag}^{commit}`], cwd);

  const previousTag = findPreviousTag(tag, cwd);
  const range = previousTag ? `${previousTag}..${tag}` : tag;
  const log = runGit(
    ["log", "--reverse", "--no-merges", "--format=%H%x1f%h%x1f%s%x1e", range],
    cwd,
  );
  const repositoryUrl = environment.GITHUB_SERVER_URL && environment.GITHUB_REPOSITORY
    ? `${environment.GITHUB_SERVER_URL}/${environment.GITHUB_REPOSITORY}`
    : repositoryUrlFromRemote(runGit(["remote", "get-url", "origin"], cwd));

  return { tag, previousTag, commits: parseGitLog(log), repositoryUrl };
}

export function writeReleaseNotes({ tag, output, cwd, environment = process.env }) {
  const outputPath = path.resolve(cwd, output);
  const outputDirectory = path.dirname(outputPath);
  if (!fs.existsSync(outputDirectory)) {
    throw new Error(`输出目录不存在：${outputDirectory}`);
  }

  const releaseData = readReleaseData({ tag, cwd, environment });
  const markdown = renderReleaseNotes(releaseData);
  fs.writeFileSync(outputPath, markdown, "utf8");
  return { ...releaseData, outputPath, markdown };
}

const isEntrypoint = process.argv[1]
  && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (isEntrypoint) {
  try {
    const options = parseArguments(process.argv.slice(2));
    const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
    const result = writeReleaseNotes({ ...options, cwd: projectRoot });
    console.log(`发布说明已生成：${result.outputPath}`);
    console.log(`提交范围：${result.previousTag ? `${result.previousTag}..${result.tag}` : result.tag}`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
