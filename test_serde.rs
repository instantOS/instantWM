use serde_json::Value;

fn main() {
    let arr_str = r#"   [   {"full_text": "hello"} ]"#;
    let res2: Result<Vec<Value>, _> = serde_json::from_str(arr_str);
    println!("{:?}", res2);
}
