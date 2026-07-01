"use client";

import { type FormEvent, useState } from "react";
import { useRouter } from "next/navigation";
import { LogOut, Plus } from "lucide-react";
import { NoWorkspaceAccess } from "@/components/NoWorkspaceAccess";
import { createOrganization } from "@/lib/api";
import type { AuthUser } from "@/lib/types";

interface OrganizationSetupPageProps {
  busyAuth: boolean;
  canCreateOrganization: boolean;
  user: AuthUser;
  onLogout: () => void;
}

export function OrganizationSetupPage({
  busyAuth,
  canCreateOrganization,
  user,
  onLogout,
}: OrganizationSetupPageProps) {
  const router = useRouter();
  const [organizationName, setOrganizationName] = useState("");
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
      });
      router.replace(`/orgs/${context.organization.slug}`);
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

  if (!canCreateOrganization) {
    return (
      <NoWorkspaceAccess
        busyAuth={busyAuth}
        user={user}
        onLogout={onLogout}
      />
    );
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
            <strong>Organization</strong>
            <p>Membership, roles, and workspace boundaries are managed separately.</p>
          </article>
          <article>
            <span>Next step</span>
            <strong>Workspace setup</strong>
            <p>Create a workspace only after entering the new organization.</p>
          </article>
        </div>
      </section>

      <section className="login-card" aria-labelledby="org-form-title">
        <div className="section-row org-card-heading">
          <div>
            <p className="section-kicker">Setup</p>
            <h2 id="org-form-title">Create an organization</h2>
          </div>
          <button
            className="button button-danger"
            disabled={busy || busyAuth}
            type="button"
            onClick={onLogout}
          >
            <LogOut aria-hidden="true" />
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

          <button className="button button-primary" disabled={busy} type="submit">
            <Plus aria-hidden="true" />
            {busy ? "Creating" : "Create organization"}
          </button>
        </form>
      </section>
    </main>
  );
}
