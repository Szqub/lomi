import { useCallback, useEffect, useId, useRef, useState } from "react";
import { api, errorMessage } from "./api";
import { CliAgentIcon } from "./CliAgentIcon";
import type { CliAgent } from "./cli-agents";
import type { ShellProfile } from "./model";
import { RefreshCw } from "./icons";
import { Modal } from "./ui";

interface InstalledCli {
  cli: CliAgent;
  name: string;
  command: string;
}

export default function AgentsDialog({
  profile,
  cwd,
  onClose,
  onLaunch,
}: {
  profile: ShellProfile;
  cwd: string;
  onClose: () => void;
  onLaunch: (cli: CliAgent, count: number) => Promise<void>;
}) {
  const id = useId();
  const refreshButton = useRef<HTMLButtonElement>(null);
  const request = useRef(0);
  const launching = useRef(false);
  const [clients, setClients] = useState<InstalledCli[]>([]);
  const [selected, setSelected] = useState<CliAgent>();
  const [count, setCount] = useState("4");
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const refresh = useCallback(async () => {
    const generation = ++request.current;
    setLoading(true);
    setClients([]);
    setError("");
    try {
      const installed = await api<InstalledCli[]>("installed_agent_clis", {
        profileId: profile.id,
        cwd,
      });
      if (generation !== request.current) return;
      setClients(installed);
      setSelected((previous) =>
        installed.some((client) => client.cli === previous)
          ? previous
          : (
              installed.find((client) => client.cli === "cursor") ??
              installed[0]
            )?.cli,
      );
    } catch (cause) {
      if (generation === request.current) setError(errorMessage(cause));
    } finally {
      if (generation === request.current) setLoading(false);
    }
  }, [profile.id, cwd]);

  useEffect(() => {
    void refresh();
    return () => {
      ++request.current;
    };
  }, [refresh]);

  return (
    <Modal
      title="Agents"
      className="agents-dialog"
      descriptionId={`${id}-description`}
      initialFocus={refreshButton}
      closeDisabled={busy}
      onClose={onClose}
    >
      <form
        className="dialog-form"
        onSubmit={async (event) => {
          event.preventDefault();
          if (launching.current || loading || !clients.length) return;
          const amount = Number(count);
          if (!Number.isSafeInteger(amount) || amount < 1) {
            setError("Enter a whole number of terminals, at least 1.");
            return;
          }
          if (!selected || !clients.some((client) => client.cli === selected))
            return;
          launching.current = true;
          setBusy(true);
          setError("");
          try {
            await onLaunch(selected, amount);
            onClose();
          } catch (cause) {
            setError(errorMessage(cause));
          } finally {
            launching.current = false;
            setBusy(false);
          }
        }}
      >
        <p id={`${id}-description`}>
          Run installed CLI agents in separate terminal panels within one tab.
        </p>
        <div className="agents-chooser-heading">
          <span id={`${id}-cli-label`}>Installed CLI</span>
          <button
            ref={refreshButton}
            type="button"
            className="icon-button"
            aria-label="Refresh installed CLI"
            disabled={loading || busy}
            onClick={() => void refresh()}
          >
            <RefreshCw size={15} aria-hidden="true" />
          </button>
        </div>
        <div className="agents-scan-status" role="status">
          {loading
            ? "Looking for installed agents…"
            : !error && !clients.length
              ? "No supported agent CLI found. Install a CLI, then refresh."
              : ""}
        </div>
        {!!clients.length && (
          <fieldset
            className="agents-cli-list"
            aria-labelledby={`${id}-cli-label`}
            disabled={busy}
          >
            {clients.map((client) => (
              <label className="agents-cli-option" key={client.cli}>
                <input
                  type="radio"
                  name={`${id}-cli`}
                  value={client.cli}
                  checked={selected === client.cli}
                  onChange={() => setSelected(client.cli)}
                />
                <CliAgentIcon cli={client.cli} />
                <span className="agents-cli-identity">
                  <span>{client.name}</span>
                  <code>{client.command}</code>
                </span>
              </label>
            ))}
          </fieldset>
        )}
        <label className="agents-count" htmlFor={`${id}-count`}>
          Number of terminals
          <input
            id={`${id}-count`}
            type="number"
            min="1"
            step="1"
            required
            value={count}
            disabled={busy}
            onChange={(event) => setCount(event.target.value)}
          />
        </label>
        <div className="agents-directory" title={cwd}>
          <span>Project</span>
          <code>{cwd}</code>
        </div>
        {error && (
          <p className="text-error" role="alert">
            {error}
          </p>
        )}
        <div className="dialog-actions">
          <button
            type="button"
            className="button"
            disabled={busy}
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            type="submit"
            className="button button-primary"
            disabled={loading || busy || !clients.length}
          >
            {busy ? "Launching…" : "Launch agents"}
          </button>
        </div>
      </form>
    </Modal>
  );
}
