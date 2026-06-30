ALTER TABLE "workspaces"
ADD COLUMN "runtime_mode" TEXT;

-- #[toasty::breakpoint]

ALTER TABLE "workspaces"
ADD COLUMN "runtime_config_ciphertext" TEXT;

-- #[toasty::breakpoint]

ALTER TABLE "workspaces"
ADD COLUMN "runtime_config_summary_json" TEXT;

-- #[toasty::breakpoint]

ALTER TABLE "workspaces"
ADD COLUMN "runtime_config_status" TEXT NOT NULL DEFAULT 'unconfigured';

-- #[toasty::breakpoint]

ALTER TABLE "workspaces"
ADD COLUMN "runtime_config_updated_at" TEXT;

-- #[toasty::breakpoint]

CREATE INDEX "index_workspaces_by_runtime_mode" ON "workspaces" ("runtime_mode");

-- #[toasty::breakpoint]

CREATE TABLE "middleware_instances" (
    "id" UUID NOT NULL,
    "workspace_id" UUID NOT NULL,
    "kind" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "namespace" TEXT NOT NULL,
    "desired_state" TEXT NOT NULL,
    "status" TEXT NOT NULL,
    "config_json" TEXT NOT NULL,
    "archived_at" TEXT,
    "created_at" TEXT NOT NULL,
    "updated_at" TEXT NOT NULL,
    PRIMARY KEY ("id")
);

-- #[toasty::breakpoint]

CREATE INDEX "index_middleware_instances_by_workspace_id" ON "middleware_instances" ("workspace_id");

-- #[toasty::breakpoint]

CREATE UNIQUE INDEX "index_middleware_instances_by_workspace_namespace_name"
ON "middleware_instances" ("workspace_id", "namespace", "name")
WHERE "archived_at" IS NULL;
