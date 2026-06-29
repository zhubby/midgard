"use client";

import { AuthGate } from "@/components/AuthGate";
import { RootRedirect } from "@/components/RootRedirect";

export function HomePageClient() {
  return (
    <AuthGate>
      {({ user }) => <RootRedirect user={user} />}
    </AuthGate>
  );
}
