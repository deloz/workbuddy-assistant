const SEMVER = "[0-9]+\\.[0-9]+\\.[0-9]+";
const TAG_PATTERN = new RegExp(`^v(${SEMVER})$`);

export function versionFromTag(tag) {
  const match = TAG_PATTERN.exec(tag || "");
  if (!match) {
    throw new Error(`发布标签必须使用 vX.Y.Z 格式，当前值：${tag || "<empty>"}`);
  }
  return match[1];
}

export function cargoManifestVersion(manifest) {
  const packageSection = manifest.match(/\[package\]([\s\S]*?)(?:\n\[|$)/)?.[1] || "";
  const version = packageSection.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  if (!version) {
    throw new Error("无法读取 src-tauri/Cargo.toml 的 package.version");
  }
  return version;
}

export function cargoLockVersion(lockfile, packageName) {
  const packageBlock = lockfile
    .split("[[package]]")
    .find((block) => new RegExp(`^name\\s*=\\s*"${packageName}"$`, "m").test(block));
  const version = packageBlock?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  if (!version) {
    throw new Error(`无法读取 Cargo.lock 中 ${packageName} 的版本`);
  }
  return version;
}

export function validateVersions({ tag, packageVersion, packageLockVersion, cargoVersion, cargoLockVersion }) {
  const expected = tag ? versionFromTag(tag) : packageVersion;
  const versions = {
    "package.json": packageVersion,
    "package-lock.json": packageLockVersion,
    "src-tauri/Cargo.toml": cargoVersion,
    "src-tauri/Cargo.lock": cargoLockVersion,
  };
  const mismatches = Object.entries(versions).filter(([, version]) => version !== expected);
  if (mismatches.length) {
    const details = mismatches.map(([file, version]) => `${file}=${version}`).join(", ");
    throw new Error(`版本必须统一为 ${expected}：${details}`);
  }
  return expected;
}
