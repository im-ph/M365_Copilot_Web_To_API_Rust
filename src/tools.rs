use std::path::Path;
use std::process::Command;
use regex::Regex;

lazy_static::lazy_static! {
    static ref COMPILED: Vec<(&'static str, Vec<Regex>)> = {
        vec![
            ("Read", vec![
                Regex::new(r#"^contents?\s+of\s+["']?(\S+(?:\.[a-zA-Z0-9]+)(?:\S*))["']?\s*$"#).unwrap(),
                Regex::new(r#"(?:what\s+(?:is|are)|tell\s+me)\s+(?:the\s+)?contents?\s+of\s+["']?(\S+(?:\.[a-zA-Z0-9]+)(?:\S*))["']?\s*$"#).unwrap(),
                Regex::new(r#"(?:what\s+(?:is|are)\s+in)\s+["']?(\S+(?:\.[a-zA-Z0-9]+)(?:\S*))["']?\s*$"#).unwrap(),
                Regex::new(r#"(?:read|open|cat|view)\s+(?:the\s+)?(?:file\s+)?["']?(\S+(?:\.[a-zA-Z0-9]+)(?:\S*))["']?\s*$"#).unwrap(),
                Regex::new(r#"(?:show|display|view)\s+(?:me\s+)?(?:the\s+)?(?:file\s+)?["']?(\S+(?:\.[a-zA-Z0-9]+)(?:\S*))["']?\s*$"#).unwrap(),
                Regex::new(r#"(?:read|open|show|view)\s+["']?(\S+[\\/]\S+)["']?\s*$"#).unwrap(),
            ]),
            ("Write", vec![
                Regex::new(r#"(?:write|create|save)\s+(?:file\s+|to\s+|new\s+file\s+)?["']?(\S+(?:\.[a-zA-Z0-9]+)(?:\S*))["']?\s*$"#).unwrap(),
                Regex::new(r#"(?:write|create|save)\s+(?:file\s+|to\s+)?["']?(\S+)["']?\s*$"#).unwrap(),
            ]),
            ("Edit", vec![
                Regex::new(r#"(?:edit|modify|update|fix)\s+(?:file\s+)?["']?(\S+(?:\.[a-zA-Z0-9]+)(?:\S*))["']?\s*$"#).unwrap(),
                Regex::new(r#"replace\s+(?:in\s+)?["']?(\S+(?:\.[a-zA-Z0-9]+)(?:\S*))["']?\s*$"#).unwrap(),
                Regex::new(r#"(?:edit|modify|update|fix)\s+(?:file\s+)?["']?(\S+)["']?\s*$"#).unwrap(),
            ]),
            ("Bash", vec![
                Regex::new(r"^(?:run|execute|bash)\s+(.+)$").unwrap(),
                Regex::new(r"^(npm|pip|python|node|go|cargo|dotnet|code|docker|kubectl|choco|winget|scoop|reg|git|rustup|dir|ls|type|cd|mkdir|rmdir|del|copy|xcopy|robocopy|attrib|where|which|echo|setx|taskkill|tasklist)\s").unwrap(),
            ]),
            ("Glob", vec![
                Regex::new(r#"glob\s+["']?(.+?)["']?\s*$"#).unwrap(),
                Regex::new(r"(?:find|list)\s+(?:all\s+)?(.+?)\s+(?:files?|matching)\s").unwrap(),
                Regex::new(r#"list\s+(?:(?:all\s+)?(?:file|files)\s+(?:matching\s+)?)?["']?(.+?)["']?\s*$"#).unwrap(),
            ]),
            ("Grep", vec![
                Regex::new(r#"(?:search|grep)\s+(?:for\s+|text\s+|pattern\s+)?["']?(.+?)["']?\s*(?:in\s+["']?(.+?)["']?)?\s*$"#).unwrap(),
                Regex::new(r#"(?:find|search)\s+(?:text|pattern|occurrences)\s+["']?(.+?)["']?\s*(?:in\s+["']?(.+?)["']?)?\s*$"#).unwrap(),
            ]),
        ]
    };
}

pub fn flatten_tool_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => {
            arr.iter()
                .filter_map(|item| {
                    let type_ = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    if type_ == "text" {
                        item.get("text").and_then(|v| v.as_str())
                    } else {
                        None
                    }
                })
                .collect()
        }
        _ => value.to_string(),
    }
}

pub fn detect_tool_call(
    user_content: &serde_json::Value,
    available_tools: &[crate::models::ToolDef],
) -> Option<Vec<crate::models::ChatToolCall>> {
    let text = flatten_tool_text(user_content);
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    let mut matches: Vec<crate::models::ChatToolCall> = Vec::new();
    for tool_def in available_tools {
        let name = &tool_def.function.name;
        if let Some((_, patterns)) = COMPILED.iter().find(|(n, _)| *n == name) {
            for pat in patterns {
                if pat.is_match(text) {
                    let short = uuid::Uuid::new_v4().to_string().replace("-", "");
                    let tc_id = format!("call_{}", &short[..12]);
                    matches.push(crate::models::ChatToolCall {
                        id: tc_id,
                        type_: "function".into(),
                        function: crate::models::ToolCallFunction {
                            name: name.clone(),
                            arguments: "{}".into(),
                        },
                    });
                    break;
                }
            }
        }
    }
    if matches.is_empty() { None } else { Some(matches) }
}

fn safe_path(path_str: &str) -> std::path::PathBuf {
    let p = Path::new(path_str);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    }
}

fn truncate(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        text.to_owned()
    } else {
        format!("{}\n\n... (truncated, {} bytes omitted)", &text[..max_len], text.len() - max_len)
    }
}

fn try_init_kwargs(arguments: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(arguments).map_err(|e| format!("parse error: {e}"))
}

pub fn execute_tool(tool_name: &str, arguments: &str) -> String {
    let args = match try_init_kwargs(arguments) {
        Ok(v) => v,
        Err(e) => return format!("Error: could not parse arguments: {e}"),
    };
    match tool_name {
        "Read" => {
            let fp = args.get("filePath").and_then(|v| v.as_str()).unwrap_or("");
            let path = safe_path(fp);
            match std::fs::read_to_string(&path) {
                Ok(text) => truncate(&text, 100_000),
                Err(e) => format!("Error reading {}: {e}", path.display()),
            }
        }
        "Write" => {
            let fp = args.get("filePath").and_then(|v| v.as_str()).unwrap_or("");
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let path = safe_path(fp);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::write(&path, content) {
                Ok(_) => format!("Written {} bytes to {}", content.len(), path.display()),
                Err(e) => format!("Error writing {}: {e}", path.display()),
            }
        }
        "Edit" => {
            let fp = args.get("filePath").and_then(|v| v.as_str()).unwrap_or("");
            let old = args.get("oldString").and_then(|v| v.as_str()).unwrap_or("");
            let new = args.get("newString").and_then(|v| v.as_str()).unwrap_or("");
            let path = safe_path(fp);
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    if !text.contains(old) {
                        return format!("Error: oldString not found in {}", path.display());
                    }
                    let text = text.replace(old, new);
                    match std::fs::write(&path, &text) {
                        Ok(_) => format!("Edited {}: replaced 1 occurrence", path.display()),
                        Err(e) => format!("Error writing {}: {e}", path.display()),
                    }
                }
                Err(e) => format!("Error reading {}: {e}", path.display()),
            }
        }
        "Bash" => {
            let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            if command.is_empty() {
                return "Error: empty command".into();
            }
            match Command::new("cmd").args(["/C", command]).output() {
                Ok(output) => {
                    let mut result = String::from_utf8_lossy(&output.stdout).to_string();
                    if !output.stderr.is_empty() {
                        if !result.is_empty() { result.push('\n'); }
                        result.push_str(&String::from_utf8_lossy(&output.stderr));
                    }
                    if !output.status.success() {
                        format!("Command failed (exit {}):\n{}",
                            output.status.code().unwrap_or(-1), truncate(&result, 100_000))
                    } else if result.is_empty() {
                        "Command succeeded (no output)".into()
                    } else {
                        truncate(&result, 100_000)
                    }
                }
                Err(e) => format!("Error running command: {e}"),
            }
        }
        "Glob" => {
            let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
            let search_path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
            let base = safe_path(search_path);
            let glob_pattern = format!("{}/{}", base.display(), pattern);
            match glob::glob(&glob_pattern) {
                Ok(entries) => {
                    let mut results: Vec<String> = entries
                        .filter_map(|e| e.ok())
                        .filter_map(|p| p.strip_prefix(&base).ok().map(|r| r.display().to_string()))
                        .collect();
                    results.sort();
                    if results.is_empty() {
                        format!("No files matching '{pattern}' in {}", base.display())
                    } else {
                        results.truncate(200);
                        results.join("\n")
                    }
                }
                Err(e) => format!("Error globbing: {e}"),
            }
        }
        "Grep" => {
            let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
            let include = args.get("include").and_then(|v| v.as_str());
            let search_path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
            let base = safe_path(search_path);
            let re = match Regex::new(pattern) {
                Ok(r) => r,
                Err(e) => return format!("Error: invalid regex '{pattern}': {e}"),
            };
            let files: Vec<std::path::PathBuf> = if let Some(g) = include {
                let gp = format!("{}/{}", base.display(), g);
                glob::glob(&gp).into_iter().flatten().filter_map(|e| e.ok()).filter(|p| p.is_file()).collect()
            } else {
                vec![base.clone()]
            };
            let mut results: Vec<String> = Vec::new();
            for f in &files {
                if !f.is_file() { continue; }
                let content = match std::fs::read_to_string(f) {
                    Ok(c) => c, Err(_) => continue,
                };
                for (i, line) in content.lines().enumerate() {
                    if re.is_match(line) {
                        let rel = f.strip_prefix(&base).map(|r| r.display().to_string()).unwrap_or_else(|_| f.display().to_string());
                        let truncated = if line.len() > 200 { &line[..200] } else { line };
                        results.push(format!("{rel}:{}: {truncated}", i + 1));
                        if results.len() >= 200 { break; }
                    }
                }
                if results.len() >= 200 { break; }
            }
            if results.is_empty() { format!("No matches for '{pattern}'") } else { results.join("\n") }
        }
        _ => format!("Error: unknown tool '{tool_name}'"),
    }
}
