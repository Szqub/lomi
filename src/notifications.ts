export interface InboxNotification {
  id: string;
  kind: "attention" | "finished";
  title: string;
  body: string;
  createdAt: number;
  read: boolean;
}

export interface NotificationSnapshot {
  revision: number;
  items: InboxNotification[];
}

// Equal revisions describe the same canonical state. Keeping the existing value
// also prevents a delayed response from replacing a snapshot received by event.
export function newestNotificationSnapshot(
  current: NotificationSnapshot | null,
  incoming: NotificationSnapshot,
): NotificationSnapshot {
  return !current || incoming.revision > current.revision ? incoming : current;
}

export function unreadNotificationCount(snapshot: NotificationSnapshot | null) {
  return snapshot?.items.filter((item) => !item.read).length ?? 0;
}

export function notificationBadge(count: number) {
  return count > 9 ? "9+" : String(count);
}

let lastError: string | null = null;
const errorListeners = new Set<(message: string) => void>();
export function reportNotificationError(message: string) {
  lastError = message;
  for (const listener of errorListeners) listener(message);
}
export function subscribeNotificationErrors(
  listener: (message: string) => void,
) {
  errorListeners.add(listener);
  if (lastError) listener(lastError);
  return () => {
    errorListeners.delete(listener);
  };
}
export function clearNotificationError() {
  lastError = null;
}
