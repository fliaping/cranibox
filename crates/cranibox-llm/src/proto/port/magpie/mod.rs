//! Messages -> Chat response mappings ported from yetone/magpie
//! internal/gateway/{anthropic,chat,chat_call_ids,chat_tool_shape}.go
//! @865b61808690bc9fd716257949414ec97618a5a5 (MIT).
use serde_json::{Value, json};
use std::collections::HashMap;
fn reason(reason: &str) -> &str {
    match reason {
        "tool_use" => "tool_calls",
        "max_tokens" => "length",
        _ => "stop",
    }
}
pub fn response(message: &Value, model: &str) -> Value {
    let mut text = String::new();
    let mut thinking = String::new();
    let mut calls = Vec::new();
    for block in message["content"].as_array().into_iter().flatten() {
        match block["type"].as_str(){
        Some("text")=>text.push_str(block["text"].as_str().unwrap_or("")),
        Some("thinking")=>thinking.push_str(block["thinking"].as_str().unwrap_or("")),
        Some("tool_use")=>calls.push(json!({"id":block["id"],"type":"function","function":{"name":block["name"],"arguments":block["input"].to_string()}})),_=>{}
    }
    }
    let mut msg = json!({"role":"assistant","content":text});
    if !thinking.is_empty() {
        msg["reasoning_content"] = thinking.into();
    }
    if !calls.is_empty() {
        msg["tool_calls"] = calls.into();
    }
    let input = message["usage"]["input_tokens"].as_u64().unwrap_or(0);
    let output = message["usage"]["output_tokens"].as_u64().unwrap_or(0);
    json!({"id":message["id"],"object":"chat.completion","created":crate::auth::now(),"model":model,"choices":[{"index":0,"message":msg,"finish_reason":reason(message["stop_reason"].as_str().unwrap_or("end_turn"))}],"usage":{"prompt_tokens":input,"completion_tokens":output,"total_tokens":input+output}})
}
pub struct ChatEmitter {
    model: String,
    id: String,
    created: u64,
    tools: HashMap<u64, u64>,
    next_tool: u64,
    input: u64,
    output: u64,
    terminal: bool,
}
impl ChatEmitter {
    pub fn new(model: &str) -> Self {
        Self {
            model: model.into(),
            id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
            created: crate::auth::now(),
            tools: HashMap::new(),
            next_tool: 0,
            input: 0,
            output: 0,
            terminal: false,
        }
    }
    fn chunk(&self, delta: Value, finish: Value) -> String {
        format!(
            "data: {}\n\n",
            json!({"id":self.id,"object":"chat.completion.chunk","created":self.created,"model":self.model,"choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
        )
    }
    pub fn apply(&mut self, event: &Value) -> Vec<String> {
        if self.terminal {
            return vec![];
        }
        let index = event["index"].as_u64().unwrap_or(0);
        match event["type"].as_str() {
            Some("message_start") => {
                self.input = event["message"]["usage"]["input_tokens"]
                    .as_u64()
                    .unwrap_or(0);
                vec![self.chunk(json!({"role":"assistant","content":""}), Value::Null)]
            }
            Some("content_block_start") => {
                let block = &event["content_block"];
                match block["type"].as_str() {
                    Some("tool_use") => {
                        let tool = self.next_tool;
                        self.next_tool += 1;
                        self.tools.insert(index, tool);
                        let args = block
                            .get("input")
                            .filter(|v| v.as_object().is_some_and(|m| !m.is_empty()))
                            .map(Value::to_string)
                            .unwrap_or_default();
                        vec![self.chunk(json!({"tool_calls":[{"index":tool,"id":block["id"],"type":"function","function":{"name":block["name"],"arguments":args}}]}),Value::Null)]
                    }
                    Some("text") if block["text"].as_str().is_some_and(|s| !s.is_empty()) => {
                        vec![self.chunk(json!({"content":block["text"]}), Value::Null)]
                    }
                    Some("thinking")
                        if block["thinking"].as_str().is_some_and(|s| !s.is_empty()) =>
                    {
                        vec![
                            self.chunk(json!({"reasoning_content":block["thinking"]}), Value::Null),
                        ]
                    }
                    _ => vec![],
                }
            }
            Some("content_block_delta") => {
                let delta = &event["delta"];
                let d = match delta["type"].as_str() {
                    Some("text_delta") => json!({"content":delta["text"]}),
                    Some("thinking_delta") => json!({"reasoning_content":delta["thinking"]}),
                    Some("input_json_delta") => match self.tools.get(&index) {
                        Some(tool) => {
                            json!({"tool_calls":[{"index":tool,"function":{"arguments":delta["partial_json"]}}]})
                        }
                        None => return vec![],
                    },
                    _ => return vec![],
                };
                vec![self.chunk(d, Value::Null)]
            }
            Some("message_delta") => {
                self.output = event["usage"]["output_tokens"]
                    .as_u64()
                    .unwrap_or(self.output);
                vec![self.chunk(
                    json!({}),
                    json!(reason(
                        event["delta"]["stop_reason"].as_str().unwrap_or("end_turn")
                    )),
                )]
            }
            Some("message_stop") => {
                self.terminal = true;
                let usage = json!({"prompt_tokens":self.input,"completion_tokens":self.output,"total_tokens":self.input+self.output});
                vec![
                    format!(
                        "data: {}\n\n",
                        json!({"id":self.id,"object":"chat.completion.chunk","created":self.created,"model":self.model,"choices":[],"usage":usage})
                    ),
                    "data: [DONE]\n\n".into(),
                ]
            }
            Some("error") => {
                self.terminal = true;
                vec![format!(
                    "data: {}\n\n",
                    json!({"error":{"type":"upstream_error","message":"upstream stream failed"}})
                )]
            }
            _ => vec![],
        }
    }
}
