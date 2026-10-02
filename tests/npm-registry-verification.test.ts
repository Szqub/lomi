import test from "node:test";
import assert from "node:assert/strict";
import {
  readFileSync,
  mkdtempSync,
  mkdirSync,
  writeFileSync,
  rmSync,
} from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  verifyRegistryMetadata,
  verifyRegistryDownloads,
  verifyRegistryLock,
  registryReleaseAgeExclusions,
} from "../scripts/verify-npm-registry.mjs";

const root = new URL("../", import.meta.url);
const packages = JSON.parse(
  readFileSync(new URL("vendor/plugin-tools/candidates.json", root), "utf8"),
).packages;
const bytes = (selected) => readFileSync(new URL(selected.archive, root));
const metadata = (selected) => ({
  name: selected.name,
  version: selected.version,
  dist: {
    integrity: selected.integrity,
    tarball: `https://registry.npmjs.org/${selected.name}/-/${selected.name.split("/").at(-1)}-${selected.version}.tgz`,
  },
});

test("exact release-age exceptions and isolated stores reach bare runners and external authors", () => {
  const directory = mkdtempSync(join(tmpdir(), "lomi-release-age-probe-"));
  try {
    const expected = packages.map(({ name, version }) => `${name}@${version}`);
    const retained = ["@ai-sdk/openai-compatible@3.0.52"];
    assert.deepEqual(
      JSON.parse(registryReleaseAgeExclusions(packages, retained)),
      [...retained, ...expected],
    );
    writeFileSync(
      join(directory, "package.json"),
      JSON.stringify({
        private: true,
        type: "module",
        packageManager: "pnpm@11.25.0",
        scripts: { probe: "node probe.mjs" },
      }),
    );
    for (const name of ["bare-runner", "external-author"]) {
      const project = join(directory, name);
      mkdirSync(project);
      writeFileSync(join(project, "package.json"), '{"private":true}');
    }
    writeFileSync(
      join(directory, "probe.mjs"),
      `
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
const expected = ${JSON.stringify(expected)};
for (const project of ['bare-runner', 'external-author']) {
  const get = (key) => {
    const result = spawnSync(process.execPath, [process.env.npm_execpath, 'config', 'get', key, '--json'], { cwd: project, env: { ...process.env, npm_config_store_dir: '/unused-legacy-store' }, encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    return JSON.parse(result.stdout);
  };
  assert.deepEqual(get('minimumReleaseAgeExclude'), expected);
  assert.equal(get('minimumReleaseAge'), 4320);
  assert.equal(get('storeDir'), process.env.pnpm_config_store_dir);
  assert.equal(get('cacheDir'), process.env.pnpm_config_cache_dir);
}
`,
    );
    const args = ["run", "probe"];
    const inherited = { ...process.env };
    for (const key of Object.keys(inherited))
      if (/^(?:pnpm|npm)_config_minimum_release_age(?:_exclude)?$/i.test(key))
        delete inherited[key];
    const result = spawnSync(
      process.env.npm_execpath ? process.execPath : "pnpm",
      process.env.npm_execpath ? [process.env.npm_execpath, ...args] : args,
      {
        cwd: directory,
        shell: !process.env.npm_execpath && process.platform === "win32",
        env: {
          ...inherited,
          PNPM_CONFIG_MINIMUM_RELEASE_AGE_EXCLUDE:
            registryReleaseAgeExclusions(packages),
          PNPM_CONFIG_MINIMUM_RELEASE_AGE: "4320",
          pnpm_config_store_dir: join(directory, "isolated-store"),
          pnpm_config_cache_dir: join(directory, "isolated-cache"),
        },
        encoding: "utf8",
        timeout: 60000,
      },
    );
    assert.equal(result.status, 0, result.stdout + result.stderr);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("registry preflight verifies all exact archives before allowing consumer installation", async () => {
  const downloaded = [];
  let installs = 0;
  const result = await verifyRegistryDownloads(packages, {
    metadata: async (selected) => metadata(selected),
    download: async (url) => {
      const selected = packages.find(
        (selected) => metadata(selected).dist.tarball === url,
      );
      downloaded.push(selected.name);
      return bytes(selected);
    },
    selectedBytes: async (selected) => bytes(selected),
  }).then((result) => {
    installs++;
    return result;
  });
  assert.deepEqual(
    downloaded,
    packages.map((selected) => selected.name),
  );
  assert.equal(installs, 1);
  assert.equal(result.length, 3);
});

test("invalid registry metadata fails before download or consumer installation", async () => {
  for (const change of [
    (value) => {
      value.name = "unreviewed-package";
    },
    (value) => {
      value.version = "0.0.0";
    },
    (value) => {
      value.dist.integrity = "sha512-unreviewed";
    },
    (value) => {
      value.dist.tarball = value.dist.tarball.replace(
        "registry.npmjs.org",
        "example.org",
      );
    },
    (value) => {
      value.dist.tarball += "?other=archive";
    },
  ]) {
    let downloads = 0;
    let installs = 0;
    await assert.rejects(
      verifyRegistryDownloads(packages, {
        metadata: async (selected) => {
          const value = metadata(selected);
          change(value);
          return value;
        },
        download: async () => {
          downloads++;
          return Buffer.alloc(0);
        },
        selectedBytes: async (selected) => bytes(selected),
      }).then(() => installs++),
    );
    assert.equal(downloads, 0);
    assert.equal(installs, 0);
  }
});

test("a late corrupted download and a platform gzip envelope cannot reach consumer installation", async () => {
  for (const mode of ["payload", "gzip-os", "size"]) {
    let installs = 0;
    await assert.rejects(
      verifyRegistryDownloads(packages, {
        metadata: async (selected) => metadata(selected),
        download: async (url) => {
          const selected = packages.find(
            (selected) => metadata(selected).dist.tarball === url,
          );
          const result = bytes(selected);
          if (selected.kind !== "generator") return result;
          if (mode === "size") return result.subarray(0, -1);
          result[mode === "gzip-os" ? 9 : 100] ^= 1;
          return result;
        },
        selectedBytes: async (selected) => bytes(selected),
      }).then(() => installs++),
    );
    assert.equal(installs, 0);
  }
});

test("registry lock verification rejects missing versions and wrong or local resolutions", () => {
  const selected = packages[0];
  const lock = `packages:\n  '${selected.name}@${selected.version}':\n    resolution: {integrity: ${selected.integrity}}\n    engines: {node: '>=22.14.0'}\n`;
  verifyRegistryLock(lock, selected);
  verifyRegistryLock(lock.replaceAll("\n", "\r\n"), selected);
  assert.throws(() =>
    verifyRegistryLock(lock.replace(selected.version, "0.0.0"), selected),
  );
  assert.throws(() =>
    verifyRegistryLock(
      lock.replace(selected.integrity, "sha512-other"),
      selected,
    ),
  );
  assert.throws(() =>
    verifyRegistryLock(
      lock.replace(
        "resolution: {",
        "resolution: {tarball: file:../candidate.tgz, ",
      ),
      selected,
    ),
  );
  assert.throws(() =>
    verifyRegistryMetadata(selected, {
      ...metadata(selected),
      version: "latest",
    }),
  );
});
