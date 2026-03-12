use std::fs;

fn main() {
    let raw = r#"{"name":"time","instance":"time","full_text":" 12:34 "}"#;
    let v: Result<serde_json::Value, serde_json::Error> = serde_json::from_str(raw);
    println!("{:?}", v);

    let raw_list = r#"[{"name":"time","instance":"time","full_text":" 12:34 "}]"#;
    let v_list: Result<serde_json::Value, serde_json::Error> = serde_json::from_str(raw_list);
    println!("{:?}", v_list);
}
