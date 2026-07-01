import { OrganizationHomePageClient } from "@/components/OrganizationHomePageClient";

interface OrganizationPageProps {
  params: Promise<{
    orgSlug: string;
  }>;
}

export default async function OrganizationPage({ params }: OrganizationPageProps) {
  const { orgSlug } = await params;

  return <OrganizationHomePageClient orgSlug={orgSlug} />;
}
