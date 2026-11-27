//! 单元测试（从内联 mod tests 整段拆出）
#![allow(unused_imports)]
use super::*;
use tempfile::TempDir;

#[test]
fn yaml_to_json_roundtrip_scalar_and_array() {
    let y: serde_yaml::Value = serde_yaml::from_str("name: p1\nlist: [1, 2, 3]").unwrap();
    let j = yaml_to_json(&y);
    assert_eq!(j.get("name").and_then(|v| v.as_str()), Some("p1"));
    assert_eq!(j.get("list").and_then(|v| v.as_array()).map(|a| a.len()), Some(3));
}

#[test]
fn json_to_yaml_roundtrip() {
    let j = json!({"key": "value", "n": 42});
    let y = json_to_yaml(&j);
    let back = yaml_to_json(&y);
    assert_eq!(back.get("key").and_then(|v| v.as_str()), Some("value"));
    assert_eq!(back.get("n").and_then(|v| v.as_i64()), Some(42));
}

#[test]
fn write_then_read_yaml_json_preserves_data() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.yaml");
    let data = json!({
        "listen": 8380,
        "providers": [{"name": "p1", "base_url": "http://x/v1"}],
    });
    write_yaml_json(&path, &data).unwrap();
    let read = read_yaml_json(&path);
    assert_eq!(read.get("listen").and_then(|v| v.as_i64()), Some(8380));
    assert_eq!(read.get("providers").and_then(|v| v.as_array()).map(|a| a.len()), Some(1));
    assert_eq!(read.get("providers").and_then(|v| v.get(0)).and_then(|v| v.get("name")).and_then(|v| v.as_str()), Some("p1"));
}

#[test]
fn read_missing_yaml_returns_null() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nonexistent.yaml");
    assert!(read_yaml_json(&path).is_null());
}

#[test]
fn write_gateway_section_preserves_other_sections() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("gw.yaml");
    // 初始：listen + providers
    write_yaml_json(&path, &json!({"listen": 8380, "providers": [{"name": "p1"}]})).unwrap();
    // 用 section 写入逻辑：加 routing 段，保留 listen/providers
    let mut doc = read_yaml_json(&path);
    if let Some(obj) = doc.as_object_mut() {
        obj.insert("routing".into(), json!({"strategy": "latency"}));
    }
    write_yaml_json(&path, &doc).unwrap();
    let read = read_yaml_json(&path);
    assert_eq!(read.get("listen").and_then(|v| v.as_i64()), Some(8380)); // 保留
    assert!(read.get("providers").is_some()); // 保留
    assert_eq!(read.get("routing").and_then(|v| v.get("strategy")).and_then(|v| v.as_str()), Some("latency")); // 新增
}