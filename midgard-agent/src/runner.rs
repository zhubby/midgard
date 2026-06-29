use std::sync::Arc;

use futures_util::StreamExt;
use midgard_core::MidgardResult;
use midgard_tools::{ToolRegistry, ToolResult};

use crate::{
    AgentMessage, AgentRunEvent, AgentRunStatus, AgentSession, AgentToolCall, LlmProvider,
    LlmRequest, LlmResponse, LlmStreamEvent, PendingApproval,
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

    pub async fn run(&self, session: AgentSession) -> MidgardResult<AgentRunResult> {
        self.run_with_observer(session, |_| {}).await
    }

    pub async fn run_with_observer<F>(
        &self,
        mut session: AgentSession,
        mut observer: F,
    ) -> MidgardResult<AgentRunResult>
    where
        F: FnMut(AgentRunEvent) + Send,
    {
        let mut events = Vec::new();
        session.status = AgentRunStatus::Running;
        session.last_error = None;

        if self
            .resume_pending_approval(&mut session, &mut events, &mut observer)
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
            let mut stream = self.provider.stream(request);
            let mut content = String::new();
            let mut streamed_tool_calls = Vec::new();
            let mut response = None;

            while let Some(event) = stream.next().await {
                match event {
                    Ok(LlmStreamEvent::ContentDelta(delta)) => {
                        content.push_str(&delta);
                        Self::emit_event(
                            &mut events,
                            &mut observer,
                            AgentRunEvent::ModelDelta { content: delta },
                        );
                    }
                    Ok(LlmStreamEvent::ToolCallDone(tool_call)) => {
                        if !streamed_tool_calls
                            .iter()
                            .any(|existing: &AgentToolCall| existing.id == tool_call.id)
                        {
                            Self::emit_event(
                                &mut events,
                                &mut observer,
                                AgentRunEvent::ToolCallRequested {
                                    tool_call: tool_call.clone(),
                                },
                            );
                            streamed_tool_calls.push(tool_call);
                        }
                    }
                    Ok(LlmStreamEvent::MessageDone(done)) => {
                        response = Some(done);
                    }
                    Err(err) => {
                        let error = err.to_string();
                        session.status = AgentRunStatus::Failed;
                        session.last_error = Some(error.clone());
                        Self::emit_event(
                            &mut events,
                            &mut observer,
                            AgentRunEvent::Failed { error },
                        );
                        return Ok(AgentRunResult { session, events });
                    }
                }
            }

            let mut response = response.unwrap_or_else(|| {
                LlmResponse::with_tool_calls(content, streamed_tool_calls.clone())
            });
            if response.tool_calls.is_empty() {
                response.tool_calls = streamed_tool_calls;
            }

            for tool_call in &response.tool_calls {
                if !events.iter().any(|event| {
                    matches!(
                        event,
                        AgentRunEvent::ToolCallRequested { tool_call: emitted }
                            if emitted.id == tool_call.id
                    )
                }) {
                    Self::emit_event(
                        &mut events,
                        &mut observer,
                        AgentRunEvent::ToolCallRequested {
                            tool_call: tool_call.clone(),
                        },
                    );
                }
            }

            let assistant_message = AgentMessage::assistant_with_tool_calls(
                response.content.clone(),
                response.tool_calls.clone(),
            );
            session.messages.push(assistant_message.clone());
            Self::emit_event(
                &mut events,
                &mut observer,
                AgentRunEvent::AssistantMessage {
                    message: assistant_message,
                },
            );
            session.iteration_count += 1;
            run_iterations += 1;

            if response.tool_calls.is_empty() {
                session.status = AgentRunStatus::Responded;
                Self::emit_event(
                    &mut events,
                    &mut observer,
                    AgentRunEvent::Completed {
                        status: session.status.clone(),
                        output: response.content,
                    },
                );
                return Ok(AgentRunResult { session, events });
            }

            for tool_call in response.tool_calls {
                if self
                    .handle_tool_call(&mut session, &mut events, &mut observer, tool_call, false)
                    .await?
                {
                    return Ok(AgentRunResult { session, events });
                }
            }
        }

        session.status = AgentRunStatus::MaxIterations;
        Self::emit_event(
            &mut events,
            &mut observer,
            AgentRunEvent::Completed {
                status: AgentRunStatus::MaxIterations,
                output: "agent reached max iterations".to_string(),
            },
        );

        Ok(AgentRunResult { session, events })
    }

    fn emit_event<F>(events: &mut Vec<AgentRunEvent>, observer: &mut F, event: AgentRunEvent)
    where
        F: FnMut(AgentRunEvent),
    {
        observer(event.clone());
        events.push(event);
    }

    fn messages_for_provider(&self, session: &AgentSession) -> Vec<AgentMessage> {
        let mut messages = Vec::with_capacity(session.messages.len() + 1);
        messages.push(AgentMessage::system(self.config.system_prompt.clone()));
        messages.extend(session.messages.clone());
        messages
    }

    async fn resume_pending_approval<F>(
        &self,
        session: &mut AgentSession,
        events: &mut Vec<AgentRunEvent>,
        observer: &mut F,
    ) -> MidgardResult<bool>
    where
        F: FnMut(AgentRunEvent),
    {
        let Some(approval) = session.pending_approval.clone() else {
            return Ok(false);
        };

        match approval.approved {
            None => {
                session.status = AgentRunStatus::AwaitingApproval;
                Self::emit_event(
                    events,
                    observer,
                    AgentRunEvent::ApprovalRequired { approval },
                );
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
                    observer,
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
                    observer,
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

    async fn handle_tool_call<F>(
        &self,
        session: &mut AgentSession,
        events: &mut Vec<AgentRunEvent>,
        observer: &mut F,
        tool_call: AgentToolCall,
        emit_requested: bool,
    ) -> MidgardResult<bool>
    where
        F: FnMut(AgentRunEvent),
    {
        if emit_requested {
            Self::emit_event(
                events,
                observer,
                AgentRunEvent::ToolCallRequested {
                    tool_call: tool_call.clone(),
                },
            );
        }

        let Some(definition) = self.tools.definition(&tool_call.name) else {
            let result = ToolResult::error(format!("tool not found: {}", tool_call.name));
            self.record_tool_result(
                session,
                events,
                observer,
                &tool_call,
                result,
                AgentRunStatus::Running,
            );
            return Ok(false);
        };

        if definition.requires_approval {
            let approval = PendingApproval::new(tool_call, definition.risk_level);
            session.status = AgentRunStatus::AwaitingApproval;
            session.pending_approval = Some(approval.clone());
            Self::emit_event(
                events,
                observer,
                AgentRunEvent::ApprovalRequired { approval },
            );
            return Ok(true);
        }

        let result = self.execute_tool_call(&tool_call).await;
        let should_stop = !result.should_continue;
        self.record_tool_result(
            session,
            events,
            observer,
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

    fn record_tool_result<F>(
        &self,
        session: &mut AgentSession,
        events: &mut Vec<AgentRunEvent>,
        observer: &mut F,
        tool_call: &AgentToolCall,
        result: ToolResult,
        next_status: AgentRunStatus,
    ) where
        F: FnMut(AgentRunEvent),
    {
        session.messages.push(AgentMessage::tool_result(
            tool_call.id.clone(),
            result.output.clone(),
        ));
        session.status = next_status.clone();
        if result.is_error {
            session.last_error = Some(result.output.clone());
        }
        Self::emit_event(
            events,
            observer,
            AgentRunEvent::ToolResult {
                tool_call_id: tool_call.id.clone(),
                name: tool_call.name.clone(),
                result: result.clone(),
            },
        );

        if !result.should_continue {
            Self::emit_event(
                events,
                observer,
                AgentRunEvent::Completed {
                    status: next_status,
                    output: result.output,
                },
            );
        }
    }
}

impl From<AgentRunResult> for AgentSession {
    fn from(value: AgentRunResult) -> Self {
        value.session
    }
}
