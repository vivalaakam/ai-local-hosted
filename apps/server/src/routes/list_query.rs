#[derive(Deserialize)]
struct ListQuery {
    cursor: Option<String>,
    limit: Option<i64>,
}

