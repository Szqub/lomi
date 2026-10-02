import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { gzipSync, gunzipSync } from "node:zlib";
import { compareNpmArchive } from "../scripts/compare-npm-archive.mjs";

const selected = readFileSync(
  new URL(
    "../vendor/plugin-sdk/lomi-dev-plugin-sdk-1.1.0-alpha.2.tgz",
    import.meta.url,
  ),
);

test("archive comparison permits only the platform gzip OS field", () => {
  const original = Buffer.from(selected);
  for (const os of [3, 10, 19]) {
    const packed = Buffer.from(selected);
    packed[9] = os;
    const result = compareNpmArchive(packed, selected);
    assert.equal(result.byteIdentical, os === selected[9]);
    assert.equal(result.packedGzipOS, os);
    assert.equal(result.selectedGzipOS, selected[9]);
    assert.equal(
      result.selectedSHA256,
      "2b58fc2b68f0a6ee342f2e2573a6f3004fc7b5b35f5a250a79955d8403811714",
    );
  }
  assert.deepEqual(
    selected,
    original,
    "Selected publication bytes are immutable",
  );
});

test("archive comparison rejects other headers, corruption and changed contents", () => {
  for (const offset of [0, 2, 3, 4, 8, 10, selected.length - 1]) {
    const corrupted = Buffer.from(selected);
    corrupted[offset] ^= 1;
    assert.throws(() => compareNpmArchive(corrupted, selected));
  }
  const unknownOS = Buffer.from(selected);
  unknownOS[9] = 255;
  assert.throws(
    () => compareNpmArchive(unknownOS, selected),
    /Unexpected gzip OS/,
  );
  assert.throws(() => compareNpmArchive(selected.subarray(0, -1), selected));
  const changedTar = gunzipSync(selected);
  changedTar[1024] ^= 1;
  const changedPackage = gzipSync(changedTar);
  changedPackage[9] = 3;
  assert.throws(() => compareNpmArchive(changedPackage, selected));
});
