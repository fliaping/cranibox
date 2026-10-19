//! Native passthrough and bounded streaming translation; pivot: Messages.
use bytes::{Bytes, BytesMut};
use cranibox_contracts::{
    llm::*,
    net::{BodyError, BoxBody},
};
use futures_util::StreamExt;
use serde_json::{Value, json};
mod chat;
#[path = "proto/port/magpie/mod.rs"]
mod magpie;
pub mod vendor;
use vendor::shunt::{
    compat,
    model::{
        inbound_responses::{chat_request, chat_stream, messages_request, messages_stream},
        responses::{self, AnthropicSseMachine, ResponseEvent},
        responses_request,
    },
};
pub fn invalid(message: &str) -> UpstreamError {
    UpstreamError::Status(400, message.into())
}
fn bad_stream() -> BodyError {
    Box::new(std::io::Error::other(
        "upstream stream is invalid or incomplete",
    ))
}
const LIMIT: usize = 32 * 1024 * 1024;

pub fn request(mut req: CanonicalRequest, target: Wire) -> Result<CanonicalRequest, UpstreamError> {
    if req.wire == target {
        return Ok(req);
    }
    let messages = match req.wire {
        Wire::Messages => req.body,
        Wire::Chat => chat::request(&req.body, &req.model)?,
        Wire::Responses => messages_request::translate_request(&req.body, &req.model)
            .map_err(|_| invalid("Responses input cannot be converted"))?,
    };
    let route = compat::Route {
        provider: String::new(),
        adapter: compat::AdapterKind::Responses,
        model: req.model.clone(),
        upstream_model: req.model.clone(),
        effort: None,
        service_tier: None,
    };
    req.body = match target {
        Wire::Messages => messages,
        Wire::Responses => responses_request::translate_request_value(
            &messages,
            &route,
            compat::ResponsesFlavor::Openai,
            false,
            req.session_id.as_deref(),
            false,
        ),
        Wire::Chat => {
            let intermediate = responses_request::translate_request_value(
                &messages,
                &route,
                compat::ResponsesFlavor::Openai,
                false,
                req.session_id.as_deref(),
                false,
            );
            chat_request::translate_request(&intermediate, &req.model)
                .map_err(|_| invalid("Messages input cannot be converted"))?
        }
    };
    req.body["stream"] = json!(req.stream);
    req.wire = target;
    Ok(req)
}
fn response_event(value: Value) -> ResponseEvent {
    ResponseEvent {
        event: value.get("type").and_then(Value::as_str).map(str::to_owned),
        data: value,
    }
}
fn to_messages(from: Wire, value: &Value, model: &str) -> Result<Value, UpstreamError> {
    if from == Wire::Messages {
        return Ok(value.clone());
    }
    let response = if from == Wire::Chat {
        chat_stream::translate_response(value, model)
    } else {
        value.clone()
    };
    let mut machine = AnthropicSseMachine::new(model, true, false);
    machine.apply(response_event(
        json!({"type":"response.created","response":response}),
    ));
    for (index, item) in response["output"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        machine.apply(response_event(
            json!({"type":"response.output_item.added","output_index":index,"item":item}),
        ));
        if item["type"] == "message" {
            for part in item["content"].as_array().into_iter().flatten() {
                if let Some(text) = part["text"].as_str() {
                    machine.apply(response_event(json!({"type":"response.output_text.delta","item_id":item["id"],"output_index":index,"delta":text})));
                }
            }
        }
        if item["type"] == "reasoning" {
            for part in item["summary"].as_array().into_iter().flatten() {
                if let Some(text) = part["text"].as_str() {
                    machine.apply(response_event(json!({"type":"response.reasoning_summary_text.delta","item_id":item["id"],"output_index":index,"delta":text})));
                }
            }
        }
        machine.apply(response_event(
            json!({"type":"response.output_item.done","output_index":index,"item":item}),
        ));
    }
    machine.apply(response_event(
        json!({"type":"response.completed","response":response}),
    ));
    if machine.take_backend_error().is_some() {
        return Err(UpstreamError::Status(502, "upstream failed".into()));
    }
    Ok(machine.final_json())
}
pub fn json(from: Wire, to: Wire, value: &Value, model: &str) -> Result<Value, UpstreamError> {
    if from == to {
        return Ok(value.clone());
    }
    let message = to_messages(from, value, model)?;
    Ok(match to {
        Wire::Messages => message,
        Wire::Chat => magpie::response(&message, model),
        Wire::Responses => messages_stream::translate_response(&message, model),
    })
}

