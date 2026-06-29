use std::sync::Arc;

use async_trait::async_trait;
use midgard_agent::{
    parse_chat_completion_response, parse_chat_stream_events, parse_responses_response,
    parse_responses_stream_events, AgentMessage, AgentRole, AgentRunStatus, AgentRunner,
    AgentSession, AgentToolCall, ApprovalDecision, ApprovalRecord, ApprovalStatus,
    CompleteTaskTool, LlmRequest, LlmResponse, LlmStreamEvent, OpenAiCompatibleProvider,
    PendingApproval, ScriptedLlmProvider,
};
use midgard_core::{CompletionStatus, LlmApiMode, LlmConfig, RiskLevel};
use midgard_tools::{Tool, ToolDefinition, ToolRegistry, ToolResult};
use serde_json::{json, Value};

#[test]
fn provider_builds_openai_compatible_chat_completions_url() {
    let provider = OpenAiCompatibleProvider::new(
        LlmConfig::new("http://gateway.local/v1/", "qwen-max"),
        "secret",
    );

    assert_eq!(
        provider.chat_completions_url(),
        "http://gateway.local/v1/chat/completions"
    );
    assert_eq!(provider.model(), "qwen-max");
}

#[test]
fn agent_session_starts_with_user_goal() {
    let session = AgentSession::new("inspect redis");

    assert_eq!(session.iteration_count, 0);
    assert_eq!(session.messages[0].role, AgentRole::User);
    assert_eq!(session.messages[0].content, "inspect redis");
}

#[tokio::test]
async fn complete_task_tool_stops_the_agent_loop() {
    let tool = CompleteTaskTool;

    let result = tool
        .call(json!({
            "status": "success",
            "summary": "Redis is healthy"
        }))
        .await;

    assert!(result.success);
    assert!(!result.should_continue);
    assert!(result.output.contains("Redis is healthy"));
}

#[test]
fn agent_message_preserves_assistant_tool_trace_text() {
    let message = AgentMessage::assistant("called list_pods");

    assert_eq!(message.role, AgentRole::Assistant);
    assert_eq!(message.content, "called list_pods");
}

#[test]
fn completion_status_is_available_to_agent_contract() {
    assert_eq!(CompletionStatus::Blocked.as_str(), "blocked");
}

#[test]
fn approval_record_serializes_pending_status() {
    let session = AgentSession::new("restart redis");
    let pending = PendingApproval::new(
        AgentToolCall::from_raw(
            "call_1",
            "restart_redis",
            r#"{"namespace":"default","name":"cache"}"#,
        ),
        RiskLevel::High,
    );

    let record = ApprovalRecord::pending(session.id, &pending);
    let json = serde_json::to_value(&record).unwrap();

    assert_eq!(record.status, ApprovalStatus::Pending);
    assert_eq!(ApprovalStatus::Pending.as_str(), "pending");
    assert_eq!(json["status"], "pending");
    assert!(json.get("actor").is_none());
}

#[test]
fn approval_record_decision_sets_actor_reason_and_timestamp() {
    let session = AgentSession::new("restart redis");
    let pending = PendingApproval::new(
        AgentToolCall::from_raw("call_1", "restart_redis", "{}"),
        RiskLevel::High,
    );
    let mut record = ApprovalRecord::pending(session.id, &pending);

    record.record_decision(
        ApprovalDecision::Approve,
        "operator@example.com",
        Some("maintenance window".to_string()),
    );

    assert_eq!(record.status, ApprovalStatus::Approved);
    assert_eq!(record.actor.as_deref(), Some("operator@example.com"));
    assert_eq!(record.reason.as_deref(), Some("maintenance window"));
    assert!(record.decided_at.is_some());
}

