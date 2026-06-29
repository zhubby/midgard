CREATE TABLE "users" (
    "id" UUID NOT NULL,
    "email_lower" TEXT NOT NULL,
    "display_name" TEXT NOT NULL,
    "role" TEXT NOT NULL,
    "password_hash" TEXT NOT NULL,
    "active" BOOLEAN NOT NULL,
    "created_at" TEXT NOT NULL,
    "updated_at" TEXT NOT NULL,
    "last_login_at" TEXT,
    PRIMARY KEY ("id")
);

-- #[toasty::breakpoint]

CREATE UNIQUE INDEX "index_users_by_email_lower" ON "users" ("email_lower");

-- #[toasty::breakpoint]

CREATE TABLE "auth_sessions" (
    "id" UUID NOT NULL,
    "user_id" UUID NOT NULL,
    "token_hash" TEXT NOT NULL,
    "created_at" TEXT NOT NULL,
    "expires_at" TEXT NOT NULL,
    "revoked_at" TEXT,
    "user_agent" TEXT,
    "ip_address" TEXT,
    PRIMARY KEY ("id")
);

-- #[toasty::breakpoint]

CREATE INDEX "index_auth_sessions_by_user_id" ON "auth_sessions" ("user_id");

-- #[toasty::breakpoint]

CREATE UNIQUE INDEX "index_auth_sessions_by_token_hash" ON "auth_sessions" ("token_hash");

-- #[toasty::breakpoint]

CREATE TABLE "auth_audit_events" (
    "id" UUID NOT NULL,
    "user_id" UUID,
    "event_type" TEXT NOT NULL,
    "email_lower" TEXT,
    "occurred_at" TEXT NOT NULL,
    "ip_address" TEXT,
    "user_agent" TEXT,
    "detail_json" TEXT,
    PRIMARY KEY ("id")
);

-- #[toasty::breakpoint]

CREATE INDEX "index_auth_audit_events_by_user_id" ON "auth_audit_events" ("user_id");
