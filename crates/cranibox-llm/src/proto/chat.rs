//! Chat -> Messages. Tool/content mappings adapted from link-assistant/router
//! src/{openai,openai_tools}.rs@07901ad24 (Unlicense). Unsupported content fails explicitly.
use super::invalid;
use cranibox_contracts::llm::UpstreamError;
use serde_json::{Value, json};

pub fn request(body: &Value, model: &str) -> Result<Value, UpstreamError> {
    if body
        .get("n")
        .and_then(Value::as_u64)
        .is_some_and(|n| n != 1)
    {
        return Err(invalid("conversion requires n=1"));
    }
    let input = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("messages must be an array"))?;
    let mut messages: Vec<Value> = Vec::new();
    let mut system = Vec::new();
    for message in input {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("message role required"))?;
        let content = parts(message.get("content").unwrap_or(&Value::Null))?;
        if matches!(role, "system" | "developer") {
            system.extend(content);
            continue;
        }
        let mut blocks = content;
        if role == "tool" {
            let id = message
                .get("tool_call_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| invalid("tool_call_id required"))?;
            blocks = vec![json!({"type":"tool_result","tool_use_id":id,"content":blocks})];
        }
        if let Some(reasoning) = message
            .get("reasoning_content")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            blocks.insert(0, json!({"type":"thinking","thinking":reasoning}));
        }
        if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let input = call
                    .pointer("/function/arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}");
                let input: Value = serde_json::from_str(input)
                    .map_err(|_| invalid("tool arguments must be JSON"))?;
                if !input.is_object() {
                    return Err(invalid("tool arguments must be an object"));
                }
                blocks.push(json!({"type":"tool_use","id":call["id"],"name":call["function"]["name"],"input":input}));
            }
        }
        let role = match role {
            "assistant" => "assistant",
            "user" | "tool" => "user",
            _ => return Err(invalid("unsupported message role")),
        };
        if let Some(last) = messages.last_mut().filter(|m| m["role"] == role) {
            last["content"]
                .as_array_mut()
                .expect("array")
                .extend(blocks)
        } else {
            messages.push(json!({"role":role,"content":blocks}));
        }
    }
    if messages.is_empty() {
        return Err(invalid("conversation is empty"));
    }
    let mut out = json!({"model":model,"messages":messages,"max_tokens":body.get("max_completion_tokens").or_else(||body.get("max_tokens")).cloned().unwrap_or(json!(8192)),"stream":body["stream"]==true});
    if !system.is_empty() {
        out["system"] = system.into();
    }
    for key in ["temperature", "top_p"] {
        if let Some(v) = body.get(key) {
            out[key] = v.clone();
        }
    }
    if let Some(stop) = body.get("stop") {
        out["stop_sequences"] = if stop.is_string() {
            json!([stop])
        } else {
            stop.clone()
        };
    }
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        let mut translated = Vec::new();
        for tool in tools {
            if tool["type"] != "function" {
                return Err(invalid("unsupported tool type for conversion"));
            }
            let f = &tool["function"];
            let mut t = json!({"name":f["name"],"input_schema":f.get("parameters").cloned().unwrap_or(json!({"type":"object","properties":{}}))});
            if let Some(d) = f.get("description") {
                t["description"] = d.clone();
            }
            translated.push(t);
        }
        if !translated.is_empty() {
            out["tools"] = translated.into();
        }
    }
    if let Some(choice) = body.get("tool_choice") {
        out["tool_choice"] = match choice.as_str() {
            Some("auto") => json!({"type":"auto"}),
            Some("required") => json!({"type":"any"}),
            Some("none") => json!({"type":"none"}),
            _ if choice.pointer("/function/name").is_some() => {
                json!({"type":"tool","name":choice["function"]["name"]})
            }
            _ => return Err(invalid("unsupported tool_choice")),
        };
    }
    if let Some(parallel) = body.get("parallel_tool_calls").and_then(Value::as_bool) {
        if out.get("tool_choice").is_none() {
            out["tool_choice"] = json!({"type":"auto"});
        }
        out["tool_choice"]["disable_parallel_tool_use"] = json!(!parallel);
    }
    if let Some(effort) = body
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .filter(|e| *e != "none")
    {
        let cap = out["max_tokens"].as_u64().unwrap_or(8192);
        let budget = match effort {
            "low" => 2048,
            "medium" => 8192,
            _ => 16384,
        }
        .min(cap.saturating_sub(1));
        if budget >= 1024 {
            out["thinking"] = json!({"type":"enabled","budget_tokens":budget});
            out.as_object_mut().expect("object").remove("temperature");
            out.as_object_mut().expect("object").remove("top_p");
        }
    }
    if let Some(format) = body.get("response_format") {
        match format["type"].as_str() {
            Some("text") => {}
            Some("json_schema") => {
                out["output_format"] =
                    json!({"type":"json_schema","schema":format["json_schema"]["schema"]});
            }
            _ => return Err(invalid("unsupported response format")),
        }
    }
    Ok(out)
}
fn parts(content: &Value) -> Result<Vec<Value>, UpstreamError> {
    match content{
        Value::Null=>Ok(vec![]),Value::String(s)=>Ok(vec![json!({"type":"text","text":s})]),
        Value::Array(parts)=>parts.iter().map(|part|match part["type"].as_str(){
            Some("text")=>Ok(json!({"type":"text","text":part["text"]})),
            Some("image_url")=>{
                let url=part.pointer("/image_url/url").and_then(Value::as_str).ok_or_else(||invalid("image URL required"))?;
                if let Some(data)=url.strip_prefix("data:"){let (media,data)=data.split_once(";base64,").ok_or_else(||invalid("image data URL must use base64"))?;Ok(json!({"type":"image","source":{"type":"base64","media_type":media,"data":data}}))}
                else{Ok(json!({"type":"image","source":{"type":"url","url":url}}))}
            },_=>Err(invalid("unsupported content block for conversion")),
        }).collect(),_=>Err(invalid("unsupported message content")),
    }
}
