use async_trait::async_trait;
use bytes::Bytes;
use futures_core::stream::Stream;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::io::{AsyncBufReadExt, BufReader};

use ai_partner_shared::{AgentEvent, Message, ModelProvider, Role, ToolCall, ToolDefinition};

use crate::adapter::{AgentResponse, LlmAdapter};
use crate::agent::AgentError;

pub struct OpenAIAdapter {
    client: Client,
}

impl OpenAIAdapter {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

impl Default for OpenAIAdapter {
    fn default() -> Self {
        Self::new()
    }
}

// ── Request types ──

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ChatTool>>,
    stream: bool,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ChatToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize)]
struct ChatTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: ChatFunction,
}

#[derive(Serialize)]
struct ChatFunction {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Serialize, Deserialize, Clone)]
struct ChatToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: ChatFunctionCall,
}

#[derive(Serialize, Deserialize, Clone)]
struct ChatFunctionCall {
    name: String,
    arguments: String,
}

// ── Streaming response types ──

#[derive(Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
    #[allow(dead_code)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct StreamDelta {
    content: Option<String>,
    tool_calls: Option<Vec<StreamToolCall>>,
}

#[derive(Deserialize)]
struct StreamToolCall {
    index: usize,
    id: Option<String>,
    function: Option<StreamFunctionCall>,
}

#[derive(Deserialize)]
struct StreamFunctionCall {
    name: Option<String>,
    arguments: Option<String>,
}

fn build_messages(messages: &[Message]) -> Vec<ChatMessage> {
    messages
        .iter()
        .map(|m| {
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
                Role::Tool => "tool",
            };
            ChatMessage {
                role: role.to_string(),
                content: if m.content.is_empty() {
                    None
                } else {
                    Some(m.content.clone())
                },
                tool_calls: m.tool_calls.as_ref().map(|tcs| {
                    tcs.iter()
                        .map(|tc| ChatToolCall {
                            id: tc.id.clone(),
                            call_type: "function".into(),
                            function: ChatFunctionCall {
                                name: tc.name.clone(),
                                arguments: tc.arguments.to_string(),
                            },
                        })
                        .collect()
                }),
                tool_call_id: m.tool_call_id.clone(),
            }
        })
        .collect()
}

fn build_tools(tools: &[ToolDefinition]) -> Vec<ChatTool> {
    tools
        .iter()
        .map(|t| ChatTool {
            tool_type: "function".into(),
            function: ChatFunction {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            },
        })
        .collect()
}

/// 累积流式工具调用的中间状态
struct ToolCallAccumulator {
    id: String,
    name: String,
    arguments: String,
}

