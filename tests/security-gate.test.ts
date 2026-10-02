import test from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, delimiter } from "node:path";
import { fileURLToPath } from "node:url";
import {
  runSecurityRegressions,
  securitySuites,
} from "../scripts/security-regressions.mjs";

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

test("Windows gates JSON persistence and crypto without selecting Unix-only control suites", () => {
  assert.deepEqual(securitySuites("win32"), [
    ["lomi", "files::tests::json_"],
    ["lomi-remote-crypto", ""],
  ]);
  for (const platform of ["darwin", "linux"]) {
    assert.deepEqual(securitySuites(platform), [
      ["lomi", "files::tests::json_"],
      ["lomi-control-core", "authentication::tests::"],
      ["lomi-control-core", "project_files::tests::"],
      ["lomi-control-core", "broker::revoke_tests::"],
      ["lomi-remote-crypto", ""],
    ]);
  }
});

test("Windows rejects a failed or empty security suite and never advances past it", () => {
  for (const failure of ["failed", "empty"]) {
    for (const failureIndex of [0, 1]) {
      const crates: string[] = [];
      assert.throws(
        () =>
          runSecurityRegressions("win32", (command: string, args: string[]) => {
            assert.equal(command, "cargo");
            crates.push(args[args.indexOf("-p") + 1]);
            const failing = crates.length - 1 === failureIndex;
            return {
              status: failing && failure === "failed" ? 1 : 0,
              stdout: `test result: ok. ${failing && failure === "empty" ? 0 : 1} passed; 0 failed; 0 ignored\n`,
              stderr: "",
            };
          }),
        /Security suite failed or selected no tests/,
      );
      assert.deepEqual(
        crates,
        ["lomi", "lomi-remote-crypto"].slice(0, failureIndex + 1),
      );
    }
  }
  const crates: string[] = [];
  runSecurityRegressions("win32", (_command: string, args: string[]) => {
    crates.push(args[args.indexOf("-p") + 1]);
    return {
      status: 0,
      stdout: "test result: ok. 2 passed; 0 failed\n",
      stderr: "",
    };
  });
  assert.deepEqual(crates, ["lomi", "lomi-remote-crypto"]);
});
