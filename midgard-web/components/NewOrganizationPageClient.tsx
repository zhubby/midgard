"use client";

import { AuthGate } from "@/components/AuthGate";
import { OrganizationSetupPage } from "@/components/OrganizationSetupPage";

export function NewOrganizationPageClient() {
  return (
    <AuthGate>
      {({ auth, busyAuth, user, onLogout }) => (
        <OrganizationSetupPage
          busyAuth={busyAuth}
          canCreateOrganization={auth.system_permissions.includes(
            "system.orgs.create",
          )}
          user={user}
          onLogout={onLogout}
        />
      )}
    </AuthGate>
  );
}
