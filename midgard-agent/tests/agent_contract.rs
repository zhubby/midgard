use midgard_agent::{
    AgentMessage, AgentRole, AgentSession, CompleteTaskTool, OpenAiCompatibleProvider,
};
use midgard_core::{CompletionStatus, LlmConfig};
use midgard_tools::Tool;
use serde_json::json;

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
