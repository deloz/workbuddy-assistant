import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { cargoLockVersion, cargoManifestVersion, validateVersions } from "./version.mjs";

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const read = (relativePath) => fs.readFileSync(path.join(projectRoot, relativePath), "utf8");

const packageJson = JSON.parse(read("package.json"));
const packageLock = JSON.parse(read("package-lock.json"));
const cargoManifest = read("src-tauri/Cargo.toml");
const cargoLock = read("src-tauri/Cargo.lock");
const tag = process.argv[2] || process.env.GITHUB_REF_NAME || "";

try {
  const version = validateVersions({
    tag,
    packageVersion: packageJson.version,
    packageLockVersion: packageLock.packages?.[""]?.version || packageLock.version,
    cargoVersion: cargoManifestVersion(cargoManifest),
    cargoLockVersion: cargoLockVersion(cargoLock, packageJson.name),
  });
  console.log(`版本校验通过：${tag || `v${version}`}`);
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
