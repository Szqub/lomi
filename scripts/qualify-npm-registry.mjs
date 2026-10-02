import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdir, readFile, writeFile, cp, mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import {
  verifyRegistryDownloads,
  verifyRegistryLock,
  registryReleaseAgeExclusions,
} from "./verify-npm-registry.mjs";

const root = resolve(import.meta.dirname, "..");
const workspace = resolve(process.argv[2] ?? join(root, ".."));
const output = resolve(
  process.argv[3] ?? join(root, "test-results/npm-qualification/registry"),
);
const host = join(workspace, "registry-host");
const cli = process.env.npm_execpath;
assert.ok(cli, "Invoke through pnpm for portable execution");
const candidates = JSON.parse(
  await readFile(join(root, "vendor/plugin-tools/candidates.json"), "utf8"),
);
const env = {
  ...process.env,
  npm_config_registry: "https://registry.npmjs.org",
  pnpm_config_registry: "https://registry.npmjs.org",
  CI: "true",
};
for (const name of [
  "LOMI_SDK_TARBALL",
  "LOMI_CLI_TARBALL",
  "LOMI_GENERATOR_TARBALL",
  "LOMI_SDK_REPO",
  "NODE_PATH",
  "NPM_AUTH_TOKEN",
  "NODE_AUTH_TOKEN",
])
  delete env[name];
const checks = [];
await mkdir(output, { recursive: true });
await writeFile(
  join(output, "selected-candidates.json"),
  JSON.stringify(candidates, null, 2) + "\n",
);
function run(command, args, cwd) {
  const result = spawnSync(command, args, {
    cwd,
    env,
    encoding: "utf8",
    timeout: 1800000,
    maxBuffer: 32 * 1024 * 1024,
  });
  assert.equal(
    result.status,
    0,
    `${command}: ${result.error ?? ""}\n${result.stdout ?? ""}\n${result.stderr ?? ""}`,
  );
  return result.stdout.trim();
}
async function check(name, args, cwd) {
  const result = spawnSync(process.execPath, [cli, ...args], {
    cwd,
    env,
    encoding: "utf8",
    timeout: 1800000,
    maxBuffer: 32 * 1024 * 1024,
  });
  await writeFile(
    join(output, `${name}.log`),
    (result.stdout ?? "") + (result.stderr ?? ""),
  );
  assert.equal(
    result.status,
    0,
    `${name}: ${result.error ?? ""}\n${result.stdout ?? ""}\n${result.stderr ?? ""}`,
  );
  checks.push(name);
  console.log(`${name}: passed`);
}
const sources = {};
for (const selected of candidates.packages) {
  const source = join(workspace, selected.repository.split("/")[1]);
  assert.equal(
    run("git", ["rev-parse", "HEAD"], source),
    selected.sourceCommit,
  );
  assert.equal(run("git", ["status", "--porcelain"], source), "");
  sources[selected.kind] = source;
}
const hostCommit = run("git", ["rev-parse", "HEAD"], host);
assert.equal(hostCommit, process.env.LOMI_HOST_COMMIT);
assert.equal(run("git", ["status", "--porcelain"], host), "");
const configuredHostExclusions = run(
  process.execPath,
  [cli, "config", "get", "minimumReleaseAgeExclude", "--json"],
  host,
);
const hostExclusions =
  configuredHostExclusions === "undefined"
    ? []
    : JSON.parse(configuredHostExclusions);
