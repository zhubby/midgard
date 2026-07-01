use async_stream::try_stream;
use async_trait::async_trait;
use futures_util::StreamExt;
use midgard_core::{LlmApiMode, LlmConfig, MidgardError, MidgardResult};
use midgard_tools::ToolDefinition;
use reqwest::Client;
use serde_json::{Value, json};

use crate::{
    AgentMessage, AgentRole, AgentToolCall, LlmProvider, LlmRequest, LlmResponse, LlmStream,
    LlmStreamEvent,
};

#[derive(Clone)]
pub struct OpenAiCompatibleProvider {
    config: LlmConfig,
    api_key: String,
    client: Client,
}

impl OpenAiCompatibleProvider {
    pub fn new(config: LlmConfig, api_key: impl Into<String>) -> Self {
        Self {
            config,
            api_key: api_key.into(),
            client: Client::new(),
        }
    }

    pub fn chat_completions_url(&self) -> String {
        self.config.base_url.trim().to_string()
    }

    pub fn responses_url(&self) -> String {
        self.config.base_url.trim().to_string()
    }

    pub fn model(&self) -> &str {
        &self.config.model
    }

    pub fn authorization_header(&self) -> String {
        format!("Bearer {}", self.api_key)
    }

    pub fn chat_completions_request_body(&self, request: &LlmRequest, stream: bool) -> Value {
        json!({
            "model": self.config.model,
            "messages": request.messages.iter().map(chat_message).collect::<Vec<_>>(),
            "tools": request.tools.iter().map(chat_tool).collect::<Vec<_>>(),
            "stream": stream
        })
    }

    pub fn responses_request_body(&self, request: &LlmRequest, stream: bool) -> Value {
        json!({
            "model": self.config.model,
            "input": responses_input(&request.messages),
            "tools": request.tools.iter().map(responses_tool).collect::<Vec<_>>(),
            "stream": stream
        })
    }

    fn endpoint_url(&self) -> String {
        self.config.base_url.trim().to_string()
    }

    fn request_body(&self, request: &LlmRequest, stream: bool) -> Value {
        match self.config.api_mode {
            LlmApiMode::ChatCompletions => self.chat_completions_request_body(request, stream),
            LlmApiMode::Responses => self.responses_request_body(request, stream),
        }
    }

