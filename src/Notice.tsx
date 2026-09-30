import type { ReactNode } from "react";
import { CircleAlert, Info, TriangleAlert } from "./icons";

export interface NoticeProps {
  tone?: "info" | "warning" | "error";
  role?: "alert" | "status" | "note";
  action?: ReactNode;
  className?: string;
  children: ReactNode;
}

export function Notice({
  tone = "info",
  role = tone === "error" ? "alert" : undefined,
  action,
  className = "",
  children,
}: NoticeProps) {
  const Icon =
    tone === "warning" ? TriangleAlert : tone === "error" ? CircleAlert : Info;

  return (
    <div
      className={`inline-notice${className ? ` ${className}` : ""}`}
      data-tone={tone}
      role={role}
    >
      <div className="inline-notice-body">
        <Icon size={16} className="inline-notice-icon" aria-hidden="true" />
        <div className="inline-notice-message">{children}</div>
      </div>
      {action && <div className="inline-notice-actions">{action}</div>}
    </div>
  );
}
