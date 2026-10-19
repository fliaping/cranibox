use cranibox_contracts::{llm::*, net::BoxBody};
use cranibox_llm::proto;
use futures_util::StreamExt;
use serde_json::Value;
const WIRES: [Wire; 3] = [Wire::Chat, Wire::Responses, Wire::Messages];
fn fixture(wire: Wire, kind: &str) -> Vec<u8> {
    let name = match wire {
        Wire::Chat => "chat",
        Wire::Responses => "responses",
        Wire::Messages => "messages",
    };
    std::fs::read(format!(
        "{}/tests/replay/{name}-{kind}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn request_matrix_preserves_parallel_calls_and_results() {
    for from in WIRES {
        for to in WIRES {
            let original: Value = serde_json::from_slice(&fixture(from, "request.json")).unwrap();
            let request = CanonicalRequest {
                wire: from,
                body: original.clone(),
                stream: false,
                session_id: Some("test-session".into()),
                client_agent: None,
                model: "test-model".into(),
            };
            let converted = proto::request(request, to).unwrap();
            let text = converted.body.to_string();
            assert!(
                text.contains("weather")
                    && text.contains("clock")
                    && text.contains("晴天")
                    && text.contains("12:00"),
                "{from:?} -> {to:?}: {text}"
            );
            assert!(
                text.contains("call_weather") && text.contains("call_clock"),
                "{from:?} -> {to:?}: {text}"
            );
            if from == to {
                assert_eq!(converted.body, original);
            }
        }
    }
}
#[test]
fn response_matrix_preserves_text_parallel_tools_and_reasoning() {
    for from in WIRES {
        for to in WIRES {
            let original: Value = serde_json::from_slice(&fixture(from, "response.json")).unwrap();
            let value = proto::json(from, to, &original, "test-model").unwrap();
            let text = value.to_string();
            for part in ["完成 🦀", "weather", "clock", "北京", "UTC", "先分析"] {
                assert!(
                    text.contains(part),
                    "{from:?} -> {to:?}, missing {part}: {text}"
                );
            }
            if from == to {
                assert_eq!(value, original);
            }
        }
    }
}
#[tokio::test]
async fn streaming_matrix_survives_single_byte_utf8_chunks() {
    for from in WIRES {
        for to in WIRES {
            let data = fixture(from, "stream.sse");
            let chunks: Vec<_> = data
                .into_iter()
                .map(|byte| Ok(bytes::Bytes::from(vec![byte])))
                .collect();
            let upstream = UpstreamResponse {
                status: 200,
                headers: vec![("content-type".into(), "text/event-stream".into())],
                wire: from,
                body: BoxBody::new(futures_util::stream::iter(chunks)),
            };
            let response = proto::response(upstream, to, true, "test-model")
                .await
                .unwrap();
            let mut stream = response.body.into_stream();
            let mut output = Vec::new();
            while let Some(chunk) = stream.next().await {
                output.extend_from_slice(
                    &chunk.unwrap_or_else(|e| panic!("{from:?} -> {to:?}: {e}")),
                );
            }
            let text = String::from_utf8(output).unwrap();
            for part in ["完成 🦀", "weather", "clock", "北京", "UTC", "先分析"] {
                assert!(
                    text.contains(part),
                    "{from:?} -> {to:?}, missing {part}: {text}"
                );
            }
            match to {
                Wire::Chat => assert!(text.contains("[DONE]")),
                Wire::Responses => assert!(text.contains("response.completed")),
                Wire::Messages => assert!(text.contains("message_stop")),
            }
        }
    }
}
#[test]
fn sse_decoder_handles_mixed_line_endings_and_incomplete_frames() {
    let mut decoder = proto::SseDecoder::default();
    let frames = decoder
        .push(b"event: first\r\ndata: {\"a\":1}\r\n\r\nevent: second\ndata: {\"b\":2}\n\n")
        .unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].data["a"], 1);
    assert_eq!(frames[1].data["b"], 2);
    decoder.finish().unwrap();
    decoder.push(b"data: {").unwrap();
    assert!(decoder.finish().is_err());
}
#[tokio::test]
async fn truncated_converted_stream_is_an_error() {
    let response = BoxBody::full(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"x\",\"usage\":{}}}\n\n",
    );
    let output = proto::translate_stream(response, Wire::Messages, Wire::Chat, "model".into())
        .into_stream()
        .collect::<Vec<_>>()
        .await;
    assert!(output.iter().any(Result::is_err));
}
