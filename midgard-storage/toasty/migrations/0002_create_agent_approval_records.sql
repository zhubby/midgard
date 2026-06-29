CREATE TABLE "agent_approval_records" (
    "id" UUID NOT NULL,
    "session_id" UUID NOT NULL,
    "tool_call_json" TEXT NOT NULL,
    "risk_level" TEXT NOT NULL,
    "status" TEXT NOT NULL,
    "requested_at" TEXT NOT NULL,
    "decided_at" TEXT,
    "actor" TEXT,
    "reason" TEXT,
    PRIMARY KEY ("id")
);

-- #[toasty::breakpoint]

CREATE INDEX "index_agent_approval_records_by_session_id" ON "agent_approval_records" ("session_id");
