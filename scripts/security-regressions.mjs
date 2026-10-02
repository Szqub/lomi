import { spawnSync } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(import.meta.dirname, "..");
const unixSuites = [
  ["lomi-control-core", "authentication::tests::"],
  ["lomi-control-core", "project_files::tests::"],
  ["lomi-control-core", "broker::revoke_tests::"],
];

export function securitySuites(platform = process.platform) {
  return [
    ["lomi", "files::tests::json_"],
    ...(platform === "win32" ? [] : unixSuites),
    ["lomi-remote-crypto", ""],
  ];
}

export function runSecurityRegressions(
  platform = process.platform,
  run = spawnSync,
) {
  for (const [crate, filter] of securitySuites(platform)) {
    console.log(`Security regressions: ${crate} ${filter}`);
    const result = run(
      "cargo",
      [
        "test",
        "--manifest-path",
        "src-tauri/Cargo.toml",
        "--locked",
        "-p",
        crate,
        ...(filter ? ["--lib", filter] : ["--tests"]),
      ],
      {
        cwd: root,
        encoding: "utf8",
        timeout: 1800000,
        maxBuffer: 16 * 1024 * 1024,
      },
    );
    process.stdout.write(result.stdout ?? "");
    process.stderr.write(result.stderr ?? "");
    if (
      result.status !== 0 ||
      !/test result: ok\. [1-9]\d* passed; 0 failed/.test(result.stdout ?? "")
    ) {
      throw new Error(
        `Security suite failed or selected no tests: ${crate} ${filter}\n${result.error ?? ""}`,
      );
    }
  }
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  runSecurityRegressions();
}
