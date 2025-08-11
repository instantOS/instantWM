use std::process::Command;
use std::env;

fn main() {
    // Generate shell completions
    let out_dir = env::var("OUT_DIR").unwrap();
    
    // TODO: Generate completions for bash, zsh, fish
    println!("cargo:rerun-if-changed=src/cli.rs");
}