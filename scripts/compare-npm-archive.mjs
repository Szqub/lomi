import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { gunzipSync } from "node:zlib";

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const integrity = (bytes) =>
  `sha512-${createHash("sha512").update(bytes).digest("base64")}`;

export function compareNpmArchive(packed, selected) {
  // RFC 1952 stores the compressor's OS at byte 9. pnpm's otherwise identical
  // gzip envelope records 3 on Linux, 10 on Windows and 19 on macOS.
  for (const bytes of [packed, selected]) {
    assert.ok(bytes.length >= 18, "Truncated gzip archive");
    assert.ok(
      bytes.subarray(0, 4).equals(Buffer.from([31, 139, 8, 0])),
      "Expected gzip deflate archive without optional header fields",
    );
    assert.ok([3, 10, 19].includes(bytes[9]), "Unexpected gzip OS field");
  }
  assert.equal(packed.length, selected.length, "Packed archive length changed");
  assert.ok(
    packed.subarray(0, 9).equals(selected.subarray(0, 9)) &&
      packed.subarray(10).equals(selected.subarray(10)),
    "Packed archive differs beyond the gzip OS header field",
  );
  const packedTar = gunzipSync(packed);
  const selectedTar = gunzipSync(selected);
  assert.ok(
    packedTar.equals(selectedTar),
    "Uncompressed package bytes changed",
  );
  return {
    packedSHA256: sha256(packed),
    packedIntegrity: integrity(packed),
    selectedSHA256: sha256(selected),
    selectedIntegrity: integrity(selected),
    tarSHA256: sha256(selectedTar),
    packedGzipOS: packed[9],
    selectedGzipOS: selected[9],
    byteIdentical: packed.equals(selected),
    comparison:
      "All bytes equal except the permitted gzip OS field at offset 9",
  };
}
