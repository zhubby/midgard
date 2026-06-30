"use client";

import { AuthGate } from "@/components/AuthGate";
import { WorkspaceRoute } from "@/components/WorkspaceRoute";

interface WorkspacePageClientProps {
  orgSlug: string;
  workspaceSlug: string;
}

export function WorkspacePageClient({
  orgSlug,
  workspaceSlug,
}: WorkspacePageClientProps) {
  return (
    <AuthGate>
      {({ auth, busyAuth, user, onLogout }) => (
        <WorkspaceRoute
          busyAuth={busyAuth}
          orgSlug={orgSlug}
          systemPermissions={auth.system_permissions}
          user={user}
          workspaceSlug={workspaceSlug}
          onLogout={onLogout}
        />
      )}
    </AuthGate>
  );
}
