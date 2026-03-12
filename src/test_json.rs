use serde_json::Value;

#[test]
pub fn test() {
    let json_str = r#"[{"full_text": "hello"}],"#;
    let single: Result<Vec<Value>, _> = serde_json::from_str(json_str);
    println!("{:?}", single);
}
