"use client";

import { type FormEvent, useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { AuthGate } from "@/components/AuthGate";
import { createWorkspace, fetchOrganizationContext } from "@/lib/api";
import type {
  AuthUser,
  OrganizationContext,
  PermissionKey,
  WorkspaceRuntimeMode,
} from "@/lib/types";

type OrganizationHomeState =
  | { status: "loading"; context: null; error: null }
  | { status: "ready"; context: OrganizationContext; error: null }
  | { status: "error"; context: null; error: string };

export function OrganizationHomePageClient({ orgSlug }: { orgSlug: string }) {
  return (
    <AuthGate>
      {({ auth, busyAuth, user, onLogout }) => (
        <OrganizationHomePage
          busyAuth={busyAuth}
          orgSlug={orgSlug}
          systemPermissions={auth.system_permissions}
          user={user}
          onLogout={onLogout}
        />
      )}
    </AuthGate>
  );
}

function OrganizationHomePage({
  busyAuth,
  orgSlug,
  systemPermissions,
  user,
  onLogout,
}: {
  busyAuth: boolean;
  orgSlug: string;
  systemPermissions: PermissionKey[];
  user: AuthUser;
  onLogout: () => void;
}) {
  const [state, setState] = useState<OrganizationHomeState>({
    status: "loading",
    context: null,
    error: null,
  });

  useEffect(() => {
    let cancelled = false;
    setState({ status: "loading", context: null, error: null });

    fetchOrganizationContext(orgSlug)
      .then((context) => {
        if (!cancelled) {
          setState({ status: "ready", context, error: null });
        }
      })
      .catch((caught) => {
        if (!cancelled) {
          setState({
            status: "error",
            context: null,
            error:
              caught instanceof Error
                ? caught.message
                : "Failed to load organization.",
          });
        }
      });

    return () => {
      cancelled = true;
    };
  }, [orgSlug]);

  if (state.status !== "ready") {
    return (
      <main className="login-shell login-loading" aria-busy="true">
        <div className="brand-lockup">
          <div className="brand-mark" aria-hidden="true">
            M
          </div>
          <div>
            <p className="section-kicker">{user.display_name || user.email}</p>
            <h1>{state.error ?? "Loading organization"}</h1>
          </div>
        </div>
      </main>
    );
  }

  const context = state.context;
  const canCreateOrganization = systemPermissions.includes("system.orgs.create");
  const canManageWorkspaces = context.permissions.includes("workspaces.manage");
  const canManageMembers =
    context.permissions.includes("org.members.manage") ||
    context.permissions.includes("org.members.read");
  const canManageRoles =
    context.permissions.includes("org.roles.manage") ||
    context.permissions.includes("org.roles.read");
  const hasWorkspaces = context.workspaces.length > 0;

  return (
    <main className="settings-shell">
      <header className="app-header">
        <div className="brand-lockup">
          <div className="brand-mark" aria-hidden="true">
            M
          </div>
          <div>
            <p className="section-kicker">Organization</p>
            <h1>{context.organization.name}</h1>
            <p className="workspace-breadcrumb">{context.organization.slug}</p>
          </div>
        </div>

        <div className="header-actions">
          <div className="user-chip" aria-label="Signed in user">
            <strong>{user.display_name || user.email}</strong>
            <span>{context.membership.role}</span>
          </div>
          <a className="button button-outline" href="/organizations">
            Organizations
          </a>
          {canCreateOrganization && (
            <a className="button button-outline" href="/organizations/new">
              New organization
            </a>
          )}
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

      <section className="settings-grid organization-home-grid">
        {hasWorkspaces ? (
          <WorkspaceListPanel context={context} />
        ) : canManageWorkspaces ? (
          <WorkspaceCreatePanel context={context} />
        ) : (
          <NoWorkspacePanel />
        )}

        <section className="workspace-panel settings-panel">
          <div className="section-row">
            <div>
              <p className="section-kicker">Organization access</p>
              <h2>Current membership</h2>
            </div>
            <span className="badge badge-outline">{context.membership.role}</span>
          </div>

          <div className="org-summary-list compact">
            <div>
              <span>Workspaces</span>
              <strong>{context.workspaces.length}</strong>
              <p>
                {hasWorkspaces
                  ? "Open a workspace to run agent operations."
                  : "Create a workspace before running agent operations."}
              </p>
            </div>
            <div>
              <span>Workspace setup</span>
              <strong>{canManageWorkspaces ? "allowed" : "blocked"}</strong>
              <p>Requires workspace management permission in this organization.</p>
            </div>
            <div>
              <span>Organization slug</span>
              <strong>{context.organization.slug}</strong>
              <p>Used in API routes and workspace URLs.</p>
            </div>
          </div>
        </section>
      </section>
    </main>
  );
}

function WorkspaceListPanel({ context }: { context: OrganizationContext }) {
  return (
    <section className="workspace-panel settings-panel">
      <div className="section-row">
        <div>
          <p className="section-kicker">Workspaces</p>
          <h2>Runtime boundaries</h2>
        </div>
        <span className="badge badge-outline">{context.workspaces.length}</span>
      </div>

      <div className="settings-instance-list">
        {context.workspaces.map((workspace) => (
          <article className="instance-row" key={workspace.id}>
            <div>
              <strong>{workspace.name}</strong>
              <p>
                {workspace.slug} · {workspace.runtime_config.mode ?? "unconfigured"}
              </p>
            </div>
            <div>
              <a
                className="button button-primary"
                href={`/orgs/${context.organization.slug}/workspaces/${workspace.slug}`}
              >
                Open
              </a>
              {context.permissions.includes("workspaces.manage") && (
                <a
                  className="button button-outline"
                  href={`/orgs/${context.organization.slug}/workspaces/${workspace.slug}/settings`}
                >
                  Settings
                </a>
              )}
            </div>
          </article>
        ))}
      </div>
    </section>
  );
}

function WorkspaceCreatePanel({ context }: { context: OrganizationContext }) {
  const router = useRouter();
  const [workspaceName, setWorkspaceName] = useState("Operations");
  const [runtimeMode, setRuntimeMode] =
    useState<WorkspaceRuntimeMode>("kubernetes");
  const [dockerApiUrl, setDockerApiUrl] = useState("");
  const [allowInsecureDocker, setAllowInsecureDocker] = useState(false);
  const [kubeconfig, setKubeconfig] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (busy) return;

    setBusy(true);
    setError(null);

    try {
      const workspace = await createWorkspace(context.organization.slug, {
        name: workspaceName,
        slug: null,
        runtime_config:
          runtimeMode === "docker"
            ? {
                mode: "docker",
                docker_api_url: dockerApiUrl,
                allow_insecure_local_endpoint: allowInsecureDocker,
              }
            : {
                mode: "kubernetes",
                kubeconfig,
              },
      });
      router.replace(
        `/orgs/${context.organization.slug}/workspaces/${workspace.slug}`,
      );
    } catch (caught) {
      setError(
        caught instanceof Error ? caught.message : "Failed to create workspace.",
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <form
      className="workspace-panel settings-panel settings-form"
      onSubmit={handleSubmit}
    >
      <div className="section-row">
        <div>
          <p className="section-kicker">First workspace</p>
          <h2>Create a workspace</h2>
        </div>
        <span className="badge badge-warn">required</span>
      </div>

      {error && (
        <div className="inline-alert settings-alert" role="alert">
          {error}
        </div>
      )}

      <label htmlFor="workspace-name">Workspace</label>
      <input
        id="workspace-name"
        name="workspace"
        placeholder="Operations"
        value={workspaceName}
        disabled={busy}
        onChange={(event) => setWorkspaceName(event.target.value)}
      />

      <fieldset className="runtime-fieldset">
        <legend>Runtime mode</legend>
        <div className="segmented-control" role="group" aria-label="Runtime mode">
          <button
            className={runtimeMode === "kubernetes" ? "active" : ""}
            disabled={busy}
            type="button"
            onClick={() => setRuntimeMode("kubernetes")}
          >
            Kubernetes
          </button>
          <button
            className={runtimeMode === "docker" ? "active" : ""}
            disabled={busy}
            type="button"
            onClick={() => setRuntimeMode("docker")}
          >
            Docker
          </button>
        </div>
      </fieldset>

      {runtimeMode === "docker" ? (
        <>
          <label htmlFor="docker-api-url">Docker API URL</label>
          <input
            id="docker-api-url"
            name="docker-api-url"
            placeholder="https://docker.example.com:2376"
            value={dockerApiUrl}
            disabled={busy}
            onChange={(event) => setDockerApiUrl(event.target.value)}
          />
          <label className="checkbox-row" htmlFor="allow-insecure-docker">
            <input
              id="allow-insecure-docker"
              checked={allowInsecureDocker}
              disabled={busy}
              type="checkbox"
              onChange={(event) =>
                setAllowInsecureDocker(event.target.checked)
              }
            />
            <span>Allow local HTTP endpoint</span>
          </label>
        </>
      ) : (
        <>
          <label htmlFor="kubeconfig">Kubeconfig</label>
          <textarea
            id="kubeconfig"
            name="kubeconfig"
            placeholder="apiVersion: v1&#10;kind: Config&#10;current-context: operations"
            rows={10}
            value={kubeconfig}
            disabled={busy}
            onChange={(event) => setKubeconfig(event.target.value)}
          />
        </>
      )}

      <button className="button button-primary" disabled={busy} type="submit">
        {busy ? "Creating" : "Create workspace"}
      </button>
    </form>
  );
}

function NoWorkspacePanel() {
  return (
    <section className="workspace-panel settings-panel">
      <div className="section-row">
        <div>
          <p className="section-kicker">Workspace required</p>
          <h2>No workspace is available</h2>
        </div>
      </div>
      <div className="empty-row">
        This organization does not have a workspace yet. Ask an organization
        owner or administrator to create one.
      </div>
    </section>
  );
}
