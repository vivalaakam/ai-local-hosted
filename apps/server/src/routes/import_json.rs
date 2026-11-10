fn import_json(counts: ImportCounts) -> Value {
    json!({
        "conversations": counts.conversations,
        "skipped": counts.skipped,
        "messages": counts.messages,
    })
}

