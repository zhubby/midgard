import { OrganizationRolesPageClient } from "@/components/RbacAdminPageClients";

interface OrganizationRolesPageProps {
  params: Promise<{
    orgSlug: string;
  }>;
}

export default async function OrganizationRolesPage({
  params,
}: OrganizationRolesPageProps) {
  const { orgSlug } = await params;

  return <OrganizationRolesPageClient orgSlug={orgSlug} />;
}
