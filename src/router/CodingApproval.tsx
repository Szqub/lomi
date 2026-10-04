import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { freezeCodingEffect } from "../editor-service";
import { Modal } from "../ui";
import { SettingsNotice } from "../settings-ui";

interface Approval {
  approvalId: string;
  projectRoot: string;
  targetPath: string;
  preview: {
    binding: { run_id: string; generation: number; call_id: string };
    path: string;
    args_digest: string;
    before_hash: string | null;
    after_hash: string;
    result_digest: string;
    before_text: string | null;
    after_text: string;
  };
}

export default function CodingApproval({ runId }: { runId: string }) {
  const [pending, setPending] = useState<Approval | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    let live = true;
    let loading = false;
    async function refresh() {
      if (loading) return;
      loading = true;
      try {
        const approvals = await invoke<Approval[]>("cli_run_effect_approvals");
        if (live)
          setPending(
            approvals.find((item) => item.preview.binding.run_id === runId) ??
              null,
          );
      } catch (failure) {
        if (live) setError(String(failure));
      } finally {
        loading = false;
      }
    }
    void refresh();
    const subscription = listen(
      "cli-router-effect-approval",
      () => void refresh(),
    );
    const timer = window.setInterval(() => void refresh(), 1000);
    return () => {
      live = false;
      window.clearInterval(timer);
      void subscription.then((stop) => stop());
    };
  }, [runId]);

  async function decide(approve: boolean) {
    if (!pending || busy) return;
    const reviewed = pending;
    setBusy(true);
    setError("");
    let frozen: Awaited<ReturnType<typeof freezeCodingEffect>> | undefined;
    try {
      if (approve)
        frozen = await freezeCodingEffect(
          reviewed.targetPath,
          reviewed.preview.before_hash,
        );
      await invoke("cli_run_decide_effect", {
        request: {
          approvalId: reviewed.approvalId,
          argsDigest: reviewed.preview.args_digest,
          afterHash: reviewed.preview.after_hash,
          resultDigest: reviewed.preview.result_digest,
          approve,
          editorProof: frozen?.proof ?? null,
        },
      });
      setPending((current) =>
        current?.approvalId === reviewed.approvalId ? null : current,
      );
    } catch (failure) {
      setError(String(failure));
    } finally {
      // Native approval returns only after the worker has committed the result
      // or retained uncertain recovery. Keep buffers frozen until that reply.
      frozen?.release();
      setBusy(false);
    }
  }
  if (!pending) return null;
  return (
    <Modal
      title="Review CLI file change"
      wide
      closeDisabled={busy}
      onClose={() => void decide(false)}
    >
      <div className="dialog-form router-file-approval" aria-busy={busy}>
        <p>
          The CLI requests a change to <strong>{pending.preview.path}</strong>{" "}
          in {pending.projectRoot}. This approval applies once to the exact
          content below.
        </p>
        {error && <SettingsNotice tone="error">{error}</SettingsNotice>}
        <details>
          <summary>
            Current file
            {pending.preview.before_text === null ? " (absent)" : ""}
          </summary>
          <pre>
            {pending.preview.before_text ?? "This file does not exist."}
          </pre>
        </details>
        <details open>
          <summary>Proposed complete file</summary>
          <pre>{pending.preview.after_text}</pre>
        </details>
        <p className="settings-help">
          Save or close any editor with unsaved changes to this file before
          approving. Matching buffers remain frozen while the approved change is
          applied.
        </p>
        <div className="dialog-actions">
          <button
            className="button"
            disabled={busy}
            onClick={() => void decide(false)}
          >
            Deny change
          </button>
          <button
            className="button button-primary"
            disabled={busy}
            onClick={() => void decide(true)}
          >
            {busy ? "Finishing operation…" : "Approve this file change"}
          </button>
        </div>
      </div>
    </Modal>
  );
}
