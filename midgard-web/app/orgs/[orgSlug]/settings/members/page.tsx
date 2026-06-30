import { OrganizationMembersPageClient } from "@/components/RbacAdminPageClients";

interface OrganizationMembersPageProps {
  params: Promise<{
    orgSlug: string;
  }>;
}

export default async function OrganizationMembersPage({
  params,
}: OrganizationMembersPageProps) {
  const { orgSlug } = await params;

  return <OrganizationMembersPageClient orgSlug={orgSlug} />;
}
