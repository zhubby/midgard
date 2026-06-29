# Repository Guidelines

## Project Structure & Module Organization

This repository is a Rust workspace with a Bun + Next.js frontend. The current crate layout is root-level `midgard-*` directories:

- `midgard-core`: shared domain types, platform config, risk levels, completion status, and common errors.
- `midgard-config`: TOML configuration loading, default config creation, and runtime config validation.
- `midgard-agent`: agent session/message model, OpenAI-compatible provider helpers, and agent completion tools.
- `midgard-storage`: Toasty/PostgreSQL storage boundary for agent sessions and messages, plus the in-memory session store used by tests.
- `midgard-tools`: tool trait, tool definitions, registry, execution result types, and risk metadata exposure.
- `midgard-controller`: middleware controller and plugin contracts.
- `midgard-k8s`: Kubernetes client abstraction, workload/pod/event summaries, and mock client.
- `midgard-plugin-example`: example Redis middleware plugin and controller tool registration.
- `midgard-server`: Axum HTTP API library and app state wiring.
- `midgard-cli`: Clap-based project entrypoint, config init command, server startup, and Toasty migration wrapper.
- `midgard-ui`: Bun + TypeScript + Next.js frontend.

Keep new code in the crate that owns the domain concern. Avoid leaking HTTP/UI concerns into runtime/domain crates, and avoid putting middleware-specific behavior into generic controller, tool, or Kubernetes abstractions.

## Agent-Native Boundaries

Midgard is an agent-native operations platform. Preserve these boundaries:

- Tools are the agent action surface. Define tool metadata so planners can infer when to call a tool, what arguments are required, and what risk level applies.
- `RiskLevel::High` and `RiskLevel::Critical` actions must remain approval-gated through `requires_approval`.
- Agent completion should remain explicit through completion status/tooling rather than inferred from free-form text alone.
- Middleware plugins should register capabilities and tools through `MiddlewarePlugin` and `MiddlewareController`; do not special-case a plugin in the agent loop.
- Kubernetes operations should go through `KubernetesClient` abstractions. Do not shell out to `kubectl` from library code unless there is a deliberate design change.

## Build, Test, and Development Commands

Use workspace-level Cargo commands from the repository root:

- `cargo check --workspace`: fast compile verification.
- `cargo build --workspace`: build all Rust crates.
- `cargo test --workspace`: run Rust unit and integration tests.
- `cargo test -p midgard-server --test api_contract`: run API contract tests.
- `cargo test -p midgard-tools --test tool_registry`: run tool registry contract tests.
- `cargo fmt --all`: apply Rust formatting.
- `cargo clippy --workspace --all-targets -- -D warnings`: lint strictly.
- `RUST_LOG=info cargo run -p midgard-cli -- server`: run the Axum API using `~/.midgard/config.toml`.
- `cargo run -p midgard-cli -- config init`: create the default config file.
- `cargo run -p midgard-cli -- migrate apply`: apply Toasty migrations to the configured PostgreSQL database.

Frontend commands:

- `cd midgard-ui && bun install`: install frontend dependencies.
- `cd midgard-ui && bun run dev`: run the Next.js development server.
- `cd midgard-ui && bun run build`: build the frontend.
- `cd midgard-ui && bun run lint`: run TypeScript checking (`tsc --noEmit`).

## Rust Style and Idioms

- The workspace currently targets Rust 2021. Do not switch editions casually.
- Use `rustfmt` output: 4-space indentation, formatter-managed trailing commas, and idiomatic import grouping.
- Prefer concrete `struct` and `enum` types over `serde_json::Value` whenever the shape is known. `Value` is appropriate at tool argument and JSON-schema boundaries.
- Match on types and enums rather than strings. Convert to strings only at serialization, display, or external protocol boundaries.
- Use traits for behavior boundaries already present in the codebase: `Tool`, `MiddlewareController`, `MiddlewarePlugin`, and `KubernetesClient`.
- Prefer `MidgardResult<T>` and `MidgardError` for shared Midgard errors. Use crate-specific `thiserror` enums when a domain needs more precise variants.
- For new production paths, avoid `.unwrap()` and `.expect()`. Prefer `?`, `ok_or_else`, guarded fallbacks, or explicit error responses. Test code may unwrap where failure should fail the test.
- Never hold a `MutexGuard` or other blocking guard across an `.await`.
- Use guard clauses and `let-else` to keep control flow flat when handling invalid input or missing state.
- Keep public API surfaces small. Expose only the types needed by other crates and tests.

## Workspace Dependency Management

All Rust dependencies should be declared in the root `Cargo.toml` under `[workspace.dependencies]`.

- Member crates should reference shared dependencies with `.workspace = true`.
- Internal path crates such as `midgard-core` and `midgard-tools` should also be declared once at the workspace root, then referenced by member crates with `.workspace = true`.
- When adding a new crate, add it to `[workspace.members]` and add its internal dependency entry under `[workspace.dependencies]` if other crates need to depend on it.
- Keep feature choices centralized at the workspace root unless a member crate has a narrow, justified feature requirement.

