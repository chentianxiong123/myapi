//! usage_proto（按主题拆分）
#![allow(unused_imports)]
use super::*;
use super::helpers::*;
use std::sync::Arc;
use serde_json::json;

#[test]
fn proto_openai_usage_nonstream() {
    // OpenAI 非流式：usage.prompt_tokens / completion_tokens
    let v = json!({"usage": {"prompt_tokens": 10, "completion_tokens": 5,
        "prompt_tokens_details": {"cached_tokens": 3}}});
    let (i, o, cr, cc) = parse_usage_json(&v);
    assert_eq!((i, o, cr, cc), (Some(10), Some(5), Some(3), None), "OpenAI 非流式");
}

#[test]
fn proto_anthropic_usage_nonstream() {
    // Anthropic 非流式：usage.input_tokens / output_tokens + cache 两字段
    let v = json!({"usage": {"input_tokens": 8, "output_tokens": 4,
        "cache_read_input_tokens": 6, "cache_creation_input_tokens": 2}});
    let (i, o, cr, cc) = parse_usage_json(&v);
    assert_eq!((i, o, cr, cc), (Some(8), Some(4), Some(6), Some(2)), "Anthropic 非流式");
}

#[test]
fn proto_gemini_usage_nonstream() {
    // Gemini 非流式：顶层 usageMetadata.promptTokenCount / candidatesTokenCount
    let v = json!({"usageMetadata": {"promptTokenCount": 12, "candidatesTokenCount": 7,
        "cachedContentTokenCount": 4}});
    let (i, o, cr, _cc) = parse_usage_json(&v);
    assert_eq!((i, o, cr), (Some(12), Some(7), Some(4)), "Gemini 非流式");
}

#[test]
fn proto_openai_usage_stream() {
    // OpenAI 流式：最后 chunk 带 usage（同一 chunk 含 input+output）
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n\
                data: {\"choices\":[],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":3}}\n\n\
                data: [DONE]\n\n";
    let u = sse_scan_usage(body.as_bytes()).unwrap();
    assert_eq!(u, (Some(9), Some(3), None, None), "OpenAI 流式");
}

#[test]
fn proto_anthropic_usage_stream_split() {
    // Anthropic 流式：usage 分片——message_start 带 input，message_delta 带 output。
    // 必须"跨 chunk 累积合并"，不能取最后一个（否则丢 input）。
    let body = "data: {\"type\":\"message_start\",\"usage\":{\"input_tokens\":11}}\n\n\
                data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hi\"}}\n\n\
                data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":5}}\n\n";
    let u = sse_scan_usage(body.as_bytes()).unwrap();
    assert_eq!(u, (Some(11), Some(5), None, None), "Anthropic 分片合并 input+output");
}

#[test]
fn proto_gemini_usage_stream() {
    // Gemini 流式：chunk 顶层 usageMetadata（promptTokenCount/candidatesTokenCount）
    let body = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hi\"}]}}]}\n\n\
                data: {\"usageMetadata\":{\"promptTokenCount\":15,\"candidatesTokenCount\":6}}\n\n";
    let u = sse_scan_usage(body.as_bytes()).unwrap();
    assert_eq!(u, (Some(15), Some(6), None, None), "Gemini 流式");
}

#[test]
fn proto_stream_no_usage_returns_none() {
    // 无 usage 的流（纯内容）→ None，不误报
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]\n\n";
    assert_eq!(sse_scan_usage(body.as_bytes()), None);
}