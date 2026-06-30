"use client";

import { AuthGate } from "@/components/AuthGate";
import { RootRedirect } from "@/components/RootRedirect";

export function HomePageClient() {
  return (
    <AuthGate>
      {({ auth, busyAuth, user, onLogout }) => (
        <RootRedirect
          busyAuth={busyAuth}
          systemPermissions={auth.system_permissions}
          user={user}
          onLogout={onLogout}
        />
      )}
    </AuthGate>
  );
}
