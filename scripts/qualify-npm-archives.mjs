import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdir, readFile, writeFile, cp } from "node:fs/promises";
import { resolve, join } from "node:path";
import { compareNpmArchive } from "./compare-npm-archive.mjs";

const root = resolve(import.meta.dirname, "..");
const workspace = resolve(process.argv[2] ?? join(root, ".."));
const output = resolve(
  process.argv[3] ?? join(root, "test-results/npm-qualification"),
);
const consumer = join(workspace, "consumer-host");
const cli = process.env.npm_execpath;
assert.ok(
  cli,
  "Invoke using pnpm test:npm-archives for portable pnpm execution",
);
const candidates = JSON.parse(
  await readFile(join(root, "vendor/plugin-tools/candidates.json"), "utf8"),
);
const env = { ...process.env };
delete env.NODE_PATH;
const checks = [];
const archiveComparisons = [];
const selectedInstallations = [];
const saveComparisons = () =>
  writeFile(
    join(output, "archive-comparisons.json"),
    JSON.stringify({ archiveComparisons, selectedInstallations }, null, 2) +
      "\n",
  );
await mkdir(output, { recursive: true });
await writeFile(
  join(output, "selected-candidates.json"),
  JSON.stringify(candidates, null, 2) + "\n",
);
function run(command, args, cwd, name) {
  const result = spawnSync(command, args, {
    cwd,
    env,
    encoding: "utf8",
    timeout: 1800000,
    maxBuffer: 32 * 1024 * 1024,
  });
  const stdout = result.stdout ?? "";
  const stderr = result.stderr ?? "";
  if (name) {
    console.log(`${name}: ${result.status === 0 ? "passed" : "failed"}`);
    // Retain failed commands too; the workflow uploads reports on failure.
    return writeFile(
      join(output, `${name.replaceAll(":", "-")}.log`),
      stdout + stderr,
    ).then(() => {
      assert.equal(
        result.status,
        0,
        `${name}: ${result.error ?? ""}\n${stdout}\n${stderr}`,
      );
      checks.push(name);
      return stdout.trim();
    });
  }
  assert.equal(
    result.status,
    0,
    `${command}: ${result.error ?? ""}\n${stdout}\n${stderr}`,
  );
  return stdout.trim();
}
const paths = {};
for (const item of candidates.packages) {
  const archive = resolve(root, item.archive);
  const bytes = await readFile(archive);
  assert.equal(createHash("sha256").update(bytes).digest("hex"), item.sha256);
  assert.equal(
    `sha512-${createHash("sha512").update(bytes).digest("base64")}`,
    item.integrity,
  );
  assert.equal(bytes.length, item.bytes);
  const source = join(workspace, item.repository.split("/")[1]);
  assert.equal(run("git", ["rev-parse", "HEAD"], source), item.sourceCommit);
  assert.equal(run("git", ["status", "--porcelain"], source), "");
  paths[item.kind] = { archive, source, metadata: item };
}
const hostCommit = run("git", ["rev-parse", "HEAD"], consumer);
assert.equal(
  hostCommit,
  process.env.LOMI_HOST_COMMIT,
  "Consumer must use reviewed host commit",
);
assert.equal(run("git", ["status", "--porcelain"], consumer), "");
env.LOMI_SDK_TARBALL = paths.sdk.archive;
env.LOMI_CLI_TARBALL = paths.cli.archive;
env.LOMI_GENERATOR_TARBALL = paths.generator.archive;
for (const [kind, { source }] of Object.entries(paths)) {
  for (const args of [
    ["install", "--frozen-lockfile"],
    ["check"],
    ["test"],
    ["format:check"],
  ]) {
    await run(process.execPath, [cli, ...args], source, `${kind}-${args[0]}`);
  }
  await run(
    process.execPath,
    [cli, kind === "cli" ? "pack:tools" : "pack:release"],
    source,
    `${kind}-pack`,
  );
  const packed = join(source, "artifacts", paths[kind].metadata.file);
  archiveComparisons.push({
    kind,
    stage: "clean-source-pack",
    ...compareNpmArchive(
      await readFile(packed),
      await readFile(paths[kind].archive),
    ),
  });
  const args =
    kind === "cli"
      ? ["test:archives"]
      : kind === "sdk"
        ? ["test:archive", paths.sdk.archive]
        : ["test:archive"];
  await saveComparisons();
  await run(process.execPath, [cli, ...args], source, `${kind}-archives`);
  archiveComparisons.push({
    kind,
    stage: "installed-archive-suite",
    ...compareNpmArchive(
      await readFile(packed),
      await readFile(paths[kind].archive),
    ),
  });
  await cp(
    join(source, "artifacts/archive-validation.json"),
    join(output, `${kind}-archive-validation.json`),
  );
  const report = JSON.parse(
    await readFile(join(source, "artifacts/archive-validation.json"), "utf8"),
  );
  const installedSelections =
    kind === "sdk"
      ? { sdk: report.sourceArchive }
      : kind === "cli"
        ? { sdk: report.archives.sdk, generator: report.archives.generator }
        : Object.fromEntries(
            report.registryPackages.map(({ name, file }) => [
              name === "@lomi-dev/plugin-sdk" ? "sdk" : "cli",
              file,
            ]),
          );
  assert.deepEqual(
    Object.keys(installedSelections).sort(),
    kind === "sdk"
      ? ["sdk"]
      : kind === "cli"
        ? ["generator", "sdk"]
        : ["cli", "sdk"],
    "Archive report must identify every selected package installation",
  );
  for (const [installedKind, archive] of Object.entries(installedSelections)) {
    const selected = paths[installedKind];
    assert.ok(
      (await readFile(archive)).equals(await readFile(selected.archive)),
      `${kind} suite must install the immutable selected ${installedKind} archive`,
    );
    selectedInstallations.push({
      suite: `${kind}-archives`,
      package: selected.metadata.name,
      installedArchive: archive,
      sha256: selected.metadata.sha256,
      integrity: selected.metadata.integrity,
      byteIdenticalToSelected: true,
    });
  }
  await saveComparisons();
  assert.equal(
    run("git", ["status", "--porcelain"], source),
    "",
    "Tests must preserve source checkout",
  );
}
// The existing SDK harness requires a release report next to its archive.
const sdkArtifacts = join(output, "sdk");
await mkdir(sdkArtifacts, { recursive: true });
await cp(paths.sdk.archive, join(sdkArtifacts, paths.sdk.metadata.file));
await writeFile(
  join(sdkArtifacts, "release.json"),
  JSON.stringify(paths.sdk.metadata, null, 2) + "\n",
);
env.CI = "true";
env.GITHUB_SHA = paths.sdk.metadata.sourceCommit;
await run(
  process.execPath,
  [
    join(paths.sdk.source, "scripts/test-consumers.mjs"),
    consumer,
    paths.cli.source,
    sdkArtifacts,
  ],
  paths.sdk.source,
  "consumer-compatibility",
);
await cp(
  join(paths.sdk.source, "artifacts/consumer-validation.json"),
  join(output, "consumer-validation.json"),
);
await writeFile(
  join(output, "qualification.json"),
  JSON.stringify(
    {
      schemaVersion: 1,
      date: new Date().toISOString(),
      platform: `${process.platform}-${process.arch}`,
      node: process.version,
      hostCommit,
      candidates,
      checks,
      archiveComparisons,
      selectedInstallations,
      archiveSelection:
        "Immutable selected archives retain exact digests; clean-source packs must match every byte except gzip's OS field, with identical uncompressed tar bytes",
      registryTested: false,
      desktopTested: false,
    },
    null,
    2,
  ) + "\n",
);
