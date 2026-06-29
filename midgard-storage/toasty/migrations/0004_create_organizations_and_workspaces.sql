CREATE TABLE "organizations" (
    "id" UUID NOT NULL,
    "slug" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "created_by_user_id" UUID NOT NULL,
    "archived_at" TEXT,
    "created_at" TEXT NOT NULL,
    "updated_at" TEXT NOT NULL,
    PRIMARY KEY ("id")
);

-- #[toasty::breakpoint]

CREATE UNIQUE INDEX "index_organizations_by_slug" ON "organizations" ("slug");

-- #[toasty::breakpoint]

CREATE INDEX "index_organizations_by_created_by_user_id" ON "organizations" ("created_by_user_id");

-- #[toasty::breakpoint]

CREATE TABLE "organization_memberships" (
    "id" UUID NOT NULL,
    "organization_id" UUID NOT NULL,
    "user_id" UUID NOT NULL,
    "role" TEXT NOT NULL,
    "active" BOOLEAN NOT NULL,
    "joined_at" TEXT NOT NULL,
    "created_at" TEXT NOT NULL,
    "updated_at" TEXT NOT NULL,
    PRIMARY KEY ("id")
);

-- #[toasty::breakpoint]

CREATE INDEX "index_organization_memberships_by_organization_id" ON "organization_memberships" ("organization_id");

-- #[toasty::breakpoint]

CREATE INDEX "index_organization_memberships_by_user_id" ON "organization_memberships" ("user_id");

-- #[toasty::breakpoint]

CREATE UNIQUE INDEX "index_organization_memberships_by_org_and_user" ON "organization_memberships" ("organization_id", "user_id");

-- #[toasty::breakpoint]

CREATE TABLE "workspaces" (
    "id" UUID NOT NULL,
    "organization_id" UUID NOT NULL,
    "slug" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "archived_at" TEXT,
    "created_at" TEXT NOT NULL,
    "updated_at" TEXT NOT NULL,
    PRIMARY KEY ("id")
);

-- #[toasty::breakpoint]

CREATE INDEX "index_workspaces_by_organization_id" ON "workspaces" ("organization_id");

-- #[toasty::breakpoint]

CREATE UNIQUE INDEX "index_workspaces_by_org_and_slug" ON "workspaces" ("organization_id", "slug");

-- #[toasty::breakpoint]

INSERT INTO "organizations" (
    "id",
    "slug",
    "name",
    "created_by_user_id",
    "archived_at",
    "created_at",
    "updated_at"
)
SELECT
    '00000000-0000-0000-0000-000000000001',
    'legacy',
    'Legacy',
    '00000000-0000-0000-0000-000000000000',
    NULL,
    COALESCE((SELECT MIN("created_at") FROM "users"), '1970-01-01T00:00:00Z'),
    COALESCE((SELECT MIN("created_at") FROM "users"), '1970-01-01T00:00:00Z')
WHERE EXISTS (SELECT 1 FROM "users") OR EXISTS (SELECT 1 FROM "agent_sessions");

-- #[toasty::breakpoint]

INSERT INTO "workspaces" (
    "id",
    "organization_id",
    "slug",
    "name",
    "archived_at",
    "created_at",
    "updated_at"
)
SELECT
    '00000000-0000-0000-0000-000000000002',
    '00000000-0000-0000-0000-000000000001',
    'default',
    'Default',
    NULL,
    COALESCE((SELECT MIN("created_at") FROM "users"), '1970-01-01T00:00:00Z'),
    COALESCE((SELECT MIN("created_at") FROM "users"), '1970-01-01T00:00:00Z')
WHERE EXISTS (SELECT 1 FROM "organizations" WHERE "id" = '00000000-0000-0000-0000-000000000001');

-- #[toasty::breakpoint]

INSERT INTO "organization_memberships" (
    "id",
    "organization_id",
    "user_id",
    "role",
    "active",
    "joined_at",
    "created_at",
    "updated_at"
)
SELECT
    "id",
    '00000000-0000-0000-0000-000000000001',
    "id",
    CASE "role"
        WHEN 'admin' THEN 'owner'
        WHEN 'operator' THEN 'operator'
        ELSE 'viewer'
    END,
    "active",
    "created_at",
    "created_at",
    "updated_at"
FROM "users"
WHERE EXISTS (SELECT 1 FROM "organizations" WHERE "id" = '00000000-0000-0000-0000-000000000001');

-- #[toasty::breakpoint]

ALTER TABLE "agent_sessions"
ADD COLUMN "workspace_id" UUID;

-- #[toasty::breakpoint]

UPDATE "agent_sessions"
SET "workspace_id" = '00000000-0000-0000-0000-000000000002'
WHERE "workspace_id" IS NULL;

-- #[toasty::breakpoint]

ALTER TABLE "agent_sessions"
ALTER COLUMN "workspace_id" SET NOT NULL;

-- #[toasty::breakpoint]

CREATE INDEX "index_agent_sessions_by_workspace_id" ON "agent_sessions" ("workspace_id");
