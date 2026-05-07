# Midgard Specification

## Vision

Midgard is an agent-native middleware operations platform for Kubernetes. Users describe operational outcomes in natural language. The agent reasons about the current cluster state, calls registered tools, and coordinates middleware controllers to inspect, repair, scale, restart, and operate middleware workloads.

## Goals

- Use LLM agents as the primary operations interface.
- Run all managed middleware on Kubernetes.
- Expose middleware capabilities through pluggable Rust controllers.
- Register controller capabilities as tools available to the agent.
- Keep tools atomic enough for the agent to compose new workflows.
- Provide a web UI for observability, tool traces, plugin discovery, and user approvals.

## Non-Goals

- Replace Kubernetes as the source of truth.
- Hardcode end-to-end remediation workflows into tools.
- Ship production-grade middleware operators in the initial scaffold.
- Bind the platform to a single LLM provider.

## Agent-Native Principles

### Action Parity

Any operation available through the UI must also be achievable by the agent through tools. The UI and agent should share the same backend services and state.

### Atomic Tools

Tools provide capabilities, not decisions. For example, `list_pods`, `read_events`, `scale_workload`, and `restart_workload` are valid primitives. "Fix Redis" is an agent outcome composed from primitives.

### Explicit Completion

Every agent session includes a `complete_task` tool. The agent must call it with `success`, `partial`, or `blocked` so the orchestrator can stop without heuristic completion detection.

### Recoverable Sessions

Agent sessions track messages, tool calls, task status, iteration count, and checkpoints. Long-running operations should be resumable after interruption.

### Approval Boundaries

Risky operations such as deletion, restarts, scaling, and configuration changes must carry risk metadata. The platform can require approval before executing those tools.

## Rust Workspace

### `midgard-core`

Shared domain types:

- IDs and names.
- Platform configuration.
- Risk levels.
- Capability descriptors.
- Common errors.

### `midgard-tools`

Tool system:

- `Tool` trait.
- `ToolRegistry`.
- JSON parameter schema descriptors.
- Tool results with `success`, `is_error`, and `should_continue`.
- Risk metadata and approval requirements.

### `midgard-agent`

Agent runtime:

- OpenAI-compatible provider interface.
- ReAct loop primitives.
- Message and tool-call models.
- Session state and checkpoints.
- Built-in `complete_task` tool convention.

### `midgard-controller`

Middleware plugin contracts:

- `MiddlewareController` trait.
- `MiddlewarePlugin` trait.
- Plugin metadata and lifecycle.
- Capability discovery.
- Tool registration hooks.

### `midgard-k8s`

Kubernetes abstraction:

- Cluster identity and health.
- Namespace listing.
- Workload listing.
- Pod listing.
- Event reading.
- Restart and scaling contracts.

Initial implementations may return mock data while preserving API boundaries for `kube-rs` integration.

### `midgard-plugin-example`

Example plugin that demonstrates:

- Plugin metadata.
- Controller capability discovery.
- Tool registration.
- Low-risk read tool and high-risk operation metadata.

### `midgard-server`

Axum API:

- Health endpoint.
- Tool listing.
- Plugin listing.
- Agent session creation.
- Agent message execution.

## OpenAI-Compatible Provider

Configuration:

- `MIDGARD_LLM_BASE_URL`
- `MIDGARD_LLM_API_KEY`
- `MIDGARD_LLM_MODEL`

The provider uses the Chat Completions style payload by default. Provider-specific behavior should live behind the provider trait and not leak into controllers or tools.

## Plugin Protocol

Each plugin exposes:

- Stable plugin ID.
- Display name and description.
- Middleware kinds it supports.
- Controller instance.
- Tool registration function.
- Capability descriptors.

Controllers expose:

- `describe_capabilities`.
- `health`.
- `register_tools`.

Plugins must not bypass the tool registry for agent-facing actions.

## API Contract

Initial backend routes:

- `GET /healthz`
- `GET /api/tools`
- `GET /api/plugins`
- `POST /api/agent/sessions`
- `POST /api/agent/sessions/:id/messages`

Responses should be JSON and use stable field names suitable for frontend and agent clients.

## Frontend

The frontend lives in `midgard-ui` and uses Bun, TypeScript, and Next.js App Router.

Initial pages:

- Dashboard for platform status.
- Agent Console for prompts, messages, and tool traces.
- Plugin Catalog for installed plugin capabilities.
- Cluster Overview for Kubernetes state.

## Testing

Backend:

- Unit tests for tool registry behavior.
- Unit tests for controller/plugin capability discovery.
- Unit tests for `complete_task` semantics.
- Workspace verification with `cargo check --workspace` and `cargo test --workspace`.

Frontend:

- TypeScript and lint checks.
- Build verification when dependencies are available.

## First Milestone

The first scaffold is complete when:

- Rust workspace compiles.
- Example plugin registers tools.
- Server exposes health, tool, plugin, and agent session endpoints.
- UI renders the main operational surfaces.
- `spec.md` documents the design and constraints.
