import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Modal } from "../ui";
import { SettingsNotice } from "../settings-ui";

interface Permission {
  id: string;
  runId: string;
  generation: number;
  tool: string;
  input: unknown;
  choices: { id: string; label: string; allow: boolean }[];
  textInput?: { label: string; initial: string; multiline: boolean };
}
export default function NativeApproval({ runId }: { runId: string }) {
  const [pending, setPending] = useState<Permission>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [text, setText] = useState("");
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => setText(pending?.textInput?.initial ?? ""), [pending?.id]);
  useEffect(() => {
    let live = true;
    let loading = false;
    const refresh = async () => {
      if (loading) return;
      loading = true;
      try {
        const next = await invoke<Permission[]>("cli_native_permissions", {
          runId,
        });
        if (live) setPending(next[0]);
      } catch (failure) {
        if (live) setError(String(failure));
      } finally {
        loading = false;
      }
    };
    void refresh();
    const timer = window.setInterval(() => void refresh(), 500);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, [runId]);
  async function decide(choiceId: string) {
    if (!pending || busy) return;
    const reviewed = pending;
    setBusy(true);
    setError("");
    try {
      await invoke("cli_native_permission_reply", {
        request: {
          approvalId: reviewed.id,
          runId,
          generation: reviewed.generation,
          choiceId,
          ...(choiceId === "submit-text" ? { text } : {}),
        },
      });
      setPending((current) =>
        current?.id === reviewed.id ? undefined : current,
      );
    } catch (failure) {
      setError(String(failure));
    } finally {
      setBusy(false);
    }
  }
  if (!pending) return null;
  const deny = pending.choices.find((choice) => !choice.allow);
  const tooLarge = new TextEncoder().encode(text).length > 64 * 1024;
  return (
    <Modal
      title={
        pending.textInput
          ? "Native CLI interaction"
          : "Review native CLI permission"
      }
      wide
      closeDisabled={busy || !deny}
      initialFocus={cancel}
      onClose={() => {
        if (deny) void decide(deny.id);
      }}
    >
      <div className="dialog-form" aria-busy={busy}>
        <p>
          The CLI requests permission for <strong>{pending.tool}</strong>. It
          executes this operation using its native tools and policies.
        </p>
        <pre>{JSON.stringify(pending.input, null, 2)}</pre>
        {pending.textInput && (
          <label>
            {pending.textInput.label}
            {pending.textInput.multiline ? (
              <textarea
                value={text}
                maxLength={64 * 1024}
                disabled={busy}
                onChange={(event) => setText(event.target.value)}
              />
            ) : (
              <input
                value={text}
                maxLength={64 * 1024}
                disabled={busy}
                onChange={(event) => setText(event.target.value)}
              />
            )}
          </label>
        )}
        {tooLarge && <p role="alert">Text exceeds the 64 KiB UTF-8 limit.</p>}
        {error && <SettingsNotice tone="error">{error}</SettingsNotice>}
        <div className="dialog-actions">
          {[...pending.choices]
            .sort((a, b) => Number(a.allow) - Number(b.allow))
            .map((choice, index) => (
              <button
                key={choice.id}
                ref={index === 0 ? cancel : undefined}
                className={`button${choice.allow ? " button-primary" : ""}`}
                disabled={busy || (choice.id === "submit-text" && tooLarge)}
                onClick={() => void decide(choice.id)}
              >
                {choice.label}
              </button>
            ))}
        </div>
      </div>
    </Modal>
  );
}