    fn parse_response(&self, value: &Value) -> MidgardResult<LlmResponse> {
        match self.config.api_mode {
            LlmApiMode::ChatCompletions => parse_chat_completion_response(value),
            LlmApiMode::Responses => parse_responses_response(value),
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    async fn complete(&self, request: LlmRequest) -> MidgardResult<LlmResponse> {
        let response = self
            .client
            .post(self.endpoint_url())
            .bearer_auth(&self.api_key)
            .json(&self.request_body(&request, false))
            .send()
            .await
            .map_err(agent_error)?;
        let status = response.status();
        let body = response.text().await.map_err(agent_error)?;

        if !status.is_success() {
            return Err(MidgardError::Agent(format!(
                "OpenAI-compatible request failed with {status}: {body}"
            )));
        }

        let value: Value = serde_json::from_str(&body)
            .map_err(|err| MidgardError::Agent(format!("invalid provider JSON: {err}")))?;

        self.parse_response(&value)
    }

    fn stream(&self, request: LlmRequest) -> LlmStream {
        let provider = self.clone();
        Box::pin(try_stream! {
            let response = provider
                .client
                .post(provider.endpoint_url())
                .bearer_auth(&provider.api_key)
                .json(&provider.request_body(&request, true))
                .send()
                .await
                .map_err(agent_error)?;
            let status = response.status();

            if !status.is_success() {
                let body = response.text().await.map_err(agent_error)?;
                Err(MidgardError::Agent(format!(
                    "OpenAI-compatible stream failed with {status}: {body}"
                )))?;
            } else {
                let mut chunks = response.bytes_stream();
                let mut buffer = String::new();
                let mut state = StreamState::default();
                while let Some(chunk) = chunks.next().await {
                    let chunk = chunk.map_err(agent_error)?;
                    let text = std::str::from_utf8(&chunk)
                        .map_err(|err| MidgardError::Agent(format!("invalid stream UTF-8: {err}")))?;
                    buffer.push_str(text);

                    while let Some(event) = pop_sse_event(&mut buffer) {
                        if event == "[DONE]" {
                            continue;
                        }
                        let value: Value = serde_json::from_str(&event)
                            .map_err(|err| MidgardError::Agent(format!("invalid provider stream JSON: {err}")))?;
                        let parsed = match provider.config.api_mode {
                            LlmApiMode::ChatCompletions => state.apply_chat_value(&value)?,
                            LlmApiMode::Responses => state.apply_responses_value(&value)?,
                        };
                        for event in parsed {
                            yield event;
                        }
                    }
                }

                for event in state.finish() {
                    yield event;
                }
            }
        })
    }
}

pub fn parse_chat_completion_response(value: &Value) -> MidgardResult<LlmResponse> {
    let message = value.pointer("/choices/0/message").ok_or_else(|| {
        MidgardError::Agent("chat completion response missing message".to_string())
    })?;
    let content = string_or_empty(message.get("content"));
    let tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .map(parse_chat_tool_call)
                .collect::<MidgardResult<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();

    Ok(LlmResponse::with_tool_calls(content, tool_calls))
}

pub fn parse_responses_response(value: &Value) -> MidgardResult<LlmResponse> {
    let output = value
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| MidgardError::Agent("responses response missing output".to_string()))?;
    let mut content = String::new();
    let mut tool_calls = Vec::new();

    for item in output {
        match item.get("type").and_then(Value::as_str).unwrap_or_default() {
            "message" => push_response_message_content(&mut content, item),
            "function_call" => tool_calls.push(parse_response_tool_call(item)?),
            _ => {}
        }
    }

    Ok(LlmResponse::with_tool_calls(content, tool_calls))
}

pub fn parse_chat_stream_events(input: &str) -> MidgardResult<Vec<LlmStreamEvent>> {
    let mut state = StreamState::default();
    let mut events = Vec::new();

    for event in sse_events(input) {
        if event == "[DONE]" {
            continue;
        }
        let value: Value = serde_json::from_str(&event)
            .map_err(|err| MidgardError::Agent(format!("invalid chat stream JSON: {err}")))?;
        events.extend(state.apply_chat_value(&value)?);
    }

    events.extend(state.finish());
    Ok(events)
}

pub fn parse_responses_stream_events(input: &str) -> MidgardResult<Vec<LlmStreamEvent>> {
    let mut state = StreamState::default();
    let mut events = Vec::new();

    for event in sse_events(input) {
        if event == "[DONE]" {
            continue;
        }
        let value: Value = serde_json::from_str(&event)
            .map_err(|err| MidgardError::Agent(format!("invalid responses stream JSON: {err}")))?;
        events.extend(state.apply_responses_value(&value)?);
    }

    events.extend(state.finish());
    Ok(events)
}

fn chat_message(message: &AgentMessage) -> Value {
    match &message.role {
        AgentRole::System => json!({"role": "system", "content": message.content}),
        AgentRole::User => json!({"role": "user", "content": message.content}),
        AgentRole::Assistant => {
            if message.tool_calls.is_empty() {
                json!({"role": "assistant", "content": message.content})
            } else {
                json!({
                    "role": "assistant",
                    "content": if message.content.is_empty() { Value::Null } else { Value::String(message.content.clone()) },
                    "tool_calls": message.tool_calls.iter().map(chat_tool_call).collect::<Vec<_>>()
                })
            }
        }
        AgentRole::Tool => json!({
            "role": "tool",
            "tool_call_id": message.tool_call_id.clone().unwrap_or_default(),
            "content": message.content
        }),
    }
}

fn chat_tool(definition: &ToolDefinition) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": definition.name,
            "description": definition.description,
            "parameters": definition.parameters_schema
        }
    })
}

