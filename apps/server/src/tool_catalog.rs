use alh_storage::ToolRow;
use alh_tools::{ToolKind, ToolSpec};

pub fn row_to_spec(row: ToolRow) -> Result<ToolSpec, String> {
    let kind =
        ToolKind::parse(&row.kind).ok_or_else(|| format!("unknown tool kind {}", row.kind))?;
    Ok(ToolSpec {
        id: row.id,
        owner_user_id: row.owner_user_id,
        slug: row.slug,
        name: row.name,
        description: row.description,
        parameters: row.parameters,
        kind,
        source: row.source,
        enabled: row.enabled,
    })
}