## Server and API Guidelines

- Keep the Axum app constructor in `midgard-server::app()` so tests can exercise routes with `tower::ServiceExt::oneshot`.
- Keep production startup in `midgard-cli`; `midgard-server` is a library crate and should not grow a separate binary entrypoint.
- Keep route paths stable under `/api/...` unless the API contract is intentionally changing.
- Return structured JSON response types instead of ad hoc strings for API responses.
- Keep `AppState` cloneable and cheap to pass. Use shared state deliberately, and avoid blocking the async runtime with long synchronous work.
- Add or update API contract tests whenever routes, request bodies, response shapes, or plugin/tool registration behavior changes.

## Tool and Plugin Guidelines

When adding tools in `midgard-tools`, `midgard-agent`, or plugin crates:

- Use precise tool names with stable snake_case identifiers.
- Write descriptions that explain when the agent should call the tool.
- Provide JSON schemas with required fields, expected types, constraints, and practical defaults when applicable.
- Set `RiskLevel` based on operational blast radius, not implementation convenience.
- Return `ToolResult::complete` only when the agent should stop iterating.
- Add tests for success, invalid/missing arguments, risk metadata, and registry visibility.

When adding middleware plugins:

- Keep plugin metadata stable: `id`, display `name`, and `middleware_kind`.
- Register all plugin-owned tools through the controller.
- Keep example plugins demonstrative, not a dumping ground for shared behavior.

## Frontend Guidelines

The UI lives in `midgard-ui` and uses the Next.js App Router under `app/`.

- Use TypeScript strict mode and keep `bun run lint` passing.
- Keep operational screens dense, readable, and task-focused. This is an operations console, not a marketing landing page.
- Prefer accessible semantic HTML, labeled form controls, and clear focus states.
- Keep API integration code isolated from presentational components as the frontend grows.
- Avoid hard-coded mock data once the matching server API is available; wire through typed fetch helpers instead.
- Keep text and controls responsive across mobile and desktop widths.

## Testing Guidelines

- Place unit tests next to implementation with `mod tests` when they are tightly scoped.
- Place integration and contract tests under each crate's `tests/` directory.
- Name tests by behavior, for example `registry_exposes_tool_definitions_with_risk_metadata`.
- For bug fixes, add regression coverage that fails without the fix.
- For API changes, test status codes, request validation, response shape, and important serialized fields.
- For tool/plugin changes, cover metadata exposure, risk approval flags, successful execution, and error paths.
- For frontend changes, at minimum run `bun run lint`; use browser verification for layout or interaction changes.

## Configuration and Security

- Never commit API keys, kubeconfigs, tokens, or production cluster details.
- Runtime configuration is loaded from TOML, defaulting to `~/.midgard/config.toml`; use `midgard config init` to create the file.
- `database.url` and `llm.api_key` are intentionally empty in generated defaults and must be filled by the operator before use.
- LLM provider configuration is OpenAI-compatible and should stay redacted in logs, tests, fixtures, README snippets, and PR descriptions.
- Treat Kubernetes operations as high-impact. Make namespace, workload name, and operation intent explicit in APIs and tool arguments.
- Do not silently downgrade risk levels or bypass approvals for mutating middleware actions.
- Redact secrets from logs, tests, fixtures, README snippets, and PR descriptions.

## Documentation Guidelines

- Keep the root `README.md` in sync with crate layout, development commands, configuration, and user-facing behavior.
- When adding a substantial crate or module, document its responsibilities and boundaries near the code or in the README.
- Use fenced code blocks with language tags for commands, JSON, TOML, and TypeScript/Rust snippets.
- Keep examples executable against the current crate names and server routes.

## Commit and Pull Request Guidelines

Use Conventional Commits. Keep one logical change per commit.

Commit format:

```text
<type>(<scope>): <subject>

<body>

<footer>
```

- Subject line: imperative mood, lowercase, no trailing period, max 72 characters.
- Body: optional, explain what changed and why.
- Footer: optional, use for `BREAKING CHANGE:`, `Closes #123`, and related metadata.

Common types:

| Type       | Description                                 |
| ---------- | ------------------------------------------- |
| `feat`     | New feature                                 |
| `fix`      | Bug fix                                     |
| `docs`     | Documentation changes                       |
| `style`    | Code style and formatting                   |
| `refactor` | Refactoring without behavior change         |
| `perf`     | Performance improvements                    |
| `test`     | Test additions or corrections               |
| `chore`    | Maintenance tasks, dependencies, tooling    |
| `ci`       | CI/CD configuration changes                 |
| `build`    | Build system or external dependency changes |
| `revert`   | Reverting a previous commit                 |

PRs should include:

- Purpose and impacted crates or UI areas.
- Test evidence with commands run and results.
- API, config, or documentation updates when behavior changes.
- Sample request/response or UI screenshots when user-facing behavior is modified.
