import assert from "node:assert/strict";
import { test } from "node:test";
import { MockLanguageModelV4 } from "ai/test";
import { generate } from "../src/generate.ts";
import { generation, MAX_TEXT_RESPONSE, type Event } from "../src/protocol.ts";

for (const limit of ["text", "reasoning", "blocks"] as const) {
  test(
    `a ${limit} response limit aborts a provider that never closes`,
    { timeout: 2000 },
    async () => {
      const controller = new AbortController();
      let produced = 0;
      let providerSignal: AbortSignal | undefined;
      const events: Event[] = [];
      const model = new MockLanguageModelV4({
        doStream: async ({ abortSignal }) => {
          providerSignal = abortSignal;
          return {
            stream: new ReadableStream({
              pull(sink) {
                const index = produced++;
                if (index >= (limit === "blocks" ? 1026 : 2))
                  return new Promise<void>(() => {});
                if (limit === "blocks") {
                  sink.enqueue({ type: "text-start", id: `block-${index}` });
                } else if (index === 0) {
                  sink.enqueue({ type: `${limit}-start`, id: "block" });
                } else {
                  sink.enqueue({
                    type: `${limit}-delta`,
                    id: "block",
                    delta: "x".repeat(MAX_TEXT_RESPONSE + 1),
                  });
                }
              },
              // Providers may not acknowledge cancellation; reporting must still finish.
              cancel() {
                return new Promise<void>(() => {});
              },
            }),
          };
        },
      });
      await generate(
        generation.parse({
          provider: "openai",
          apiKey: "PRIVATE_KEY",
          model: "fixture",
          assistantId: "assistant",
          messages: [
            {
              id: "user",
              role: "user",
              parts: [{ type: "text", text: "PRIVATE_QUESTION" }],
            },
          ],
        }),
        "limit",
        controller,
        async (event) => {
          events.push(event);
        },
        model,
      );
      assert.equal(controller.signal.aborted, true);
      assert.equal(providerSignal?.aborted, true);
      const terminal = events.filter(({ type }) =>
        ["completed", "cancelled", "failed"].includes(type),
      );
      assert.equal(terminal.length, 1);
      assert.equal(terminal[0].type, "failed");
      assert.deepEqual(terminal[0].payload, {
        code: "response-limit",
        usage: undefined,
      });
      assert.equal(events.at(-2)?.type, "message-snapshot");
      assert.ok(produced < (limit === "blocks" ? 1100 : 10));
      assert.ok(!JSON.stringify(events).includes("PRIVATE"));
    },
  );
}
