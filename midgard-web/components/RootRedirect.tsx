"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { NoWorkspaceAccess } from "@/components/NoWorkspaceAccess";
import { fetchOrganizationContexts } from "@/lib/api";
import type { AuthUser, PermissionKey } from "@/lib/types";

interface RootRedirectProps {
  busyAuth: boolean;
  systemPermissions: PermissionKey[];
  user: AuthUser;
  onLogout: () => void;
}

type RedirectState =
  | { status: "opening"; error: null }
  | { status: "no_access"; error: null }
  | { status: "error"; error: string };

export function RootRedirect({
  busyAuth,
  systemPermissions,
  user,
  onLogout,
}: RootRedirectProps) {
  const router = useRouter();
  const [state, setState] = useState<RedirectState>({
    status: "opening",
    error: null,
  });

  useEffect(() => {
    let cancelled = false;

    fetchOrganizationContexts()
      .then((contexts) => {
        if (cancelled) return;
        if (contexts.length > 0) {
          router.replace("/organizations");
          return;
        }

        if (!systemPermissions.includes("system.orgs.create")) {
          setState({ status: "no_access", error: null });
          return;
        }

        router.replace("/organizations/new");
      })
      .catch((caught) => {
        if (!cancelled) {
          setState({
            status: "error",
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
  }, [router, systemPermissions]);

  if (state.status === "no_access") {
    return (
      <NoWorkspaceAccess
        busyAuth={busyAuth}
        user={user}
        onLogout={onLogout}
      />
    );
  }

  return (
    <main className="login-shell login-loading" aria-busy="true">
      <div className="brand-lockup">
        <div className="brand-mark" aria-hidden="true">
          M
        </div>
        <div>
          <p className="section-kicker">{user.display_name || user.email}</p>
          <h1>{state.error ?? "Opening organizations"}</h1>
        </div>
      </div>
    </main>
  );
}
