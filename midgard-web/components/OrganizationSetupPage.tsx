"use client";

import { type FormEvent, useState } from "react";
import { useRouter } from "next/navigation";
import { createOrganization } from "@/lib/api";
import type { AuthUser } from "@/lib/types";

interface OrganizationSetupPageProps {
  busyAuth: boolean;
  user: AuthUser;
  onLogout: () => void;
}

export function OrganizationSetupPage({
  busyAuth,
  user,
  onLogout,
}: OrganizationSetupPageProps) {
  const router = useRouter();
  const [organizationName, setOrganizationName] = useState("");
  const [workspaceName, setWorkspaceName] = useState("Operations");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (busy) return;

    setBusy(true);
    setError(null);

    try {
      const context = await createOrganization({
        name: organizationName,
        slug: null,
        workspace_name: workspaceName,
        workspace_slug: null,
      });
      const workspace = context.workspaces[0];
      router.replace(
        `/orgs/${context.organization.slug}/workspaces/${workspace.slug}`,
      );
    } catch (caught) {
      setError(
        caught instanceof Error
          ? caught.message
          : "Failed to create organization.",
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="login-shell org-setup-shell">
      <section className="login-brief" aria-labelledby="org-title">
        <div className="brand-lockup">
          <div className="brand-mark" aria-hidden="true">
            M
          </div>
          <div>
            <p className="section-kicker">Midgard</p>
            <h1 id="org-title">Create an organization</h1>
          </div>
        </div>

        <div className="login-status-grid" aria-label="Account context">
          <article>
            <span>Signed in</span>
            <strong>{user.display_name || user.email}</strong>
            <p>{user.email}</p>
          </article>
          <article>
            <span>Scope</span>
            <strong>Organization / Workspace</strong>
            <p>Agent sessions and middleware events are isolated by workspace.</p>
          </article>
          <article>
            <span>Default role</span>
            <strong>Owner</strong>
            <p>You can add operators and viewers after the workspace is ready.</p>
          </article>
        </div>
      </section>

      <section className="login-card" aria-labelledby="org-form-title">
        <div className="section-row org-card-heading">
          <div>
            <p className="section-kicker">Setup</p>
            <h2 id="org-form-title">Name the first workspace boundary</h2>
          </div>
          <button
            className="button button-outline"
            disabled={busy || busyAuth}
            type="button"
            onClick={onLogout}
          >
            Logout
          </button>
        </div>

        {error && (
          <div className="inline-alert login-alert" role="alert">
            {error}
          </div>
        )}

        <form className="login-form" onSubmit={handleSubmit}>
          <label htmlFor="organization-name">Organization</label>
          <input
            id="organization-name"
            autoComplete="organization"
            name="organization"
            placeholder="Platform Ops"
            value={organizationName}
            disabled={busy}
            onChange={(event) => setOrganizationName(event.target.value)}
          />

          <label htmlFor="workspace-name">Workspace</label>
          <input
            id="workspace-name"
            name="workspace"
            placeholder="Operations"
            value={workspaceName}
            disabled={busy}
            onChange={(event) => setWorkspaceName(event.target.value)}
          />

          <button className="button button-primary" disabled={busy} type="submit">
            {busy ? "Creating" : "Create organization"}
          </button>
        </form>
      </section>
    </main>
  );
}
