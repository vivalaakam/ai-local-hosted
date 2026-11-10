#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_text_thinking_tools_files_and_the_active_branch() {
        let raw = r#"{
            "conversations": [
                {
                    "uuid": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                    "name": "From Claude",
                    "model": "claude-sonnet-4-5",
                    "created_at": "2024-06-01T00:00:00Z",
                    "updated_at": "2024-06-01T00:05:00Z",
                    "chat_messages": [
                        {
                            "uuid": "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                            "sender": "human",
                            "text": "Hello from the export",
                            "created_at": "2024-06-01T00:00:00Z",
                            "attachments": [{
                                "file_name": "notes.txt",
                                "extracted_content": "remember this"
                            }]
                        },
                        {
                            "uuid": "cccccccc-cccc-cccc-cccc-cccccccccccc",
                            "sender": "assistant",
                            "created_at": "2024-06-01T00:01:00Z",
                            "content": [
                                {"type": "thinking", "thinking": "plan"},
                                {"type": "text", "text": "Here you go"},
                                {"type": "tool_use", "name": "search", "input": {"q": "alh"}}
                            ]
                        }
                    ]
                },
                {
                    "uuid": "dddddddd-dddd-dddd-dddd-dddddddddddd",
                    "name": "Branched",
                    "created_at": "2024-05-01T00:00:00Z",
                    "updated_at": "2024-05-01T00:00:02Z",
                    "current_leaf_message_uuid": "ffffffff-ffff-ffff-ffff-ffffffffffff",
                    "chat_messages": [
                        {"uuid": "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee", "sender": "human", "text": "question", "created_at": "2024-05-01T00:00:00Z"},
                        {"uuid": "99999999-9999-9999-9999-999999999999", "parent_message_uuid": "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee", "sender": "assistant", "text": "old", "created_at": "2024-05-01T00:00:03Z"},
                        {"uuid": "ffffffff-ffff-ffff-ffff-ffffffffffff", "parent_message_uuid": "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee", "sender": "assistant", "text": "chosen", "created_at": "2024-05-01T00:00:01Z"}
                    ]
                }
            ]
        }"#;
        let chats = parse_claude_export(raw.as_bytes()).unwrap();
        assert_eq!(chats.len(), 2);
        assert_eq!(chats[0].title, "From Claude");
        assert_eq!(chats[0].model, "claude-sonnet-4-5");
        assert_eq!(chats[0].messages.len(), 2);
        assert_eq!(chats[0].messages[0].role, "user");
        assert!(
            chats[0].messages[0]
                .content
                .contains("Hello from the export")
        );
        assert!(chats[0].messages[0].content.contains("remember this"));
        assert_eq!(chats[0].messages[1].thinking.as_deref(), Some("plan"));
        assert!(chats[0].messages[1].content.contains("Here you go"));
        assert!(chats[0].messages[1].content.contains("[tool search]"));
        assert_eq!(chats[1].messages.len(), 2);
        assert_eq!(chats[1].messages[1].content, "chosen");
    }

    #[test]
    fn manifest_is_not_a_conversation_file() {
        let err = parse_claude_export(br#"{"manifest":"x","files":[]}"#).unwrap_err();
        assert!(err.contains("manifest"));
    }
}
