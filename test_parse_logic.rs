use serde_json::Value;

#[derive(serde::Deserialize, Debug)]
struct RawI3Block {
    #[serde(default)]
    full_text: String,
    #[serde(default)]
    short_text: Option<String>,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    background: Option<String>,
    #[serde(default)]
    border: Option<String>,
    #[serde(default)]
    border_top: Option<i32>,
    #[serde(default)]
    border_right: Option<i32>,
    #[serde(default)]
    border_bottom: Option<i32>,
    #[serde(default)]
    border_left: Option<i32>,
    #[serde(default)]
    min_width: Option<Value>,
    #[serde(default)]
    align: Option<String>,
    #[serde(default)]
    urgent: bool,
    #[serde(default)]
    separator: bool,
    #[serde(default)]
    separator_block_width: Option<i32>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    instance: Option<String>,
    #[serde(default)]
    markup: Option<String>,
}

fn main() {
    let json_str = r#"   [   {"full_text": "hello"} ]"#;
    let single: Result<Vec<RawI3Block>, _> = serde_json::from_str(json_str);
    println!("{:?}", single);
}