fn chat_tool_call(tool_call: &AgentToolCall) -> Value {
    json!({
        "id": tool_call.id,
        "type": "function",
        "function": {
            "name": tool_call.name,
            "arguments": tool_call.raw_arguments
        }
    })
}

fn responses_input(messages: &[AgentMessage]) -> Vec<Value> {
    let mut input = Vec::new();

    for message in messages {
        match &message.role {
            AgentRole::System | AgentRole::User | AgentRole::Assistant => {
                let role = match &message.role {
                    AgentRole::System => "system",
                    AgentRole::User => "user",
                    AgentRole::Assistant => "assistant",
                    AgentRole::Tool => unreachable!(),
                };
                if !message.content.is_empty() {
                    input.push(json!({
                        "type": "message",
                        "role": role,
                        "content": [{"type": "input_text", "text": message.content}]
                    }));
                }
                for tool_call in &message.tool_calls {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": tool_call.id,
                        "name": tool_call.name,
                        "arguments": tool_call.raw_arguments
                    }));
                }
            }
            AgentRole::Tool => input.push(json!({
                "type": "function_call_output",
                "call_id": message.tool_call_id.clone().unwrap_or_default(),
                "output": message.content
            })),
        }
    }

    input
}

fn responses_tool(definition: &ToolDefinition) -> Value {
    json!({
        "type": "function",
        "name": definition.name,
        "description": definition.description,
        "parameters": definition.parameters_schema
    })
}

fn parse_chat_tool_call(value: &Value) -> MidgardResult<AgentToolCall> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| MidgardError::Agent("chat tool call missing id".to_string()))?;
    let function = value
        .get("function")
        .ok_or_else(|| MidgardError::Agent("chat tool call missing function".to_string()))?;
    let name = function
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| MidgardError::Agent("chat tool call missing function name".to_string()))?;
    let raw_arguments = function
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or("{}");

    Ok(AgentToolCall::from_raw(id, name, raw_arguments))
}

fn parse_response_tool_call(value: &Value) -> MidgardResult<AgentToolCall> {
    let id = value
        .get("call_id")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            MidgardError::Agent("responses function_call missing call_id".to_string())
        })?;
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| MidgardError::Agent("responses function_call missing name".to_string()))?;
    let raw_arguments = value
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or("{}");

    Ok(AgentToolCall::from_raw(id, name, raw_arguments))
}

fn push_response_message_content(content: &mut String, item: &Value) {
    let Some(parts) = item.get("content").and_then(Value::as_array) else {
        return;
    };

    for part in parts {
        if let Some(text) = part
            .get("text")
            .or_else(|| part.get("output_text"))
            .and_then(Value::as_str)
        {
            content.push_str(text);
        }
    }
}

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    raw_arguments: String,
}

#[derive(Default)]
struct StreamState {
    content: String,
    tool_calls: Vec<PartialToolCall>,
}

