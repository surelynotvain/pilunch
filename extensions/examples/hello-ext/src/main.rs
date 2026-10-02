//! Example PiLunch extension: two tools, one read-only. Install it from PiLunch with
//! Customize → Extensions → Build Rust extension, and pick this folder.

use pilunch_extension::{json, Extension, Output};

fn main() {
    Extension::new("hello-ext", env!("CARGO_PKG_VERSION"))
        .instructions("Example tools: greet people and count words.")
        .read_only_tool(
            "word_count",
            "Count the words, lines and characters in a piece of text.",
            json!({ "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] }),
            |args| {
                let text = args["text"].as_str().ok_or("text is required")?;
                Ok(Output::text(format!("{} words, {} lines, {} characters", text.split_whitespace().count(), text.lines().count(), text.chars().count())))
            },
        )
        .tool(
            "greet",
            "Write a greeting for someone, optionally shouting.",
            json!({
                "type": "object",
                "properties": { "name": { "type": "string" }, "shout": { "type": "boolean" } },
                "required": ["name"]
            }),
            |args| {
                let name = args["name"].as_str().unwrap_or("world");
                let msg = format!("Hello, {name}! Greetings from a Rust extension.");
                Ok(Output::text(if args["shout"].as_bool().unwrap_or(false) { msg.to_uppercase() } else { msg }))
            },
        )
        .run();
}
