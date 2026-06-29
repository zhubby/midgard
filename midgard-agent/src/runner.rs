use std::sync::Arc;

use midgard_core::MidgardResult;
use midgard_tools::{ToolRegistry, ToolResult};

use crate::{
    AgentMessage, AgentRunEvent, AgentRunStatus, AgentSession, AgentToolCall, LlmProvider,
    LlmRequest, PendingApproval,
};

const DEFAULT_MAX_ITERATIONS: usize = 8;

pub const DEFAULT_SYSTEM_PROMPT: &str = r#"You are Midgard's middleware operations agent.

You work toward the user's operational goal by reasoning, calling available tools, checking tool results, and continuing until the goal is handled.

When the request is complete, call complete_task with a concise summary.
If you are blocked, call complete_task with status "blocked" and explain why.
Do not claim completion from free-form text alone."#;

#[derive(Clone, Debug)]
pub struct AgentRunConfig {
    pub max_iterations: usize,
    pub system_prompt: String,
}

impl Default for AgentRunConfig {
    fn default() -> Self {
        Self {
            max_iterations: DEFAULT_MAX_ITERATIONS,
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AgentRunResult {
    pub session: AgentSession,
    pub events: Vec<AgentRunEvent>,
}

pub struct AgentRunner {
    provider: Arc<dyn LlmProvider>,
    tools: Arc<ToolRegistry>,
    config: AgentRunConfig,
}

impl AgentRunner {
    pub fn new(provider: Arc<dyn LlmProvider>, tools: Arc<ToolRegistry>) -> Self {
        Self::with_config(provider, tools, AgentRunConfig::default())
    }

    pub fn with_config(
        provider: Arc<dyn LlmProvider>,
        tools: Arc<ToolRegistry>,
        config: AgentRunConfig,
    ) -> Self {
        Self {
            provider,
            tools,
            config,
        }
    }

    pub async fn run(&self, mut session: AgentSession) -> MidgardResult<AgentRunResult> {
        let mut events = Vec::new();
        session.status = AgentRunStatus::Running;
        session.last_error = None;

        if self
            .resume_pending_approval(&mut session, &mut events)
            .await?
        {
            return Ok(AgentRunResult { session, events });
        }

        let mut run_iterations = 0;
        while run_iterations < self.config.max_iterations {
            let request = LlmRequest::new(
                self.messages_for_provider(&session),
                self.tools.definitions(),
            );
            let response = match self.provider.complete(request).await {
                Ok(response) => response,
                Err(err) => {
                    let error = err.to_string();
                    session.status = AgentRunStatus::Failed;
                    session.last_error = Some(error.clone());
                    events.push(AgentRunEvent::Failed { error });
                    return Ok(AgentRunResult { session, events });
                }
            };

            let assistant_message = AgentMessage::assistant_with_tool_calls(
                response.content.clone(),
                response.tool_calls.clone(),
            );
            session.messages.push(assistant_message.clone());
            events.push(AgentRunEvent::AssistantMessage {
                message: assistant_message,
            });
            session.iteration_count += 1;
            run_iterations += 1;

            if response.tool_calls.is_empty() {
                session.status = AgentRunStatus::Responded;
                events.push(AgentRunEvent::Completed {
                    status: session.status.clone(),
                    output: response.content,
                });
                return Ok(AgentRunResult { session, events });
            }

            for tool_call in response.tool_calls {
                if self
                    .handle_tool_call(&mut session, &mut events, tool_call)
                    .await?
                {
                    return Ok(AgentRunResult { session, events });
                }
            }
        }

        session.status = AgentRunStatus::MaxIterations;
        events.push(AgentRunEvent::Completed {
            status: AgentRunStatus::MaxIterations,
            output: "agent reached max iterations".to_string(),
        });

        Ok(AgentRunResult { session, events })
    }

    fn messages_for_provider(&self, session: &AgentSession) -> Vec<AgentMessage> {
        let mut messages = Vec::with_capacity(session.messages.len() + 1);
        messages.push(AgentMessage::system(self.config.system_prompt.clone()));
        messages.extend(session.messages.clone());
        messages
    }

    async fn resume_pending_approval(
        &self,
        session: &mut AgentSession,
        events: &mut Vec<AgentRunEvent>,
    ) -> MidgardResult<bool> {
        let Some(approval) = session.pending_approval.clone() else {
            return Ok(false);
        };

        match approval.approved {
            None => {
                session.status = AgentRunStatus::AwaitingApproval;
                events.push(AgentRunEvent::ApprovalRequired { approval });
                Ok(true)
            }
            Some(false) => {
                let result = ToolResult::error(format!(
                    "tool call rejected by operator: {}",
                    approval.tool_call.name
                ));
                self.record_tool_result(
                    session,
                    events,
                    &approval.tool_call,
                    result,
                    AgentRunStatus::Running,
                );
                session.pending_approval = None;
                Ok(false)
            }
            Some(true) => {
                let result = self.execute_tool_call(&approval.tool_call).await;
                let should_stop = !result.should_continue;
                self.record_tool_result(
                    session,
                    events,
                    &approval.tool_call,
                    result.clone(),
                    if should_stop {
                        AgentRunStatus::Completed
                    } else {
                        AgentRunStatus::Running
                    },
                );
                session.pending_approval = None;
                Ok(should_stop)
            }
        }
    }

    async fn handle_tool_call(
        &self,
        session: &mut AgentSession,
        events: &mut Vec<AgentRunEvent>,
        tool_call: AgentToolCall,
    ) -> MidgardResult<bool> {
        events.push(AgentRunEvent::ToolCallRequested {
            tool_call: tool_call.clone(),
        });

        let Some(definition) = self.tools.definition(&tool_call.name) else {
            let result = ToolResult::error(format!("tool not found: {}", tool_call.name));
            self.record_tool_result(session, events, &tool_call, result, AgentRunStatus::Running);
            return Ok(false);
        };

        if definition.requires_approval {
            let approval = PendingApproval::new(tool_call, definition.risk_level);
            session.status = AgentRunStatus::AwaitingApproval;
            session.pending_approval = Some(approval.clone());
            events.push(AgentRunEvent::ApprovalRequired { approval });
            return Ok(true);
        }

        let result = self.execute_tool_call(&tool_call).await;
        let should_stop = !result.should_continue;
        self.record_tool_result(
            session,
            events,
            &tool_call,
            result,
            if should_stop {
                AgentRunStatus::Completed
            } else {
                AgentRunStatus::Running
            },
        );

        Ok(should_stop)
    }

    async fn execute_tool_call(&self, tool_call: &AgentToolCall) -> ToolResult {
        if !tool_call.arguments.is_object() {
            return ToolResult::error(format!(
                "invalid arguments for {}: expected JSON object, got {}",
                tool_call.name, tool_call.raw_arguments
            ));
        }

        match self
            .tools
            .call(&tool_call.name, tool_call.arguments.clone())
            .await
        {
            Ok(result) => result,
            Err(err) => ToolResult::error(err.to_string()),
        }
    }

    fn record_tool_result(
        &self,
        session: &mut AgentSession,
        events: &mut Vec<AgentRunEvent>,
        tool_call: &AgentToolCall,
        result: ToolResult,
        next_status: AgentRunStatus,
    ) {
        session.messages.push(AgentMessage::tool_result(
            tool_call.id.clone(),
            result.output.clone(),
        ));
        session.status = next_status.clone();
        if result.is_error {
            session.last_error = Some(result.output.clone());
        }
        events.push(AgentRunEvent::ToolResult {
            tool_call_id: tool_call.id.clone(),
            name: tool_call.name.clone(),
            result: result.clone(),
        });

        if !result.should_continue {
            events.push(AgentRunEvent::Completed {
                status: next_status,
                output: result.output,
            });
        }
    }
}

impl From<AgentRunResult> for AgentSession {
    fn from(value: AgentRunResult) -> Self {
        value.session
    }
}