#[async_trait]
impl LlmAdapter for OpenAIAdapter {
    async fn chat(
        &self,
        provider: &ModelProvider,
        messages: &[Message],
        tools: &[ToolDefinition],
        event_tx: &mpsc::UnboundedSender<AgentEvent>,
    ) -> Result<AgentResponse, AgentError> {
        let url = format!("{}/chat/completions", provider.base_url.trim_end_matches('/'));

        let chat_messages = build_messages(messages);
        let chat_tools = if tools.is_empty() {
            None
        } else {
            Some(build_tools(tools))
        };

        let request = ChatRequest {
            model: provider.model.clone(),
            messages: chat_messages,
            max_tokens: Some(provider.max_tokens),
            tools: chat_tools,
            stream: true,
        };

        let _ = event_tx.send(AgentEvent::Thinking);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", provider.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AgentError::Other(format!(
                "OpenAI API error {status}: {body}"
            )));
        }

        // 流式读取 SSE
        let stream = response.bytes_stream();
        let reader = StreamReader::new(stream);
        let buf_reader = BufReader::new(reader);

        let mut full_content = String::new();
        let mut tool_accumulators: Vec<ToolCallAccumulator> = Vec::new();
        let mut has_tool_calls = false;

        let mut lines = buf_reader.lines();
        while let Some(line) = lines.next_line().await? {
            let line = line.trim();
            if line.is_empty() || line.starts_with(':') {
                continue;
            }
            if line == "data: [DONE]" {
                break;
            }

            let data = match line.strip_prefix("data: ") {
                Some(d) => d,
                None => continue,
            };

            let chunk: StreamChunk = match serde_json::from_str(data) {
                Ok(c) => c,
                Err(_) => continue,
            };

            for choice in chunk.choices {
                // 内容 delta
                if let Some(content) = choice.delta.content {
                    full_content.push_str(&content);
                    let _ = event_tx.send(AgentEvent::Delta(content));
                }

                // 工具调用 delta
                if let Some(tc_deltas) = choice.delta.tool_calls {
                    has_tool_calls = true;
                    for delta in tc_deltas {
                        // 扩展累积器
                        while tool_accumulators.len() <= delta.index {
                            tool_accumulators.push(ToolCallAccumulator {
                                id: String::new(),
                                name: String::new(),
                                arguments: String::new(),
                            });
                        }
                        let acc = &mut tool_accumulators[delta.index];
                        if let Some(id) = delta.id {
                            acc.id = id;
                        }
                        if let Some(func) = delta.function {
                            if let Some(name) = func.name {
                                acc.name = name;
                            }
                            if let Some(args) = func.arguments {
                                acc.arguments.push_str(&args);
                            }
                        }
                    }
                }
            }
        }

        // 构建最终响应
        if has_tool_calls && !tool_accumulators.is_empty() {
            let calls: Vec<ToolCall> = tool_accumulators
                .into_iter()
                .map(|acc| {
                    let args: Value = serde_json::from_str(&acc.arguments)
                        .unwrap_or(Value::Object(serde_json::Map::new()));
                    ToolCall {
                        id: acc.id,
                        name: acc.name,
                        arguments: args,
                    }
                })
                .collect();

            for call in &calls {
                let _ = event_tx.send(AgentEvent::ToolCallStart(call.clone()));
            }

            Ok(AgentResponse::ToolCalls(calls))
        } else {
            let assistant_msg = Message::assistant(&full_content);
            let _ = event_tx.send(AgentEvent::MessageComplete(assistant_msg.clone()));
            let _ = event_tx.send(AgentEvent::Done);
            Ok(AgentResponse::MessageComplete(assistant_msg))
        }
    }
}

/// 将 reqwest 的 bytes_stream 包装为 AsyncRead
struct StreamReader {
    inner: std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
    buffer: Vec<u8>,
    pos: usize,
}

impl StreamReader {
    fn new(stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static) -> Self {
        Self {
            inner: Box::pin(stream),
            buffer: Vec::new(),
            pos: 0,
        }
    }
}

impl tokio::io::AsyncRead for StreamReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        // 如果缓冲区还有数据，直接返回
        if self.pos < self.buffer.len() {
            let remaining = &self.buffer[self.pos..];
            let to_copy = remaining.len().min(buf.remaining());
            buf.put_slice(&remaining[..to_copy]);
            self.pos += to_copy;
            if self.pos >= self.buffer.len() {
                self.buffer.clear();
                self.pos = 0;
            }
            return std::task::Poll::Ready(Ok(()));
        }

        // 从 stream 读取下一块
        match self.inner.as_mut().poll_next(cx) {
            std::task::Poll::Ready(Some(Ok(bytes))) => {
                self.buffer = bytes.to_vec();
                self.pos = 0;
                let to_copy = self.buffer.len().min(buf.remaining());
                buf.put_slice(&self.buffer[..to_copy]);
                self.pos = to_copy;
                if self.pos >= self.buffer.len() {
                    self.buffer.clear();
                    self.pos = 0;
                }
                std::task::Poll::Ready(Ok(()))
            }
            std::task::Poll::Ready(Some(Err(e))) => {
                std::task::Poll::Ready(Err(std::io::Error::other(e)))
            }
            std::task::Poll::Ready(None) => std::task::Poll::Ready(Ok(())),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}