impl StreamState {
    fn apply_chat_value(&mut self, value: &Value) -> MidgardResult<Vec<LlmStreamEvent>> {
        let mut events = Vec::new();
        let Some(delta) = value.pointer("/choices/0/delta") else {
            return Ok(events);
        };

        if let Some(content) = delta.get("content").and_then(Value::as_str) {
            self.content.push_str(content);
            events.push(LlmStreamEvent::ContentDelta(content.to_string()));
        }

        if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for tool_call in tool_calls {
                let index = tool_call
                    .get("index")
                    .and_then(Value::as_u64)
                    .unwrap_or(self.tool_calls.len() as u64) as usize;
                self.ensure_tool_call(index);
                if let Some(id) = tool_call.get("id").and_then(Value::as_str) {
                    self.tool_calls[index].id = id.to_string();
                }
                if let Some(function) = tool_call.get("function") {
                    if let Some(name) = function.get("name").and_then(Value::as_str) {
                        self.tool_calls[index].name = name.to_string();
                    }
                    if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                        self.tool_calls[index].raw_arguments.push_str(arguments);
                    }
                }
            }
        }

        Ok(events)
    }

    fn apply_responses_value(&mut self, value: &Value) -> MidgardResult<Vec<LlmStreamEvent>> {
        let mut events = Vec::new();
        match value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "response.output_text.delta" => {
                if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                    self.content.push_str(delta);
                    events.push(LlmStreamEvent::ContentDelta(delta.to_string()));
                }
            }
            "response.output_item.added" => {
                if let Some(item) = value.get("item")
                    && item.get("type").and_then(Value::as_str) == Some("function_call")
                {
                    let index = self.tool_calls.len();
                    self.ensure_tool_call(index);
                    self.tool_calls[index].id = item
                        .get("call_id")
                        .or_else(|| item.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    self.tool_calls[index].name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                }
            }
            "response.function_call_arguments.delta" => {
                let item_id = value
                    .get("item_id")
                    .or_else(|| value.get("call_id"))
                    .and_then(Value::as_str);
                let delta = value
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let index = item_id
                    .and_then(|id| {
                        self.tool_calls
                            .iter()
                            .position(|tool_call| tool_call.id == id)
                    })
                    .unwrap_or_else(|| self.tool_calls.len().saturating_sub(1));
                self.ensure_tool_call(index);
                self.tool_calls[index].raw_arguments.push_str(delta);
            }
            "response.output_item.done" => {
                if let Some(item) = value.get("item")
                    && item.get("type").and_then(Value::as_str) == Some("function_call")
                {
                    let id = item
                        .get("call_id")
                        .or_else(|| item.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let index = self
                        .tool_calls
                        .iter()
                        .position(|tool_call| tool_call.id == id)
                        .unwrap_or_else(|| self.tool_calls.len().saturating_sub(1));
                    self.ensure_tool_call(index);
                    if self.tool_calls[index].id.is_empty() {
                        self.tool_calls[index].id = id.to_string();
                    }
                    if self.tool_calls[index].name.is_empty() {
                        self.tool_calls[index].name = item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                    }
                    if self.tool_calls[index].raw_arguments.is_empty() {
                        self.tool_calls[index].raw_arguments = item
                            .get("arguments")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                    }
                }
            }
            _ => {}
        }

        Ok(events)
    }

    fn finish(self) -> Vec<LlmStreamEvent> {
        let tool_calls = self
            .tool_calls
            .into_iter()
            .filter(|tool_call| !tool_call.name.is_empty())
            .map(|tool_call| {
                AgentToolCall::from_raw(tool_call.id, tool_call.name, tool_call.raw_arguments)
            })
            .collect::<Vec<_>>();
        let mut events = tool_calls
            .iter()
            .cloned()
            .map(LlmStreamEvent::ToolCallDone)
            .collect::<Vec<_>>();
        events.push(LlmStreamEvent::MessageDone(LlmResponse::with_tool_calls(
            self.content,
            tool_calls,
        )));
        events
    }

    fn ensure_tool_call(&mut self, index: usize) {
        while self.tool_calls.len() <= index {
            self.tool_calls.push(PartialToolCall::default());
        }
    }
}

fn sse_events(input: &str) -> Vec<String> {
    input
        .split("\n\n")
        .filter_map(|event| {
            let data = event
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(str::trim)
                .collect::<Vec<_>>()
                .join("\n");
            if data.is_empty() { None } else { Some(data) }
        })
        .collect()
}

fn pop_sse_event(buffer: &mut String) -> Option<String> {
    let split_at = buffer.find("\n\n")?;
    let event = buffer[..split_at].to_string();
    let rest = buffer[split_at + 2..].to_string();
    *buffer = rest;
    sse_events(&event).into_iter().next()
}

fn string_or_empty(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn agent_error(err: impl std::fmt::Display) -> MidgardError {
    MidgardError::Agent(err.to_string())
}
