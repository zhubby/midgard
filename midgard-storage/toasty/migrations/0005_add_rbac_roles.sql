CREATE TABLE "rbac_roles" (
    "id" UUID NOT NULL,
    "scope_kind" TEXT NOT NULL,
    "organization_id" UUID,
    "slug" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "description" TEXT,
    "builtin_key" TEXT,
    "protected" BOOLEAN NOT NULL,
    "archived_at" TEXT,
    "created_at" TEXT NOT NULL,
    "updated_at" TEXT NOT NULL,
    PRIMARY KEY ("id")
);

-- #[toasty::breakpoint]

CREATE INDEX "index_rbac_roles_by_scope_kind" ON "rbac_roles" ("scope_kind");

-- #[toasty::breakpoint]

CREATE INDEX "index_rbac_roles_by_organization_id" ON "rbac_roles" ("organization_id");

-- #[toasty::breakpoint]

CREATE INDEX "index_rbac_roles_by_builtin_key" ON "rbac_roles" ("builtin_key");

-- #[toasty::breakpoint]

CREATE UNIQUE INDEX "index_rbac_roles_system_slug" ON "rbac_roles" ("scope_kind", "slug") WHERE "organization_id" IS NULL;

-- #[toasty::breakpoint]

CREATE UNIQUE INDEX "index_rbac_roles_org_slug" ON "rbac_roles" ("organization_id", "slug") WHERE "organization_id" IS NOT NULL;

-- #[toasty::breakpoint]

CREATE TABLE "rbac_role_permissions" (
    "id" BIGSERIAL NOT NULL,
    "role_id" UUID NOT NULL,
    "permission_key" TEXT NOT NULL,
    PRIMARY KEY ("id")
);

-- #[toasty::breakpoint]

CREATE INDEX "index_rbac_role_permissions_by_role_id" ON "rbac_role_permissions" ("role_id");

-- #[toasty::breakpoint]

CREATE UNIQUE INDEX "index_rbac_role_permissions_by_role_and_key" ON "rbac_role_permissions" ("role_id", "permission_key");

-- #[toasty::breakpoint]

