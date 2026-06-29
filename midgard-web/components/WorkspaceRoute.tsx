"use client";

import { useEffect, useState } from "react";
import { WorkspaceShell } from "@/components/WorkspaceShell";
import { fetchOrganizationContext } from "@/lib/api";
import type { AuthUser, OrganizationContext, Workspace } from "@/lib/types";

interface WorkspaceRouteProps {
  busyAuth: boolean;
  orgSlug: string;
  user: AuthUser;
  workspaceSlug: string;
  onLogout: () => void;
}

type WorkspaceRouteState =
  | { status: "loading"; context: null; workspace: null; error: null }
  | {
      status: "ready";
      context: OrganizationContext;
      workspace: Workspace;
      error: null;
    }
  | { status: "error"; context: null; workspace: null; error: string };

export function WorkspaceRoute({
  busyAuth,
  orgSlug,
  user,
  workspaceSlug,
  onLogout,
}: WorkspaceRouteProps) {
  const [state, setState] = useState<WorkspaceRouteState>({
    status: "loading",
    context: null,
    workspace: null,
    error: null,
  });

  useEffect(() => {
    let cancelled = false;
    setState({ status: "loading", context: null, workspace: null, error: null });

    fetchOrganizationContext(orgSlug)
      .then((context) => {
        if (cancelled) return;
        const workspace = context.workspaces.find(
          (candidate) => candidate.slug === workspaceSlug,
        );
        if (!workspace) {
          setState({
            status: "error",
            context: null,
            workspace: null,
            error: "Workspace not found.",
          });
          return;
        }
        setState({ status: "ready", context, workspace, error: null });
      })
      .catch((caught) => {
        if (!cancelled) {
          setState({
            status: "error",
            context: null,
            workspace: null,
            error:
              caught instanceof Error
                ? caught.message
                : "Failed to load workspace.",
          });
        }
      });

    return () => {
      cancelled = true;
    };
  }, [orgSlug, workspaceSlug]);

  if (state.status !== "ready") {
    return (
      <main className="login-shell login-loading" aria-busy="true">
        <div className="brand-lockup">
          <div className="brand-mark" aria-hidden="true">
            M
          </div>
          <div>
            <p className="section-kicker">{user.display_name || user.email}</p>
            <h1>{state.error ?? "Loading workspace"}</h1>
          </div>
        </div>
      </main>
    );
  }

  return (
    <WorkspaceShell
      busyAuth={busyAuth}
      context={state.context}
      workspace={state.workspace}
      user={user}
      onLogout={onLogout}
    />
  );
}
