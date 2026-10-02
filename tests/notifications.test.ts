import assert from "node:assert/strict";
import { test } from "node:test";
import {
  newestNotificationSnapshot,
  notificationBadge,
  unreadNotificationCount,
  reportNotificationError,
  subscribeNotificationErrors,
  clearNotificationError,
} from "../src/notifications.ts";
import type { NotificationSnapshot } from "../src/notifications.ts";

test("canonical inbox ignores stale and equal initial loads or mutation responses", () => {
  const current: NotificationSnapshot = { revision: 8, items: [] };
  assert.equal(
    newestNotificationSnapshot(current, { revision: 7, items: [] }),
    current,
  );
  assert.equal(
    newestNotificationSnapshot(current, { revision: 8, items: [] }),
    current,
  );
  const next = { revision: 9, items: [] };
  assert.equal(newestNotificationSnapshot(current, next), next);
  assert.equal(newestNotificationSnapshot(null, current), current);
});

test("unread counts use explicit read state and displayed badges cap at nine", () => {
  assert.equal(unreadNotificationCount(null), 0);
  const items = Array.from({ length: 12 }, (_, i) => ({
    id: String(i),
    kind: "finished" as const,
    title: "Done",
    body: "Context",
    createdAt: i,
    read: i < 2,
  }));
  assert.equal(unreadNotificationCount({ revision: 1, items }), 10);
  assert.equal(notificationBadge(1), "1");
  assert.equal(notificationBadge(9), "9");
  assert.equal(notificationBadge(10), "9+");
});

test("save errors reach an existing or subsequently mounted inbox only", () => {
  clearNotificationError();
  const errors: string[] = [];
  const unsubscribe = subscribeNotificationErrors((message) =>
    errors.push(message),
  );
  reportNotificationError("Disk is full");
  unsubscribe();
  reportNotificationError("Storage unavailable");
  assert.deepEqual(errors, ["Disk is full"]);
  const stop = subscribeNotificationErrors((message) => errors.push(message));
  assert.deepEqual(errors, ["Disk is full", "Storage unavailable"]);
  stop();
  clearNotificationError();
});
