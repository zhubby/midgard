ALTER TABLE "agent_sessions"
ADD COLUMN "status" TEXT NOT NULL DEFAULT 'responded';

-- #[toasty::breakpoint]

ALTER TABLE "agent_sessions"
ADD COLUMN "pending_approval_json" TEXT;

-- #[toasty::breakpoint]

ALTER TABLE "agent_sessions"
ADD COLUMN "last_error" TEXT;

-- #[toasty::breakpoint]

ALTER TABLE "agent_messages"
ADD COLUMN "tool_calls_json" TEXT;

-- #[toasty::breakpoint]

ALTER TABLE "agent_messages"
ADD COLUMN "tool_call_id" TEXT;
