import { mkdtemp, mkdir, readFile, writeFile, chmod } from "node:fs/promises";
import { tmpdir, homedir } from "node:os";
import { join, resolve } from "node:path";
import { spawn } from "node:child_process";
import { newSession, newProject } from "../../src/model.ts";

if (process.platform !== "darwin")
  throw Error("This native launcher runner supports macOS.");
const root = resolve(import.meta.dirname, "../..");
const directory = await mkdtemp(join(tmpdir(), "lomi-agent-launch-"));
const identifier = `dev.lomi.agent-launch-smoke-${Date.now()}`;
const appData = join(homedir(), "Library/Application Support", identifier);
const projectPath = join(directory, "project");
const shellHome = join(directory, "shell-home");
const bin = join(directory, "fixture bin");
const startupDelay = Number(
  process.env.LOMI_AGENT_LAUNCH_SMOKE_STARTUP_DELAY_SECONDS ?? 0,
);
if (!Number.isFinite(startupDelay) || startupDelay < 0 || startupDelay > 10)
  throw Error("Choose a fixture shell startup delay between 0 and 10 seconds.");
for (const path of [appData, projectPath, shellHome, bin])
  await mkdir(path, { recursive: true });
const quote = (value) => "'" + value.replaceAll("'", "'\\''") + "'";
for (const file of [".zshrc", ".zprofile"])
  await writeFile(
    join(shellHome, file),
    `${file === ".zshrc" && startupDelay ? `/bin/sleep ${startupDelay}\n` : ""}export PATH=${quote(bin)}:/usr/bin:/bin\nexport HISTFILE=/dev/null\n`,
  );
const executable = join(bin, "cursor-agent");
await writeFile(
  executable,
  `#!/bin/sh
printf '%s\\t%s\\n' "$$" "$PWD" >> "$LOMI_AGENT_LAUNCH_SMOKE_DIRECTORY/launches.txt"
awk 'BEGIN { for (i=0; i<16000; i++) print "background agent output"; }'
printf '\\033]0;Agent fixture %s\\007' "$$"
printf '%s\\n' "$$" >> "$LOMI_AGENT_LAUNCH_SMOKE_DIRECTORY/streamed.txt"
while IFS= read -r line; do :; done
`,
);
await chmod(executable, 0o755);
const project = newProject(projectPath, "local:zsh");
await writeFile(
  join(appData, "agent-control-preferences.json"),
  JSON.stringify({ version: 1, autoStart: false, yoloMode: false }),
);
await writeFile(
  join(appData, "session.json"),
  JSON.stringify({
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  }),
);
const port = Number(process.env.LOMI_AGENT_LAUNCH_SMOKE_PORT ?? 1442);
const config = join(directory, "config.json");
await writeFile(
  config,
  JSON.stringify({
    identifier,
    build: {
      beforeDevCommand: `pnpm dev --port ${port}`,
      devUrl: `http://127.0.0.1:${port}`,
    },
  }),
);
console.log(`Native agent artifacts: ${directory}`);
const child = spawn(
  "pnpm",
  [
    "tauri",
    "dev",
    "--no-watch",
    "--features",
    "native-smoke",
    "--config",
    config,
  ],
  {
    cwd: root,
    env: {
      ...process.env,
      LOMI_AGENT_LAUNCH_SMOKE_DIRECTORY: directory,
      SHELL: "/bin/zsh",
      ZDOTDIR: shellHome,
      HISTFILE: "/dev/null",
    },
    detached: true,
    stdio: ["ignore", "pipe", "pipe"],
  },
);
let log = "";
for (const stream of [child.stdout, child.stderr])
  stream.on("data", (chunk) => {
    log += chunk;
    process.stdout.write(chunk);
  });
try {
  const deadline = Date.now() + 300000;
  let result;
  while (!result && Date.now() < deadline) {
    try {
      result = JSON.parse(
        await readFile(join(directory, "result.json"), "utf8"),
      );
    } catch (error) {
      if (error.code !== "ENOENT" && !(error instanceof SyntaxError))
        throw error;
    }
    if (!result) {
      if (child.exitCode !== null)
        throw Error(`Tauri exited before reporting: ${child.exitCode}`);
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
  }
  if (!result) throw Error("Native launcher smoke timed out.");
  console.log(JSON.stringify(result, null, 2));
  if (result.stage !== "passed") process.exitCode = 1;
} finally {
  try {
    process.kill(-child.pid, "SIGTERM");
  } catch (error) {
    if (error.code !== "ESRCH") throw error;
  }
  await writeFile(join(directory, "native.log"), log);
}
