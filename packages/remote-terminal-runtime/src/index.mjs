import { TerminalModels } from "./model.mjs";
const models = new TerminalModels();
let pending = Buffer.alloc(0),
  queue = Promise.resolve(),
  queued = 0,
  closed = false;
function stop() {
  if (closed) return;
  closed = true;
  models.dispose();
  process.exitCode = 1;
  process.stdin.destroy();
}
process.stdin.on("data", (chunk) => {
  if (closed) return;
  pending = Buffer.concat([pending, chunk]);
  if (pending.length > 65536) return stop();
  for (;;) {
    const i = pending.indexOf(10);
    if (i < 0) break;
    const raw = pending.subarray(0, i);
    pending = pending.subarray(i + 1);
    if (raw.length > 32768 || ++queued > 128) return stop();
    queue = queue
      .then(async () => {
        if (closed) return;
        let req;
        try {
          req = JSON.parse(raw.toString("utf8"));
          const result = await models.request(req);
          await reply({ id: req.id, ok: true, result });
        } catch (err) {
          await reply({
            id: Number.isSafeInteger(req?.id) ? req.id : null,
            ok: false,
            error: [
              "INVALID_REQUEST",
              "INVALID_DIMENSIONS",
              "INVALID_DATA",
              "SESSION_LIMIT",
              "STALE_SESSION",
              "STALE_SEQUENCE",
              "SNAPSHOT_PENDING",
              "SNAPSHOT_LIMIT",
              "MODEL_LIMIT",
            ].includes(err?.message)
              ? err.message
              : "MODEL_UNAVAILABLE",
          });
        } finally {
          queued--;
        }
      })
      .catch(stop);
  }
});
async function reply(v) {
  const line = JSON.stringify(v) + "\n";
  if (line.length > 12 * 1024 * 1024) throw new Error("REPLY_LIMIT");
  if (!process.stdout.write(line))
    await new Promise((resolve, reject) => {
      const cleanup = () => {
        process.stdout.off("drain", drain);
        process.stdout.off("error", error);
      };
      const drain = () => {
        cleanup();
        resolve();
      };
      const error = (err) => {
        cleanup();
        reject(err);
      };
      process.stdout.once("drain", drain);
      process.stdout.once("error", error);
    });
}
process.stdin.on("end", () => {
  queue.finally(() => models.dispose());
});
process.stdin.on("error", stop);
process.stdout.on("error", stop);
