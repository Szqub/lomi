import { useRef, useState } from "react";
import type { Workspace } from "./model";
import {
  sanitizeWorkspaceAppearance,
  type WorkspaceAppearance,
} from "./workspace-appearance";
import { Modal } from "./ui";
import WorkspaceAvatar, { workspaceIconOptions } from "./WorkspaceAvatar";

const colors = [
  "#b6ef5b",
  "#6ba8ff",
  "#66c9b6",
  "#f6bf66",
  "#ef8aab",
  "#b09aef",
  "#f08066",
  "#98a6b8",
];

async function workspaceImage(file: File) {
  if (!["image/png", "image/jpeg", "image/webp"].includes(file.type))
    throw new Error("Choose a PNG, JPEG or WebP image.");
  if (file.size > 5 * 1024 * 1024)
    throw new Error("Choose an image smaller than 5 MB.");
  const url = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error("Could not read this image."));
    reader.readAsDataURL(file);
  });
  const image = new Image();
  image.src = url;
  await image.decode();
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = 128;
  const context = canvas.getContext("2d");
  if (!context) throw new Error("Could not prepare this image.");
  const side = Math.min(image.naturalWidth, image.naturalHeight);
  context.drawImage(
    image,
    (image.naturalWidth - side) / 2,
    (image.naturalHeight - side) / 2,
    side,
    side,
    0,
    0,
    128,
    128,
  );
  return canvas.toDataURL("image/png");
}

export default function WorkspaceAppearanceDialog({
  workspace,
  onSave,
  onClose,
}: {
  workspace: Workspace;
  onSave: (appearance?: WorkspaceAppearance) => void;
  onClose: () => void;
}) {
  const [appearance, setAppearance] = useState<WorkspaceAppearance>(
    workspace.appearance ?? {},
  );
  const [error, setError] = useState("");
  const [uploading, setUploading] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const uploadVersion = useRef(0);
  const close = () => {
    uploadVersion.current++;
    onClose();
  };
  return (
    <Modal
      title="Customize workspace"
      onClose={close}
      className="workspace-appearance-dialog"
    >
      <form
        className="dialog-form"
        onSubmit={(event) => {
          event.preventDefault();
          if (uploading) return;
          onSave(sanitizeWorkspaceAppearance(appearance));
          close();
        }}
      >
        <div className="workspace-appearance-body">
          <div className="workspace-appearance-preview">
            <WorkspaceAvatar appearance={appearance} />
            <div>
              <strong>{workspace.name}</strong>
              <p>Give this workspace its own identity.</p>
            </div>
          </div>
          <fieldset className="workspace-appearance-field">
            <legend>Icon</legend>
            <div className="workspace-icon-options">
              {workspaceIconOptions.map(({ id, label, Icon }) => (
                <button
                  type="button"
                  key={id}
                  title={label}
                  aria-label={`${label} icon`}
                  aria-pressed={
                    !appearance.image && (appearance.icon || "layers") === id
                  }
                  onClick={() => {
                    uploadVersion.current++;
                    setUploading(false);
                    setError("");
                    setAppearance(({ image: _image, ...current }) => ({
                      ...current,
                      icon: id,
                    }));
                  }}
                >
                  <Icon size={20} />
                </button>
              ))}
            </div>
            <label className="workspace-custom-icon-label">
              Custom emoji or text
              <input
                aria-label="Custom emoji or text"
                placeholder="e.g. 🚀"
                maxLength={16}
                value={
                  workspaceIconOptions.some(({ id }) => id === appearance.icon)
                    ? ""
                    : (appearance.icon ?? "")
                }
                onChange={(event) => {
                  uploadVersion.current++;
                  setUploading(false);
                  setAppearance(({ image: _image, ...current }) => ({
                    ...current,
                    icon: event.target.value,
                  }));
                }}
              />
            </label>
            <div className="workspace-image-actions">
              <button
                type="button"
                className="button"
                onClick={() => input.current?.click()}
              >
                {uploading ? "Preparing image…" : "Upload image…"}
              </button>
              {appearance.image && (
                <button
                  type="button"
                  className="button"
                  onClick={() => {
                    uploadVersion.current++;
                    setUploading(false);
                    setError("");
                    setAppearance(({ image: _image, ...current }) => current);
                  }}
                >
                  Remove image
                </button>
              )}
              <input
                ref={input}
                type="file"
                accept="image/png,image/jpeg,image/webp"
                aria-label="Workspace image"
                hidden
                onChange={async (event) => {
                  const file = event.target.files?.[0];
                  event.target.value = "";
                  if (!file) return;
                  const version = ++uploadVersion.current;
                  setUploading(true);
                  setError("");
                  try {
                    const image = await workspaceImage(file);
                    if (version === uploadVersion.current)
                      setAppearance((current) => ({ ...current, image }));
                  } catch (error) {
                    if (version === uploadVersion.current)
                      setError(
                        error instanceof Error
                          ? error.message
                          : "Could not load this image.",
                      );
                  } finally {
                    if (version === uploadVersion.current) setUploading(false);
                  }
                }}
              />
            </div>
            {error && (
              <p className="workspace-image-error" role="alert">
                {error}
              </p>
            )}
          </fieldset>
          <fieldset className="workspace-appearance-field">
            <legend>Color</legend>
            <div className="workspace-color-options">
              {colors.map((color) => (
                <button
                  type="button"
                  key={color}
                  aria-label={`Use color ${color}`}
                  aria-pressed={appearance.color === color}
                  style={{ backgroundColor: color }}
                  onClick={() =>
                    setAppearance((current) => ({ ...current, color }))
                  }
                />
              ))}
            </div>
            <label className="workspace-color-label">
              Custom color{" "}
              <input
                type="color"
                aria-label="Custom color"
                value={appearance.color ?? colors[0]}
                onChange={(event) =>
                  setAppearance((current) => ({
                    ...current,
                    color: event.target.value,
                  }))
                }
              />
            </label>
          </fieldset>
        </div>
        <div className="dialog-actions">
          <button
            type="button"
            className="button workspace-appearance-reset"
            onClick={() => {
              uploadVersion.current++;
              setUploading(false);
              setError("");
              setAppearance({});
            }}
          >
            Reset
          </button>
          <button type="button" className="button" onClick={close}>
            Cancel
          </button>
          <button
            type="submit"
            className="button button-primary"
            disabled={uploading}
          >
            Save
          </button>
        </div>
      </form>
    </Modal>
  );
}