/// Incremental UTF-8-safe SSE framing. A frame may span any transport chunk.
#[derive(Default)]
pub struct SseDecoder {
    pending: BytesMut,
}
impl SseDecoder {
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<ResponseEvent>, BodyError> {
        if self.pending.len() + chunk.len() > 4 * 1024 * 1024 {
            return Err(bad_stream());
        }
        self.pending.extend_from_slice(chunk);
        let mut frames = Vec::new();
        loop {
            let boundary = self
                .pending
                .windows(2)
                .position(|w| w == b"\n\n")
                .map(|n| (n, 2))
                .into_iter()
                .chain(
                    self.pending
                        .windows(4)
                        .position(|w| w == b"\r\n\r\n")
                        .map(|n| (n, 4)),
                )
                .min_by_key(|(n, _)| *n);
            let Some((n, sep)) = boundary else { break };
            let bytes = self.pending.split_to(n + sep);
            let frame = std::str::from_utf8(&bytes).map_err(|_| bad_stream())?;
            let mut event = None;
            let mut data = Vec::new();
            for line in frame.lines() {
                if let Some(v) = line.strip_prefix("event:") {
                    event = Some(v.trim().into())
                } else if let Some(v) = line.strip_prefix("data:") {
                    data.push(v.strip_prefix(' ').unwrap_or(v));
                }
            }
            if data.is_empty() {
                continue;
            }
            let data = data.join("\n");
            if data.trim() == "[DONE]" {
                frames.push(ResponseEvent {
                    event: Some("done".into()),
                    data: Value::Null,
                });
                continue;
            }
            let data: Value = serde_json::from_str(&data).map_err(|_| bad_stream())?;
            if event.is_none() {
                event = data.get("type").and_then(Value::as_str).map(str::to_owned)
            }
            frames.push(ResponseEvent { event, data });
        }
        Ok(frames)
    }
    pub fn finish(&self) -> Result<(), BodyError> {
        if self.pending.iter().all(u8::is_ascii_whitespace) {
            Ok(())
        } else {
            Err(bad_stream())
        }
    }
}
fn parsed_frames(frames: Vec<String>) -> Result<Vec<ResponseEvent>, BodyError> {
    let mut decoder = SseDecoder::default();
    decoder.push(frames.concat().as_bytes())
}

pub fn translate_stream(body: BoxBody, from: Wire, to: Wire, model: String) -> BoxBody {
    BoxBody::new(async_stream::try_stream! {
        let mut decoder=SseDecoder::default();let mut stream=body.into_stream();
        let mut chat_to_responses=chat_stream::ChatSseMachine::new(&model);
        let mut responses_to_messages=AnthropicSseMachine::new(&model,true,false).without_content_accumulation();
        let mut messages_to_responses=messages_stream::MessagesSseMachine::new(&model);
        let mut messages_to_chat=magpie::ChatEmitter::new(&model);
        let mut terminal=false;let mut total=0usize;
        while let Some(chunk)=stream.next().await{
            let chunk=chunk?;total=total.saturating_add(chunk.len());if total>LIMIT{Err(bad_stream())?;}
            for event in decoder.push(&chunk)?{
                let messages=match from{
                    Wire::Messages=>{if matches!(event.event.as_deref(),Some("message_stop"|"error")){terminal=true;}vec![event]},
                    Wire::Responses=>{if matches!(event.event.as_deref(),Some("response.completed"|"response.done"|"response.incomplete"|"response.failed"|"error")){terminal=true;}parsed_frames(responses_to_messages.apply(event))?},
                    Wire::Chat=>{
                        let data=if event.event.as_deref()==Some("done"){"[DONE]".into()}else{event.data.to_string()};
                        let responses=parsed_frames(chat_to_responses.apply(&data))?;terminal=chat_to_responses.is_terminal();
                        let mut messages=Vec::new();for response in responses{messages.extend(parsed_frames(responses_to_messages.apply(response))?);}messages
                    }
                };
                for message in messages{
                    let frames=match to{Wire::Messages=>vec![format!("event: {}\ndata: {}\n\n",message.event.as_deref().unwrap_or("message"),message.data)],Wire::Chat=>messages_to_chat.apply(&message.data),Wire::Responses=>messages_to_responses.apply(message)};
                    for frame in frames{yield Bytes::from(frame);}
                }
            }
        }
        decoder.finish()?;
        if !terminal{Err(bad_stream())?;}
    })
}
pub async fn response(
    mut upstream: UpstreamResponse,
    to: Wire,
    stream: bool,
    model: &str,
) -> Result<UpstreamResponse, UpstreamError> {
    let sse = upstream
        .headers
        .iter()
        .any(|(n, v)| n.eq_ignore_ascii_case("content-type") && v.contains("text/event-stream"));
    if upstream.wire == to && (stream || !sse) {
        return Ok(upstream);
    }
    if stream {
        if !sse {
            return Err(UpstreamError::Status(
                502,
                "upstream did not return an event stream".into(),
            ));
        }
        upstream.body = translate_stream(upstream.body, upstream.wire, to, model.into());
    } else {
        let mut bytes = Vec::new();
        let mut body = upstream.body.into_stream();
        while let Some(chunk) = body.next().await {
            let chunk =
                chunk.map_err(|_| UpstreamError::Network("upstream body interrupted".into()))?;
            if bytes.len() + chunk.len() > LIMIT {
                return Err(UpstreamError::Status(
                    502,
                    "upstream response exceeds limit".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value = if sse {
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| UpstreamError::Status(502, "invalid SSE encoding".into()))?;
            let mut machine = AnthropicSseMachine::new(model, true, false);
            let mut terminal = false;
            for event in responses::parse_sse_events(text) {
                if matches!(
                    event.event.as_deref(),
                    Some("response.completed" | "response.done" | "response.incomplete")
                ) {
                    terminal = true;
                }
                machine.apply(event);
            }
            if upstream.wire != Wire::Responses
                || !terminal
                || machine.take_backend_error().is_some()
            {
                return Err(UpstreamError::Network("upstream stream incomplete".into()));
            }
            let message = machine.final_json();
            json(Wire::Messages, to, &message, model)?
        } else {
            let value = serde_json::from_slice(&bytes)
                .map_err(|_| UpstreamError::Status(502, "invalid upstream JSON".into()))?;
            json(upstream.wire, to, &value, model)?
        };
        upstream.body = BoxBody::full(serde_json::to_vec(&value).expect("JSON"));
    }
    upstream.wire = to;
    upstream
        .headers
        .retain(|(n, _)| !n.eq_ignore_ascii_case("content-type"));
    upstream.headers.push((
        "content-type".into(),
        if stream {
            "text/event-stream"
        } else {
            "application/json"
        }
        .into(),
    ));
    Ok(upstream)
}
