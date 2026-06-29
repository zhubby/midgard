import { WorkspacePageClient } from "@/components/WorkspacePageClient";

interface WorkspacePageProps {
  params: Promise<{
    orgSlug: string;
    workspaceSlug: string;
  }>;
}

export default async function WorkspacePage({ params }: WorkspacePageProps) {
  const { orgSlug, workspaceSlug } = await params;

  return <WorkspacePageClient orgSlug={orgSlug} workspaceSlug={workspaceSlug} />;
}