#[test]
fn chat_completions_request_serializes_tools() {
    let provider = OpenAiCompatibleProvider::new(
        LlmConfig::new("http://gateway.local/v1", "qwen-max"),
        "secret",
    );
    let tool = CompleteTaskTool;
    let request = LlmRequest::new(
        vec![AgentMessage::user("inspect redis")],
        vec![tool.definition()],
    );

    let body = provider.chat_completions_request_body(&request, false);

    assert_eq!(body["model"], "qwen-max");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["tools"][0]["type"], "function");
    assert_eq!(body["tools"][0]["function"]["name"], "complete_task");
}

#[test]
fn responses_request_serializes_tools_and_function_outputs() {
    let provider = OpenAiCompatibleProvider::new(
        LlmConfig::new("http://gateway.local/v1", "gpt-4o-mini")
            .with_api_mode(LlmApiMode::Responses),
        "secret",
    );
    let request = LlmRequest::new(
        vec![AgentMessage::tool_result("call_1", "Redis is ready")],
        vec![CompleteTaskTool.definition()],
    );

    let body = provider.responses_request_body(&request, true);

    assert_eq!(body["stream"], true);
    assert_eq!(body["input"][0]["type"], "function_call_output");
    assert_eq!(body["input"][0]["call_id"], "call_1");
    assert_eq!(body["tools"][0]["name"], "complete_task");
}

#[test]
fn chat_completion_parser_extracts_tool_calls() {
    let response = json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": "redis_describe",
                        "arguments": "{\"namespace\":\"default\",\"name\":\"cache\"}"
                    }
                }]
            }
        }]
    });

    let parsed = parse_chat_completion_response(&response).unwrap();

    assert_eq!(parsed.tool_calls[0].name, "redis_describe");
    assert_eq!(parsed.tool_calls[0].arguments["name"], "cache");
}

#[test]
fn responses_parser_extracts_messages_and_function_calls() {
    let response = json!({
        "output": [
            {
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Checking Redis"}]
            },
            {
                "type": "function_call",
                "call_id": "call_1",
                "name": "redis_describe",
                "arguments": "{\"namespace\":\"default\",\"name\":\"cache\"}"
            }
        ]
    });

    let parsed = parse_responses_response(&response).unwrap();

    assert_eq!(parsed.content, "Checking Redis");
    assert_eq!(parsed.tool_calls[0].name, "redis_describe");
}

#[test]
fn chat_stream_parser_aggregates_tool_arguments() {
    let input = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"hi \"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"redis_describe\",\"arguments\":\"{\\\"namespace\\\":\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"default\\\"}\"}}]}}]}\n\n",
        "data: [DONE]\n\n",
    );

    let events = parse_chat_stream_events(input).unwrap();

    assert!(events
        .iter()
        .any(|event| matches!(event, LlmStreamEvent::ContentDelta(delta) if delta == "hi ")));
    assert!(events.iter().any(|event| {
        matches!(event, LlmStreamEvent::ToolCallDone(call) if call.arguments["namespace"] == "default")
    }));
}

#[test]
fn responses_stream_parser_aggregates_function_arguments() {
    let input = concat!(
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"id\":\"item_1\",\"call_id\":\"call_1\",\"name\":\"redis_describe\"}}\n\n",
        "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"call_1\",\"delta\":\"{\\\"name\\\":\"}\n\n",
        "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"call_1\",\"delta\":\"\\\"cache\\\"}\"}\n\n",
    );

    let events = parse_responses_stream_events(input).unwrap();

    assert!(events.iter().any(|event| {
        matches!(event, LlmStreamEvent::ToolCallDone(call) if call.name == "redis_describe")
    }));
}

#[tokio::test]
async fn react_loop_stops_when_complete_task_returns_complete() {
    let mut registry = ToolRegistry::default();
    registry.register(CompleteTaskTool);
    let provider = ScriptedLlmProvider::single(LlmResponse::with_tool_calls(
        "",
        vec![AgentToolCall::from_raw(
            "call_1",
            "complete_task",
            r#"{"summary":"Redis is healthy"}"#,
        )],
    ));
    let runner = AgentRunner::new(Arc::new(provider), Arc::new(registry));

    let result = runner
        .run(AgentSession::new("inspect redis"))
        .await
        .unwrap();

    assert_eq!(result.session.status, AgentRunStatus::Completed);
    assert!(result
        .session
        .messages
        .last()
        .unwrap()
        .content
        .contains("Redis is healthy"));
}