assert.ok(Array.isArray(hostExclusions));
for (const key of Object.keys(env)) {
  if (/^(?:pnpm|npm)_config_minimum_release_age_exclude$/i.test(key))
    delete env[key];
}
const verified = await verifyRegistryDownloads(candidates.packages, {
  metadata: async (selected) => {
    const response = await fetch(
      `https://registry.npmjs.org/${selected.name}/${selected.version}`,
      { redirect: "error", signal: AbortSignal.timeout(30000) },
    );
    assert.ok(
      response.ok,
      `Registry metadata HTTP ${response.status}: ${selected.name}@${selected.version}`,
    );
    return response.json();
  },
  download: async (url) => {
    const response = await fetch(url, {
      redirect: "error",
      signal: AbortSignal.timeout(30000),
    });
    assert.ok(response.ok, `Registry tarball HTTP ${response.status}`);
    return Buffer.from(await response.arrayBuffer());
  },
  selectedBytes: (selected) => readFile(join(root, selected.archive)),
});
const registryPackages = [];
for (const { selected, url, bytes } of verified) {
  await writeFile(join(output, selected.file), bytes);
  registryPackages.push({
    name: selected.name,
    version: selected.version,
    tarball: url,
    bytes: bytes.length,
    sha256: selected.sha256,
    integrity: selected.integrity,
    byteIdenticalToSelected: true,
  });
}
await writeFile(
  join(output, "registry-packages.json"),
  JSON.stringify(registryPackages, null, 2) + "\n",
);
const sdk = candidates.packages.find(({ kind }) => kind === "sdk");
const registryTemporary = await mkdtemp(join(tmpdir(), "lomi-registry-"));
// The pinned suites create bare runners and the author test creates a separate
// project. An inherited pnpm setting reaches every disposable nested install.
env.PNPM_CONFIG_MINIMUM_RELEASE_AGE_EXCLUDE = registryReleaseAgeExclusions(
  candidates.packages,
);
for (const kind of ["cli", "generator"]) {
  env.pnpm_config_store_dir = join(registryTemporary, `${kind}-store`);
  env.pnpm_config_cache_dir = join(registryTemporary, `${kind}-cache`);
  await check(`${kind}-registry`, ["test:registry"], sources[kind]);
  const report = JSON.parse(
    await readFile(
      join(sources[kind], "artifacts/registry-validation.json"),
      "utf8",
    ),
  );
  assert.equal(report.distribution, "npm");
  assert.equal(report.platform, `${process.platform}-${process.arch}`);
  for (const selected of candidates.packages) {
    const published = report.registryPackages.find(
      ({ name }) => name === selected.name,
    );
    assert.equal(published?.version, selected.version);
    assert.equal(published?.integrity, selected.integrity);
  }
  await cp(
    join(sources[kind], "artifacts/registry-validation.json"),
    join(output, `${kind}-registry-validation.json`),
  );
  await cp(
    join(report.directory, "runner/pnpm-lock.yaml"),
    join(output, `${kind}-runner-lock.yaml`),
  );
  verifyRegistryLock(
    await readFile(join(report.directory, "runner/pnpm-lock.yaml"), "utf8"),
    candidates.packages.find(({ kind }) => kind === "generator"),
  );
  for (const result of report.results) {
    const project = join(
      report.directory,
      kind === "generator" ? `autor żółć ${result.template}` : result.template,
    );
    const lock = await readFile(join(project, "pnpm-lock.yaml"), "utf8");
    for (const selected of candidates.packages.filter(
      ({ kind }) => kind !== "generator",
    ))
      verifyRegistryLock(lock, selected);
    await cp(
      join(project, "pnpm-lock.yaml"),
      join(output, `${kind}-${result.template}-lock.yaml`),
    );
  }
}
env.PNPM_CONFIG_MINIMUM_RELEASE_AGE_EXCLUDE = registryReleaseAgeExclusions(
  candidates.packages,
  hostExclusions,
);
env.pnpm_config_store_dir = join(registryTemporary, "host-store");
env.pnpm_config_cache_dir = join(registryTemporary, "host-cache");
for (const file of [
  join(host, "package.json"),
  join(host, "tests/fixtures/context-plugin/package.json"),
]) {
  const metadata = JSON.parse(await readFile(file, "utf8"));
  assert.ok(metadata.dependencies[sdk.name]);
  metadata.dependencies[sdk.name] = sdk.version;
  await writeFile(file, JSON.stringify(metadata, null, 2) + "\n");
}
const workspaceFile = join(host, "pnpm-workspace.yaml");
const configuration = await readFile(workspaceFile, "utf8");
assert.match(configuration, /^minimumReleaseAgeExclude:\r?$/m);
await writeFile(
  workspaceFile,
  configuration.replace(
    /^minimumReleaseAgeExclude:\r?\n/m,
    `minimumReleaseAgeExclude:\n  - "${sdk.name}@${sdk.version}"\n`,
  ),
);
await check(
  "host-registry-install",
  ["install", "--no-frozen-lockfile", "--ignore-scripts"],
  host,
);
await check(
  "host-registry-frozen",
  ["install", "--frozen-lockfile", "--ignore-scripts"],
  host,
);
const hostLock = await readFile(join(host, "pnpm-lock.yaml"), "utf8");
verifyRegistryLock(hostLock, sdk);
for (const directory of [host, join(host, "tests/fixtures/context-plugin")]) {
  const installed = JSON.parse(
    await readFile(
      join(directory, "node_modules", sdk.name, "package.json"),
      "utf8",
    ),
  );
  assert.equal(installed.version, sdk.version);
}
for (const args of [
  ["sdk:verify"],
  ["check"],
  ["test"],
  ["build"],
  ["sdk:test:consumer"],
]) {
  if (args[0] === "sdk:test:consumer") {
    env.pnpm_config_store_dir = join(registryTemporary, "author-store");
    env.pnpm_config_cache_dir = join(registryTemporary, "author-cache");
  }
  await check(`host-registry-${args[0].replaceAll(":", "-")}`, args, host);
}
await cp(join(host, "pnpm-lock.yaml"), join(output, "host-registry-lock.yaml"));
await writeFile(
  join(output, "registry-qualification.json"),
  JSON.stringify(
    {
      schemaVersion: 1,
      date: new Date().toISOString(),
      hostCommit,
      platform: `${process.platform}-${process.arch}`,
      registryPackages,
      sourceCommits: Object.fromEntries(
        candidates.packages.map(({ repository, sourceCommit }) => [
          repository,
          sourceCommit,
        ]),
      ),
      releaseAgeExclusions: {
        runners: JSON.parse(registryReleaseAgeExclusions(candidates.packages)),
        hostAndAuthor: JSON.parse(env.PNPM_CONFIG_MINIMUM_RELEASE_AGE_EXCLUDE),
      },
      checks,
      hostLockSHA256: createHash("sha256").update(hostLock).digest("hex"),
      registryTested: true,
      desktopTested: false,
    },
    null,
    2,
  ) + "\n",
);
