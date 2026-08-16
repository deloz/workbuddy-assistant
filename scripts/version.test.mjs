import assert from "node:assert/strict";
import test from "node:test";

import {
  cargoLockVersion,
  cargoManifestVersion,
  validateVersions,
  versionFromTag,
} from "./version.mjs";

test("parses release tags and Cargo versions", () => {
  assert.equal(versionFromTag("v0.0.1"), "0.0.1");
  assert.equal(cargoManifestVersion('[package]\nname = "app"\nversion = "0.0.1"\n'), "0.0.1");
  assert.equal(
    cargoLockVersion('[[package]]\nname = "app"\nversion = "0.0.1"\n', "app"),
    "0.0.1",
  );
});

test("rejects malformed release tags", () => {
  assert.throws(() => versionFromTag("0.0.1"), /vX\.Y\.Z/);
  assert.throws(() => versionFromTag("v0.0.1-beta.1"), /vX\.Y\.Z/);
});

test("requires every package version to match the tag", () => {
  const versions = {
    tag: "v0.0.1",
    packageVersion: "0.0.1",
    packageLockVersion: "0.0.1",
    cargoVersion: "0.0.1",
    cargoLockVersion: "0.0.1",
  };
  assert.equal(validateVersions(versions), "0.0.1");
  assert.throws(
    () => validateVersions({ ...versions, cargoVersion: "0.1.0" }),
    /Cargo\.toml=0\.1\.0/,
  );
});
