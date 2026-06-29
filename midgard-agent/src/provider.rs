use std::{
    collections::VecDeque,
    pin::Pin,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use midgard_core::{MidgardError, MidgardResult};
use midgard_tools::ToolDefinition;
use tokio_stream::{self as stream, Stream};

use crate::{AgentMessage, AgentToolCall};

pub type LlmStream = Pin<Box<dyn Stream<Item = MidgardResult<LlmStreamEvent>> + Send>>;

#[derive(Clone, Debug)]
pub struct LlmRequest {
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<ToolDefinition>,
}

impl LlmRequest {
    pub fn new(messages: Vec<AgentMessage>, tools: Vec<ToolDefinition>) -> Self {
        Self { messages, tools }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LlmResponse {
    pub content: String,
    pub tool_calls: Vec<AgentToolCall>,
}

impl LlmResponse {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            tool_calls: Vec::new(),
        }
    }

    pub fn with_tool_calls(content: impl Into<String>, tool_calls: Vec<AgentToolCall>) -> Self {
        Self {
            content: content.into(),
            tool_calls,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LlmStreamEvent {
    ContentDelta(String),
    ToolCallDone(AgentToolCall),
    MessageDone(LlmResponse),
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, request: LlmRequest) -> MidgardResult<LlmResponse>;
    fn stream(&self, request: LlmRequest) -> LlmStream;
}

#[derive(Clone)]
pub struct ScriptedLlmProvider {
    responses: Arc<Mutex<VecDeque<LlmResponse>>>,
}

impl ScriptedLlmProvider {
    pub fn new(responses: impl IntoIterator<Item = LlmResponse>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into_iter().collect())),
        }
    }

    pub fn single(response: LlmResponse) -> Self {
        Self::new([response])
    }
}

#[async_trait]
impl LlmProvider for ScriptedLlmProvider {
    async fn complete(&self, _request: LlmRequest) -> MidgardResult<LlmResponse> {
        let mut responses = self
            .responses
            .lock()
            .map_err(|_| MidgardError::Agent("scripted provider poisoned".to_string()))?;

        Ok(responses
            .pop_front()
            .unwrap_or_else(|| LlmResponse::text("")))
    }

    fn stream(&self, _request: LlmRequest) -> LlmStream {
        let response = match self.responses.lock() {
            Ok(mut responses) => responses
                .pop_front()
                .unwrap_or_else(|| LlmResponse::text("")),
            Err(_) => {
                let error = MidgardError::Agent("scripted provider poisoned".to_string());
                return Box::pin(stream::iter([Err(error)]));
            }
        };

        let mut events = Vec::new();
        if !response.content.is_empty() {
            events.push(Ok(LlmStreamEvent::ContentDelta(response.content.clone())));
        }
        for tool_call in &response.tool_calls {
            events.push(Ok(LlmStreamEvent::ToolCallDone(tool_call.clone())));
        }
        events.push(Ok(LlmStreamEvent::MessageDone(response)));

        Box::pin(stream::iter(events))
    }
}
