import test from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, delimiter } from "node:path";
import { fileURLToPath } from "node:url";

test(
  "the native release gate stops on failures and empty test selections",
  {
    skip:
      process.platform === "win32"
        ? "The fake Cargo executable fixture requires a Unix shebang."
        : false,
  },
  () => {
    const directory = mkdtempSync(join(tmpdir(), "lomi-security-gate-"));
    const log = join(directory, "calls.jsonl");
    try {
      writeFileSync(
        join(directory, "cargo"),
        `#!${process.execPath}
const fs = require('node:fs');
fs.appendFileSync(process.env.LOMI_SECURITY_TEST_LOG, JSON.stringify(process.argv.slice(2)) + '\\n');
if (process.env.LOMI_SECURITY_TEST_MODE === 'failure') process.exit(1);
const count = process.env.LOMI_SECURITY_TEST_MODE === 'empty' ? 0 : 1;
console.log('test result: ok. ' + count + ' passed; 0 failed; 0 ignored');
`,
        { mode: 0o700 },
      );
      const run = (mode: string) => {
        writeFileSync(log, "");
        return spawnSync(
          process.execPath,
          [
            fileURLToPath(
              new URL("../scripts/security-regressions.mjs", import.meta.url),
            ),
          ],
          {
            encoding: "utf8",
            timeout: 30000,
            env: {
              ...process.env,
              PATH: directory + delimiter + process.env.PATH,
              LOMI_SECURITY_TEST_LOG: log,
              LOMI_SECURITY_TEST_MODE: mode,
            },
          },
        );
      };
      for (const mode of ["failure", "empty"]) {
        const result = run(mode);
        assert.notEqual(result.status, 0, mode);
        assert.match(
          result.stderr,
          /Security suite failed or selected no tests/,
        );
        assert.equal(
          readFileSync(log, "utf8").trim().split("\n").length,
          1,
          "The failed gate must not advance to later suites.",
        );
      }
      const success = run("success");
      assert.equal(success.status, 0, success.stderr);
      assert.equal(readFileSync(log, "utf8").trim().split("\n").length, 5);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  },
);
