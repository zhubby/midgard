"use client";

import { useEffect, useState } from "react";
import { AuthGate } from "@/components/AuthGate";
import { fetchOrganizationContexts } from "@/lib/api";
import type {
  AuthUser,
  OrganizationContext,
  PermissionKey,
  Workspace,
} from "@/lib/types";

type OrganizationsState =
  | { status: "loading"; contexts: OrganizationContext[]; error: null }
  | { status: "ready"; contexts: OrganizationContext[]; error: null }
  | { status: "error"; contexts: OrganizationContext[]; error: string };

export function OrganizationsPageClient() {
  return (
    <AuthGate>
      {({ auth, busyAuth, user, onLogout }) => (
        <OrganizationsPage
          busyAuth={busyAuth}
          systemPermissions={auth.system_permissions}
          user={user}
          onLogout={onLogout}
        />
      )}
    </AuthGate>
  );
}

function OrganizationsPage({
  busyAuth,
  systemPermissions,
  user,
  onLogout,
}: {
  busyAuth: boolean;
  systemPermissions: PermissionKey[];
  user: AuthUser;
  onLogout: () => void;
}) {
  const [state, setState] = useState<OrganizationsState>({
    status: "loading",
    contexts: [],
    error: null,
  });

  useEffect(() => {
    let cancelled = false;

    fetchOrganizationContexts()
      .then((contexts) => {
        if (!cancelled) {
          setState({ status: "ready", contexts, error: null });
        }
      })
      .catch((caught) => {
        if (!cancelled) {
          setState({
            status: "error",
            contexts: [],
            error:
              caught instanceof Error
                ? caught.message
                : "Failed to load organizations.",
          });
        }
      });

    return () => {
      cancelled = true;
    };
  }, []);

  const canCreateOrganization = systemPermissions.includes("system.orgs.create");
  const canReadSystemAdmin =
    systemPermissions.includes("system.users.read") ||
    systemPermissions.includes("system.roles.read");
  const workspaceCount = state.contexts.reduce(
    (total, context) => total + context.workspaces.length,
    0,
  );

  return (
    <main className="app-shell">
      <header className="app-header">
        <div className="brand-lockup">
          <div className="brand-mark" aria-hidden="true">
            M
          </div>
          <div>
            <p className="section-kicker">Midgard</p>
            <h1>Organizations</h1>
            <p className="workspace-breadcrumb">
              Choose an organization or finish workspace setup.
            </p>
          </div>
        </div>

        <div className="header-actions">
          <div className="user-chip" aria-label="Signed in user">
            <strong>{user.display_name || user.email}</strong>
            <span>{user.email}</span>
          </div>
          {canCreateOrganization && (
            <a className="button button-primary" href="/organizations/new">
              New organization
            </a>
          )}
          {canReadSystemAdmin && (
            <a className="button button-outline" href="/admin/users">
              Admin
            </a>
          )}
          <button
            className="button button-outline logout-button"
            disabled={busyAuth}
            type="button"
            onClick={onLogout}
          >
            Logout
          </button>
        </div>
      </header>

      {state.error && (
        <div className="inline-alert settings-alert" role="alert">
          {state.error}
        </div>
      )}

      <section className="org-management-grid">
        <section className="workspace-panel dashboard-panel">
          <div className="panel-header">
            <div>
              <p className="section-kicker">Organization list</p>
              <h2>Available scopes</h2>
            </div>
            <span className="badge badge-outline">{state.status}</span>
          </div>

          <div className="org-list">
            {state.status === "loading" ? (
              <div className="empty-row org-empty-row">
                <strong>Loading organizations.</strong>
                <p>Checking organization memberships for this account.</p>
              </div>
            ) : state.contexts.length > 0 ? (
              state.contexts.map((context) => (
                <OrganizationRow context={context} key={context.organization.id} />
              ))
            ) : (
              <div className="empty-row org-empty-row">
                <strong>No organizations available.</strong>
                <p>
                  {canCreateOrganization
                    ? "Create an organization first, then create a workspace inside it."
                    : "Ask a Midgard administrator to add this account to an organization."}
                </p>
                {canCreateOrganization && (
                  <a className="button button-primary" href="/organizations/new">
                    Create organization
                  </a>
                )}
              </div>
            )}
          </div>
        </section>

        <aside className="workspace-panel dashboard-panel">
          <div className="panel-header">
            <div>
              <p className="section-kicker">Summary</p>
              <h2>Access overview</h2>
            </div>
          </div>

          <div className="org-summary-list">
            <div>
              <span>Organizations</span>
              <strong>{state.contexts.length}</strong>
              <p>Memberships visible to this account.</p>
            </div>
            <div>
              <span>Workspaces</span>
              <strong>{workspaceCount}</strong>
              <p>Runtime boundaries ready for agent operations.</p>
            </div>
            <div>
              <span>Create organization</span>
              <strong>{canCreateOrganization ? "allowed" : "blocked"}</strong>
              <p>System permission controls new organization creation.</p>
            </div>
          </div>
        </aside>
      </section>
    </main>
  );
}

function OrganizationRow({ context }: { context: OrganizationContext }) {
  const workspace = context.workspaces[0];
  const hasWorkspaces = context.workspaces.length > 0;
  const canManageMembers =
    context.permissions.includes("org.members.manage") ||
    context.permissions.includes("org.members.read");
  const canManageRoles =
    context.permissions.includes("org.roles.manage") ||
    context.permissions.includes("org.roles.read");

  return (
    <article className="org-row">
      <div className="org-row-main">
        <strong>{context.organization.name}</strong>
        <p>{context.organization.slug}</p>
      </div>

      <div className="org-row-meta">
        <span className={hasWorkspaces ? "badge badge-ready" : "badge badge-warn"}>
          {context.workspaces.length} workspace
          {context.workspaces.length === 1 ? "" : "s"}
        </span>
        <span className="badge badge-outline">{context.membership.role}</span>
      </div>

      <div className="instance-actions">
        {workspace ? (
          <a
            className="button button-primary"
            href={workspaceHref(context.organization.slug, workspace)}
          >
            Open workspace
          </a>
        ) : (
          <a className="button button-primary" href={`/orgs/${context.organization.slug}`}>
            Set up workspace
          </a>
        )}
        <a className="button button-outline" href={`/orgs/${context.organization.slug}`}>
          Organization
        </a>
        {canManageMembers && (
          <a
            className="button button-outline"
            href={`/orgs/${context.organization.slug}/settings/members`}
          >
            Members
          </a>
        )}
        {canManageRoles && (
          <a
            className="button button-outline"
            href={`/orgs/${context.organization.slug}/settings/roles`}
          >
            Roles
          </a>
        )}
      </div>
    </article>
  );
}

function workspaceHref(orgSlug: string, workspace: Workspace) {
  return `/orgs/${orgSlug}/workspaces/${workspace.slug}`;
}