INSERT INTO "rbac_roles" (
    "id", "scope_kind", "organization_id", "slug", "name", "description",
    "builtin_key", "protected", "archived_at", "created_at", "updated_at"
)
VALUES
    ('00000000-0000-0000-0000-000000000101', 'system', NULL, 'owner', 'System owner', 'Full system administration.', 'system_owner', TRUE, NULL, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z'),
    ('00000000-0000-0000-0000-000000000102', 'system', NULL, 'admin', 'System admin', 'Manage users and create organizations.', 'system_admin', TRUE, NULL, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z'),
    ('00000000-0000-0000-0000-000000000103', 'system', NULL, 'viewer', 'System viewer', 'Read-only system identity.', 'system_viewer', TRUE, NULL, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');

-- #[toasty::breakpoint]

INSERT INTO "rbac_role_permissions" ("role_id", "permission_key")
VALUES
    ('00000000-0000-0000-0000-000000000101', 'system.users.read'),
    ('00000000-0000-0000-0000-000000000101', 'system.users.manage'),
    ('00000000-0000-0000-0000-000000000101', 'system.roles.read'),
    ('00000000-0000-0000-0000-000000000101', 'system.roles.manage'),
    ('00000000-0000-0000-0000-000000000101', 'system.orgs.create'),
    ('00000000-0000-0000-0000-000000000101', 'system.orgs.read'),
    ('00000000-0000-0000-0000-000000000102', 'system.users.read'),
    ('00000000-0000-0000-0000-000000000102', 'system.users.manage'),
    ('00000000-0000-0000-0000-000000000102', 'system.roles.read'),
    ('00000000-0000-0000-0000-000000000102', 'system.orgs.create'),
    ('00000000-0000-0000-0000-000000000102', 'system.orgs.read'),
    ('00000000-0000-0000-0000-000000000103', 'system.orgs.read');

-- #[toasty::breakpoint]

ALTER TABLE "users"
ADD COLUMN "system_role_id" UUID;

-- #[toasty::breakpoint]

UPDATE "users"
SET "system_role_id" = CASE "role"
    WHEN 'admin' THEN '00000000-0000-0000-0000-000000000101'::UUID
    WHEN 'operator' THEN '00000000-0000-0000-0000-000000000102'::UUID
    ELSE '00000000-0000-0000-0000-000000000103'::UUID
END
WHERE "system_role_id" IS NULL;

-- #[toasty::breakpoint]

ALTER TABLE "users"
ALTER COLUMN "system_role_id" SET NOT NULL;

-- #[toasty::breakpoint]

CREATE INDEX "index_users_by_system_role_id" ON "users" ("system_role_id");

-- #[toasty::breakpoint]

INSERT INTO "rbac_roles" (
    "id", "scope_kind", "organization_id", "slug", "name", "description",
    "builtin_key", "protected", "archived_at", "created_at", "updated_at"
)
SELECT
    (
        SUBSTR(MD5("id"::TEXT || ':owner'), 1, 8) || '-' ||
        SUBSTR(MD5("id"::TEXT || ':owner'), 9, 4) || '-' ||
        SUBSTR(MD5("id"::TEXT || ':owner'), 13, 4) || '-' ||
        SUBSTR(MD5("id"::TEXT || ':owner'), 17, 4) || '-' ||
        SUBSTR(MD5("id"::TEXT || ':owner'), 21, 12)
    )::UUID,
    'organization',
    "id",
    'owner',
    'Owner',
    'Full organization administration.',
    'owner',
    TRUE,
    NULL,
    "created_at",
    "updated_at"
FROM "organizations";

-- #[toasty::breakpoint]

INSERT INTO "rbac_roles" (
    "id", "scope_kind", "organization_id", "slug", "name", "description",
    "builtin_key", "protected", "archived_at", "created_at", "updated_at"
)
SELECT
    (
        SUBSTR(MD5("id"::TEXT || ':admin'), 1, 8) || '-' ||
        SUBSTR(MD5("id"::TEXT || ':admin'), 9, 4) || '-' ||
        SUBSTR(MD5("id"::TEXT || ':admin'), 13, 4) || '-' ||
        SUBSTR(MD5("id"::TEXT || ':admin'), 17, 4) || '-' ||
        SUBSTR(MD5("id"::TEXT || ':admin'), 21, 12)
    )::UUID,
    'organization',
    "id",
    'admin',
    'Admin',
    'Manage members and workspaces.',
    'admin',
    TRUE,
    NULL,
    "created_at",
    "updated_at"
FROM "organizations";

-- #[toasty::breakpoint]

INSERT INTO "rbac_roles" (
    "id", "scope_kind", "organization_id", "slug", "name", "description",
    "builtin_key", "protected", "archived_at", "created_at", "updated_at"
)
SELECT
    (
        SUBSTR(MD5("id"::TEXT || ':operator'), 1, 8) || '-' ||
        SUBSTR(MD5("id"::TEXT || ':operator'), 9, 4) || '-' ||
        SUBSTR(MD5("id"::TEXT || ':operator'), 13, 4) || '-' ||
        SUBSTR(MD5("id"::TEXT || ':operator'), 17, 4) || '-' ||
        SUBSTR(MD5("id"::TEXT || ':operator'), 21, 12)
    )::UUID,
    'organization',
    "id",
    'operator',
    'Operator',
    'Operate workspaces.',
    'operator',
    TRUE,
    NULL,
    "created_at",
    "updated_at"
FROM "organizations";

-- #[toasty::breakpoint]

INSERT INTO "rbac_roles" (
    "id", "scope_kind", "organization_id", "slug", "name", "description",
    "builtin_key", "protected", "archived_at", "created_at", "updated_at"
)
SELECT
    (
        SUBSTR(MD5("id"::TEXT || ':viewer'), 1, 8) || '-' ||
        SUBSTR(MD5("id"::TEXT || ':viewer'), 9, 4) || '-' ||
        SUBSTR(MD5("id"::TEXT || ':viewer'), 13, 4) || '-' ||
        SUBSTR(MD5("id"::TEXT || ':viewer'), 17, 4) || '-' ||
        SUBSTR(MD5("id"::TEXT || ':viewer'), 21, 12)
    )::UUID,
    'organization',
    "id",
    'viewer',
    'Viewer',
    'Read workspace state.',
    'viewer',
    TRUE,
    NULL,
    "created_at",
    "updated_at"
FROM "organizations";

-- #[toasty::breakpoint]

INSERT INTO "rbac_role_permissions" ("role_id", "permission_key")
SELECT "id", permission_key
FROM "rbac_roles"
CROSS JOIN (
    VALUES
        ('org.read'), ('org.manage'), ('org.members.read'), ('org.members.manage'),
        ('org.roles.read'), ('org.roles.manage'), ('workspaces.read'), ('workspaces.manage'),
        ('workspace.read'), ('workspace.operate')
) AS permissions(permission_key)
WHERE "scope_kind" = 'organization' AND "builtin_key" = 'owner';

-- #[toasty::breakpoint]

INSERT INTO "rbac_role_permissions" ("role_id", "permission_key")
SELECT "id", permission_key
FROM "rbac_roles"
CROSS JOIN (
    VALUES
        ('org.read'), ('org.manage'), ('org.members.read'), ('org.members.manage'),
        ('org.roles.read'), ('workspaces.read'), ('workspaces.manage'),
        ('workspace.read'), ('workspace.operate')
) AS permissions(permission_key)
WHERE "scope_kind" = 'organization' AND "builtin_key" = 'admin';

-- #[toasty::breakpoint]

INSERT INTO "rbac_role_permissions" ("role_id", "permission_key")
SELECT "id", permission_key
FROM "rbac_roles"
CROSS JOIN (
    VALUES
        ('org.read'), ('org.members.read'), ('workspaces.read'), ('workspace.read'), ('workspace.operate')
) AS permissions(permission_key)
WHERE "scope_kind" = 'organization' AND "builtin_key" = 'operator';

-- #[toasty::breakpoint]

INSERT INTO "rbac_role_permissions" ("role_id", "permission_key")
SELECT "id", permission_key
FROM "rbac_roles"
CROSS JOIN (
    VALUES
        ('org.read'), ('org.members.read'), ('workspaces.read'), ('workspace.read')
) AS permissions(permission_key)
WHERE "scope_kind" = 'organization' AND "builtin_key" = 'viewer';

-- #[toasty::breakpoint]

ALTER TABLE "organization_memberships"
ADD COLUMN "role_id" UUID;

-- #[toasty::breakpoint]

UPDATE "organization_memberships" m
SET "role_id" = r."id"
FROM "rbac_roles" r
WHERE r."scope_kind" = 'organization'
  AND r."organization_id" = m."organization_id"
  AND r."builtin_key" = CASE m."role"
      WHEN 'owner' THEN 'owner'
      WHEN 'admin' THEN 'admin'
      WHEN 'operator' THEN 'operator'
      ELSE 'viewer'
  END
  AND m."role_id" IS NULL;

-- #[toasty::breakpoint]

ALTER TABLE "organization_memberships"
ALTER COLUMN "role_id" SET NOT NULL;

-- #[toasty::breakpoint]

CREATE INDEX "index_organization_memberships_by_role_id" ON "organization_memberships" ("role_id");
