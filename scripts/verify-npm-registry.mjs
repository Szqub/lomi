import assert from "node:assert/strict";
import { createHash } from "node:crypto";

export function registryReleaseAgeExclusions(packages, retained = []) {
  return JSON.stringify([
    ...new Set([
      ...retained,
      ...packages.map(({ name, version }) => `${name}@${version}`),
    ]),
  ]);
}

export function verifyRegistryMetadata(selected, metadata) {
  assert.equal(metadata.name, selected.name, "Registry package name mismatch");
  assert.equal(metadata.version, selected.version, "Registry version mismatch");
  assert.equal(
    metadata.dist.integrity,
    selected.integrity,
    "Registry integrity mismatch",
  );
  const url = new URL(metadata.dist.tarball);
  assert.equal(url.origin, "https://registry.npmjs.org");
  assert.equal(url.username + url.password + url.search + url.hash, "");
  assert.equal(
    url.pathname,
    `/${selected.name}/-/${selected.name.split("/").at(-1)}-${selected.version}.tgz`,
    "Unexpected registry tarball URL",
  );
  return url.href;
}

export async function verifyRegistryDownloads(
  packages,
  { metadata, download, selectedBytes },
) {
  const verified = [];
  // Complete all verification before the caller can start consumer installs.
  for (const selected of packages) {
    const published = await metadata(selected);
    const url = verifyRegistryMetadata(selected, published);
    const bytes = Buffer.from(await download(url));
    assert.equal(
      bytes.length,
      selected.bytes,
      "Registry archive size mismatch",
    );
    assert.equal(
      createHash("sha256").update(bytes).digest("hex"),
      selected.sha256,
      "Registry SHA-256 mismatch",
    );
    assert.equal(
      `sha512-${createHash("sha512").update(bytes).digest("base64")}`,
      selected.integrity,
      "Registry SHA-512 mismatch",
    );
    assert.ok(
      bytes.equals(await selectedBytes(selected)),
      "Registry archive differs from immutable selected bytes",
    );
    verified.push({ selected, url, bytes });
  }
  return verified;
}

export function verifyRegistryLock(lock, selected) {
  const escape = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const key = escape(`${selected.name}@${selected.version}`);
  const entry = lock.match(
    new RegExp(`^  ['"]?${key}['"]?:\\r?\\n((?: {4}[^\\n]*(?:\\n|$))*)`, "m"),
  );
  assert.ok(
    entry,
    `Missing registry lock entry: ${selected.name}@${selected.version}`,
  );
  const integrity = entry[1].match(
    /\bintegrity:\s*['"]?(sha512-[A-Za-z0-9+/=]+)/,
  );
  assert.equal(
    integrity?.[1],
    selected.integrity,
    `Registry lock integrity mismatch: ${selected.name}`,
  );
  assert.doesNotMatch(
    entry[1],
    /\b(?:tarball|directory):\s*['"]?(?:file:|\.\.?[\\/])/,
    "Registry lock must not resolve a local candidate",
  );
}
