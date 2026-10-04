import { api, native } from "./api";

/** Only user gestures count here; rendering, polling and heartbeats do not. */
export function trackRemoteActivity() {
  if (!native) return;
  let last = -Infinity;
  const record = (event: Event) => {
    if (!event.isTrusted) return;
    const now = performance.now();
    if (now - last < 1000) return;
    last = now;
    void api("remote_note_activity").catch(() => {});
  };
  for (const type of [
    "pointerdown",
    "pointermove",
    "keydown",
    "input",
    "wheel",
  ])
    window.addEventListener(type, record, { capture: true, passive: true });
}
