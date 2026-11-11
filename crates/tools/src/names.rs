pub const USER_PREFIX: &str = "user__";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolOwner {
    System,
    User,
}

pub fn model_name(owner: ToolOwner, slug: &str) -> String {
    match owner {
        ToolOwner::System => slug.to_string(),
        ToolOwner::User => format!("{USER_PREFIX}{slug}"),
    }
}

pub fn parse_model_name(name: &str) -> (ToolOwner, &str) {
    if let Some(slug) = name.strip_prefix(USER_PREFIX) {
        (ToolOwner::User, slug)
    } else {
        (ToolOwner::System, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_user_and_system_names() {
        assert_eq!(
            model_name(ToolOwner::System, "current_time"),
            "current_time"
        );
        assert_eq!(model_name(ToolOwner::User, "ping"), "user__ping");
        assert_eq!(parse_model_name("user__ping"), (ToolOwner::User, "ping"));
        assert_eq!(
            parse_model_name("get_weather"),
            (ToolOwner::System, "get_weather")
        );
    }
}
