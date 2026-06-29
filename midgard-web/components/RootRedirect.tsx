"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { fetchOrganizationContexts } from "@/lib/api";
import type { AuthUser } from "@/lib/types";

interface RootRedirectProps {
  user: AuthUser;
}

export function RootRedirect({ user }: RootRedirectProps) {
  const router = useRouter();
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    fetchOrganizationContexts()
      .then((contexts) => {
        if (cancelled) return;
        const context = contexts.find((item) => item.workspaces.length > 0);
        const workspace = context?.workspaces[0];
        if (!context || !workspace) {
          router.replace("/organizations/new");
          return;
        }

        router.replace(
          `/orgs/${context.organization.slug}/workspaces/${workspace.slug}`,
        );
      })
      .catch((caught) => {
        if (!cancelled) {
          setError(
            caught instanceof Error
              ? caught.message
              : "Failed to load organizations.",
          );
        }
      });

    return () => {
      cancelled = true;
    };
  }, [router]);

  return (
    <main className="login-shell login-loading" aria-busy="true">
      <div className="brand-lockup">
        <div className="brand-mark" aria-hidden="true">
          M
        </div>
        <div>
          <p className="section-kicker">{user.display_name || user.email}</p>
          <h1>{error ?? "Opening workspace"}</h1>
        </div>
      </div>
    </main>
  );
}