#[tokio::test]
async fn react_loop_returns_tool_errors_for_unknown_tools() {
    let registry = ToolRegistry::default();
    let provider = ScriptedLlmProvider::new([
        LlmResponse::with_tool_calls(
            "",
            vec![AgentToolCall::from_raw("call_1", "missing_tool", "{}")],
        ),
        LlmResponse::text("I could not call that tool."),
    ]);
    let runner = AgentRunner::new(Arc::new(provider), Arc::new(registry));

    let result = runner
        .run(AgentSession::new("inspect redis"))
        .await
        .unwrap();

    assert_eq!(result.session.status, AgentRunStatus::Responded);
    assert!(result
        .session
        .messages
        .iter()
        .any(|message| message.content.contains("tool not found")));
}

#[tokio::test]
async fn react_loop_reports_invalid_arguments_as_tool_result() {
    let mut registry = ToolRegistry::default();
    registry.register(EchoTool);
    let provider = ScriptedLlmProvider::new([
        LlmResponse::with_tool_calls(
            "",
            vec![AgentToolCall::from_raw("call_1", "echo", "not-json")],
        ),
        LlmResponse::text("Arguments were invalid."),
    ]);
    let runner = AgentRunner::new(Arc::new(provider), Arc::new(registry));

    let result = runner
        .run(AgentSession::new("inspect redis"))
        .await
        .unwrap();

    assert_eq!(result.session.status, AgentRunStatus::Responded);
    assert!(result
        .session
        .last_error
        .unwrap()
        .contains("invalid arguments"));
}

#[tokio::test]
async fn high_risk_tool_pauses_for_approval_and_resumes() {
    let mut registry = ToolRegistry::default();
    registry.register(HighRiskTool);
    registry.register(CompleteTaskTool);
    let provider = ScriptedLlmProvider::new([
        LlmResponse::with_tool_calls(
            "",
            vec![AgentToolCall::from_raw(
                "call_1",
                "restart_redis",
                r#"{"namespace":"default","name":"cache"}"#,
            )],
        ),
        LlmResponse::with_tool_calls(
            "",
            vec![AgentToolCall::from_raw(
                "call_2",
                "complete_task",
                r#"{"summary":"Restart requested"}"#,
            )],
        ),
    ]);
    let runner = AgentRunner::new(Arc::new(provider), Arc::new(registry));

    let first = runner
        .run(AgentSession::new("restart redis"))
        .await
        .unwrap();
    assert_eq!(first.session.status, AgentRunStatus::AwaitingApproval);
    assert!(first.session.pending_approval.is_some());

    let mut session = first.session;
    session
        .record_approval_decision(ApprovalDecision::Approve)
        .unwrap();
    let second = runner.run(session).await.unwrap();

    assert_eq!(second.session.status, AgentRunStatus::Completed);
    assert!(second
        .session
        .messages
        .iter()
        .any(|message| message.content.contains("restart requested")));
}

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "echo",
            "Echo input",
            json!({"type":"object"}),
            RiskLevel::Low,
        )
    }

    async fn call(&self, arguments: Value) -> ToolResult {
        ToolResult::success(arguments.to_string())
    }
}

struct HighRiskTool;

#[async_trait]
impl Tool for HighRiskTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "restart_redis",
            "Restart Redis",
            json!({
                "type": "object",
                "properties": {
                    "namespace": {"type": "string"},
                    "name": {"type": "string"}
                },
                "required": ["namespace", "name"]
            }),
            RiskLevel::High,
        )
    }

    async fn call(&self, _arguments: Value) -> ToolResult {
        ToolResult::success("restart requested")
    }
}
