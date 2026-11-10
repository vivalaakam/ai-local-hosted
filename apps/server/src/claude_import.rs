//! Read a Claude export zip, or a raw `conversations.json`, into chats.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use alh_core::{ImportedChat, parse_claude_documents};
use uuid::Uuid;

const MAX_EXPORT_BYTES: usize = 512 * 1024 * 1024;

pub fn load_claude_chats(bytes: &[u8]) -> Result<Vec<ImportedChat>, String> {
    if bytes.is_empty() {
        return Err("Claude export is empty".into());
    }
    if bytes.len() > MAX_EXPORT_BYTES {
        return Err("Claude export is larger than 512 MB".into());
    }
    let documents = if is_zip(bytes) {
        conversation_documents(bytes, 0)?
    } else {
        vec![bytes.to_vec()]
    };
    if documents.is_empty() {
        return Err("No conversations.json in that archive.".into());
    }
    parse_claude_documents(&documents)
}

fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK")
}

fn conversation_documents(bytes: &[u8], depth: u8) -> Result<Vec<Vec<u8>>, String> {
    if depth > 2 {
        return Err("Claude export archive is nested too deeply".into());
    }
    let file = TempZip::new(bytes)?;
    let names = zip_names(&file.0)?;
    let sizes = zip_sizes(&file.0);
    let mut json_names: Vec<String> = names
        .iter()
        .filter(|name| is_conversation_json(name))
        .cloned()
        .collect();
    json_names.sort();
    let mut zip_names_nested: Vec<String> = names
        .iter()
        .filter(|name| is_conversation_zip(name))
        .cloned()
        .collect();
    zip_names_nested.sort();
    let selected = json_names.len() + zip_names_nested.len();
    if selected == 0 {
        return Err("No conversations.json in that archive.".into());
    }
    if selected > 100 {
        return Err("Too many conversation files in the archive.".into());
    }
    let total: u64 = json_names
        .iter()
        .chain(zip_names_nested.iter())
        .filter_map(|name| sizes.get(name))
        .sum();
    if total > MAX_EXPORT_BYTES as u64 {
        return Err("Claude export is larger than 512 MB".into());
    }
    let mut documents = Vec::new();
    for name in &json_names {
        documents.push(unzip_member(&file.0, name)?);
    }
    for name in &zip_names_nested {
        let nested = unzip_member(&file.0, name)?;
        if is_zip(&nested) {
            documents.extend(conversation_documents(&nested, depth + 1)?);
        }
    }
    Ok(documents)
}

fn is_conversation_json(name: &str) -> bool {
    let base = base_name(name);
    if base.starts_with('.') || base.starts_with("__MACOSX") {
        return false;
    }
    base == "conversations.json" || conversation_shard(&base)
}

fn conversation_shard(base: &str) -> bool {
    let Some(rest) = base.strip_prefix("conversations-") else {
        return false;
    };
    let Some(number) = rest.strip_suffix(".json") else {
        return false;
    };
    !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_conversation_zip(name: &str) -> bool {
    let base = base_name(name);
    base.contains("conversation") && base.ends_with(".zip")
}

fn base_name(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .trim()
        .to_ascii_lowercase()
}

fn zip_names(path: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("unzip")
        .arg("-Z1")
        .arg(path)
        .output()
        .map_err(|_| "unzip is required to read a Claude export archive".to_string())?;
    if !output.status.success() {
        return Err(unzip_failure(&output.stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim_end().to_string())
        .filter(|line| !line.is_empty() && !line.ends_with('/'))
        .collect())
}

fn zip_sizes(path: &Path) -> BTreeMap<String, u64> {
    let Ok(output) = Command::new("unzip").arg("-l").arg(path).output() else {
        return BTreeMap::new();
    };
    if !output.status.success() {
        return BTreeMap::new();
    }
    let mut sizes = BTreeMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some((size, name)) = listed_size(line) {
            sizes.insert(name, size);
        }
    }
    sizes
}

fn listed_size(line: &str) -> Option<(u64, String)> {
    let trimmed = line.trim();
    let mut rest = trimmed;
    let size_end = rest.find(char::is_whitespace)?;
    let size: u64 = rest[..size_end].parse().ok()?;
    rest = rest[size_end..].trim_start();
    let date_end = rest.find(char::is_whitespace)?;
    let date = &rest[..date_end];
    if !date.contains('-') {
        return None;
    }
    rest = rest[date_end..].trim_start();
    let time_end = rest.find(char::is_whitespace)?;
    let time = &rest[..time_end];
    if !time.contains(':') {
        return None;
    }
    let name = rest[time_end..].trim();
    if name.is_empty() {
        return None;
    }
    Some((size, name.to_string()))
}

fn unzip_member(path: &Path, name: &str) -> Result<Vec<u8>, String> {
    let output = Command::new("unzip")
        .arg("-p")
        .arg("--")
        .arg(path)
        .arg(name)
        .output()
        .map_err(|_| "unzip is required to read a Claude export archive".to_string())?;
    if !output.status.success() {
        return Err(format!(
            "could not read {name} ({})",
            unzip_failure(&output.stderr)
        ));
    }
    if output.stdout.len() > MAX_EXPORT_BYTES {
        return Err("Claude export is larger than 512 MB".into());
    }
    Ok(output.stdout)
}

fn unzip_failure(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    if text.is_empty() {
        "could not read the Claude export archive".to_string()
    } else {
        text.to_string()
    }
}

struct TempZip(PathBuf);

impl TempZip {
    fn new(bytes: &[u8]) -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!("alh-claude-{}.zip", Uuid::new_v4()));
        std::fs::write(&path, bytes)
            .map_err(|err| format!("could not read the archive ({err})"))?;
        Ok(Self(path))
    }
}

impl Drop for TempZip {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
        let dir = std::env::temp_dir().join(format!("alh-claude-src-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut args = vec!["-q".to_string(), "export.zip".to_string()];
        for (name, bytes) in files {
            std::fs::write(dir.join(name), bytes).unwrap();
            args.push((*name).to_string());
        }
        let status = Command::new("zip")
            .args(&args)
            .current_dir(&dir)
            .status()
            .expect("zip");
        assert!(status.success());
        let bytes = std::fs::read(dir.join("export.zip")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        bytes
    }

    #[test]
    fn reads_conversations_json_from_a_zip_and_a_nested_shard() {
        let chat = br#"[{"uuid":"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa","name":"Nested","created_at":"2024-01-02T03:04:05Z","updated_at":"2024-01-02T03:04:05Z","chat_messages":[{"uuid":"bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb","sender":"human","text":"inside","created_at":"2024-01-02T03:04:05Z"}]}]"#;
        let inner = zip_bytes(&[("conversations.json", chat)]);
        let dir = std::env::temp_dir().join(format!("alh-claude-outer-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("conversations-000.zip"), &inner).unwrap();
        let status = Command::new("zip")
            .args(["-q", "outer.zip", "conversations-000.zip"])
            .current_dir(&dir)
            .status()
            .unwrap();
        assert!(status.success());
        let outer = std::fs::read(dir.join("outer.zip")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let chats = load_claude_chats(&outer).unwrap();
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].title, "Nested");
        assert_eq!(chats[0].messages[0].content, "inside");
    }
}
