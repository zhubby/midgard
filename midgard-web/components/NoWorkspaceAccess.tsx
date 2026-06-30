"use client";

import type { AuthUser } from "@/lib/types";

interface NoWorkspaceAccessProps {
  busyAuth: boolean;
  user: AuthUser;
  onLogout: () => void;
}

export function NoWorkspaceAccess({
  busyAuth,
  user,
  onLogout,
}: NoWorkspaceAccessProps) {
  return (
    <main className="login-shell login-loading">
      <section className="login-card access-card" aria-labelledby="access-title">
        <div className="brand-lockup">
          <div className="brand-mark" aria-hidden="true">
            M
          </div>
          <div>
            <p className="section-kicker">{user.display_name || user.email}</p>
            <h1 id="access-title">No workspace access</h1>
          </div>
        </div>
        <p className="access-copy">
          Ask a Midgard administrator to add this account to an organization.
        </p>
        <button
          className="button button-outline"
          disabled={busyAuth}
          type="button"
          onClick={onLogout}
        >
          Logout
        </button>
      </section>
    </main>
  );
}
