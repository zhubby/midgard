"use client";

import { AuthGate } from "@/components/AuthGate";
import {
  OrganizationMembersAdmin,
  OrganizationRolesAdmin,
  SystemRolesAdmin,
  SystemUsersAdmin,
} from "@/components/RbacAdmin";

export function AdminUsersPageClient() {
  return (
    <AuthGate>
      {({ busyAuth, user, onLogout }) => (
        <SystemUsersAdmin busyAuth={busyAuth} user={user} onLogout={onLogout} />
      )}
    </AuthGate>
  );
}

export function AdminRolesPageClient() {
  return (
    <AuthGate>
      {({ busyAuth, user, onLogout }) => (
        <SystemRolesAdmin busyAuth={busyAuth} user={user} onLogout={onLogout} />
      )}
    </AuthGate>
  );
}

export function OrganizationMembersPageClient({
  orgSlug,
}: {
  orgSlug: string;
}) {
  return (
    <AuthGate>
      {({ busyAuth, user, onLogout }) => (
        <OrganizationMembersAdmin
          busyAuth={busyAuth}
          orgSlug={orgSlug}
          user={user}
          onLogout={onLogout}
        />
      )}
    </AuthGate>
  );
}

export function OrganizationRolesPageClient({
  orgSlug,
}: {
  orgSlug: string;
}) {
  return (
    <AuthGate>
      {({ busyAuth, user, onLogout }) => (
        <OrganizationRolesAdmin
          busyAuth={busyAuth}
          orgSlug={orgSlug}
          user={user}
          onLogout={onLogout}
        />
      )}
    </AuthGate>
  );
}
