import { WorkspaceSettingsPageClient } from "@/components/WorkspaceSettingsPageClient";

interface WorkspaceSettingsPageProps {
  params: Promise<{
    orgSlug: string;
    workspaceSlug: string;
  }>;
}

export default async function WorkspaceSettingsPage({
  params,
}: WorkspaceSettingsPageProps) {
  const { orgSlug, workspaceSlug } = await params;

  return (
    <WorkspaceSettingsPageClient
      orgSlug={orgSlug}
      workspaceSlug={workspaceSlug}
    />
  );
}
