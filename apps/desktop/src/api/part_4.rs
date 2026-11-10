#[derive(Deserialize)]
struct MessageBody {
    id: Uuid,
    role: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    error_code: Option<String>,
}
