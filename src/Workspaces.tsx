import ResourceIcon from "./ResourceIcon";
import { useRef, useState, useSyncExternalStore } from "react";
import {
  FileDiff,
  ChevronRight,
  Ellipsis,
  GitBranch,
  GitCommitHorizontal,
  Globe,
  MessageSquare,
  Monitor,
  Plus,
  Puzzle,
  Terminal,
} from "./icons";
import type { Project, Workspace } from "./model";
import { basename, tabTitle } from "./model";
import type { GitRepositoryScan } from "./api";
import type { WorkspaceAppearance } from "./workspace-appearance";
import ContextMenu from "./ContextMenu";
import { IconButton } from "./ui";
import WorkspaceAvatar from "./WorkspaceAvatar";
import WorkspaceAppearanceDialog from "./WorkspaceAppearanceDialog";
import { getTerminalAgents, subscribeTerminalAgents } from "./terminal-runtime";
import { summarizeWorkspaceAgents } from "./workspace-summary";
import { CliAgentIcon } from "./CliAgentIcon";
import { cliNames } from "./cli-agents";
import useWorkspaceRepositories from "./useWorkspaceRepositories";

export default function Workspaces({
  projects,
  activeWorkspaceId,
  activeRoot,
  git,
  onSelect,
  onNew,
  onRename,
  onAppearanceChange,
  onDelete,
}: {
  projects: Project[];
  activeWorkspaceId?: string;
  activeRoot: string;
  git: GitRepositoryScan & { loading: boolean };
  onSelect: (path: string, workspaceId: string, tabId?: string) => void;
  onNew: () => void;
  onRename: (workspace: Workspace) => void;
  onAppearanceChange: (
    workspace: Workspace,
    appearance?: WorkspaceAppearance,
  ) => void;
  onDelete: (workspace: Workspace) => void;
}) {
  const [menu, setMenu] = useState<{ id: string; x: number; y: number }>();
  const [customizing, setCustomizing] = useState<Workspace>();
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const trigger = useRef<HTMLButtonElement>(null);
  const agents = useSyncExternalStore(
    subscribeTerminalAgents,
    getTerminalAgents,
  );
  const repositories = useWorkspaceRepositories(projects, activeRoot, git);
  const count = projects.reduce(
    (total, project) => total + project.workspaces.length,
    0,
  );
  const workspace =
    menu &&
    projects
      .flatMap((project) => project.workspaces)
      .find((workspace) => workspace.id === menu.id);
  const showMenu = (
    id: string,
    element: HTMLButtonElement,
    x?: number,
    y?: number,
  ) => {
    trigger.current = element;
    const bounds = element.getBoundingClientRect();
    setMenu({ id, x: x ?? bounds.left, y: y ?? bounds.bottom });
  };
  return (
    <div className="sidebar-panel workspace-panel">
      <div className="sidebar-heading">
        <span>
          WORKSPACES <span className="workspace-heading-count">{count}</span>
        </span>
        <IconButton title="New workspace" onClick={onNew}>
          <Plus size={15} />
        </IconButton>
      </div>
      <nav className="workspace-list" aria-label="Workspace list">
        {projects.flatMap((project) =>
          project.workspaces.map((workspace) => {
            const summary = summarizeWorkspaceAgents(workspace, agents);
            const repository = repositories[project.path];
            const uncertain = repository?.limited || !!repository?.error;
            const repositoryCount =
              !repository || repository.loading
                ? "…"
                : repository.error && repository.count === 0
                  ? "?"
                  : `${repository.count}${uncertain ? "+" : ""}`;
            return (
              <div
                className={`workspace-list-entry${workspace.id === activeWorkspaceId ? " is-active" : ""}`}
                key={workspace.id}
              >
                <div className="workspace-list-heading">
                  <button
                    type="button"
                    className="workspace-list-item"
                    aria-current={
                      workspace.id === activeWorkspaceId ? "true" : undefined
                    }
                    title={`${workspace.name}\n${project.path}`}
                    onClick={() => onSelect(project.path, workspace.id)}
                    aria-haspopup="menu"
                    aria-expanded={menu?.id === workspace.id}
                    onContextMenu={(event) => {
                      event.preventDefault();
                      showMenu(
                        workspace.id,
                        event.currentTarget,
                        event.clientX,
                        event.clientY,
                      );
                    }}
                    onKeyDown={(event) => {
                      if (
                        event.key === "ContextMenu" ||
                        (event.shiftKey && event.key === "F10")
                      ) {
                        event.preventDefault();
                        showMenu(workspace.id, event.currentTarget);
                      }
                    }}
                  >
                    <WorkspaceAvatar appearance={workspace.appearance} />
                    <span className="workspace-list-details">
                      <span>{workspace.name}</span>
                      <small className="workspace-folder">
                        {basename(project.path)}
                      </small>
                    </span>
                  </button>
                  <IconButton
                    className="icon-button workspace-menu-trigger"
                    title={`Actions for ${workspace.name}`}
                    aria-haspopup="menu"
                    aria-expanded={menu?.id === workspace.id}
                    onClick={(event) =>
                      showMenu(workspace.id, event.currentTarget)
                    }
                  >
                    <Ellipsis size={16} />
                  </IconButton>
                </div>
                <div className="workspace-overview">
                  <span
                    className="workspace-repository-count"
                    title={
                      repository?.error ||
                      (repository?.limited
                        ? "Repository discovery reached its limit. Showing discovered repositories."
                        : "Initialized Git repositories in this folder")
                    }
                    aria-label={`${repositoryCount} Git repositories${uncertain ? " (incomplete scan)" : ""}`}
                  >
                    <GitBranch size={13} aria-hidden="true" />
                    {repositoryCount}{" "}
                    {repository?.count === 1 && !uncertain ? "repo" : "repos"}
                  </span>
                  <span
                    className="workspace-agent-count"
                    title="Detected CLI agents in open terminal panes"
                  >
                    <Terminal size={13} aria-hidden="true" />
                    {summary.total} {summary.total === 1 ? "agent" : "agents"}
                  </span>
                  <button
                    type="button"
                    className="workspace-tabs-toggle"
                    aria-label={`${expanded.has(workspace.id) ? "Collapse" : "Expand"} tabs in ${workspace.name}`}
                    aria-expanded={expanded.has(workspace.id)}
                    aria-controls={`workspace-tabs-${workspace.id}`}
                    onClick={() =>
                      setExpanded((current) => {
                        const next = new Set(current);
                        if (next.has(workspace.id)) next.delete(workspace.id);
                        else next.add(workspace.id);
                        return next;
                      })
                    }
                  >
                    {workspace.tabs.length}{" "}
                    {workspace.tabs.length === 1 ? "tab" : "tabs"}
                    <ChevronRight size={12} aria-hidden="true" />
                  </button>
                </div>
                {summary.total > 0 && (
                  <div
                    className="workspace-agent-list"
                    aria-label={`Agents in ${workspace.name}`}
                  >
                    {summary.groups.map(({ cli, count }) => (
                      <span
                        className="workspace-agent-chip"
                        key={cli}
                        role="img"
                        aria-label={`${cliNames[cli]} · ${count} ${count === 1 ? "agent" : "agents"}`}
                        title={`${cliNames[cli]} · ${count} ${count === 1 ? "agent" : "agents"}`}
                      >
                        <CliAgentIcon cli={cli} />
                      </span>
                    ))}
                  </div>
                )}
                <ul
                  id={`workspace-tabs-${workspace.id}`}
                  className="workspace-tab-list"
                  aria-label={`Tabs in ${workspace.name}`}
                  hidden={!expanded.has(workspace.id)}
                >
                  {workspace.tabs.map((tab) => {
                    const tabAgents = summary.byTab[tab.id] ?? [];
                    return (
                      <li key={tab.id}>
                        <button
                          type="button"
                          className="workspace-tab-item"
                          aria-current={
                            workspace.id === activeWorkspaceId &&
                            tab.id === workspace.activeTabId
                              ? "true"
                              : undefined
                          }
                          title={
                            tab.type === "file" ? tab.relative : tabTitle(tab)
                          }
                          onClick={() =>
                            onSelect(project.path, workspace.id, tab.id)
                          }
                        >
                          {tab.type === "commit" ? (
                            <GitCommitHorizontal size={14} aria-hidden="true" />
                          ) : tab.type === "diff" ? (
                            <ResourceIcon
                              path={`${tab.root}/${tab.relative}`}
                              size={14}
                              fallback={FileDiff}
                            />
                          ) : tab.type === "file" ? (
                            <ResourceIcon
                              path={`${tab.root}/${tab.relative}`}
                              size={14}
                            />
                          ) : tab.type === "browser" ? (
                            <Globe size={14} aria-hidden="true" />
                          ) : tab.type === "android" ? (
                            <Monitor size={14} aria-hidden="true" />
                          ) : tab.type === "chat" ? (
                            <MessageSquare size={14} aria-hidden="true" />
                          ) : tab.type === "plugin" ? (
                            <Puzzle size={14} aria-hidden="true" />
                          ) : tabAgents.length === 1 ? (
                            <CliAgentIcon cli={tabAgents[0].cli} />
                          ) : (
                            <Terminal size={14} aria-hidden="true" />
                          )}
                          <span>{tabTitle(tab)}</span>
                          {tabAgents.length > 0 && (
                            <span
                              className="workspace-tab-agents"
                              title={tabAgents
                                .map(
                                  ({ cli, count }) =>
                                    `${cliNames[cli]} ×${count}`,
                                )
                                .join(", ")}
                              aria-label={tabAgents
                                .map(
                                  ({ cli, count }) =>
                                    `${count} ${cliNames[cli]}`,
                                )
                                .join(", ")}
                            >
                              {tabAgents
                                .map(({ count }) => count)
                                .reduce((sum, count) => sum + count, 0)}
                            </span>
                          )}
                        </button>
                      </li>
                    );
                  })}
                </ul>
              </div>
            );
          }),
        )}
        {count === 0 && (
          <div className="workspace-empty">
            <WorkspaceAvatar />
            <p>Your workspaces live here.</p>
            <small>Choose a folder to get started.</small>
            <button type="button" className="button" onClick={onNew}>
              <Plus size={14} />
              Add workspace
            </button>
          </div>
        )}
      </nav>
      {menu && workspace && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          label="Workspace actions"
          actions={[
            { label: "Rename workspace", run: () => onRename(workspace) },
            {
              label: "Customize workspace…",
              run: () => setCustomizing(workspace),
            },
            {
              label: "Delete workspace…",
              danger: true,
              run: () => onDelete(workspace),
            },
          ]}
          onClose={() => {
            setMenu(undefined);
            trigger.current?.focus({ preventScroll: true });
          }}
        />
      )}
      {customizing && (
        <WorkspaceAppearanceDialog
          key={customizing.id}
          workspace={customizing}
          onSave={(appearance) => onAppearanceChange(customizing, appearance)}
          onClose={() => setCustomizing(undefined)}
        />
      )}
    </div>
  );
}
