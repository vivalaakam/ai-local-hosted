use alh_desktop::{Command, ToolDraft};
use gpui_kit::Context;
use uuid::Uuid;

use super::{ChatWindow, valid_slug};

const DEFAULT_PARAMETERS: &str = r#"{
  "type": "object",
  "properties": {}
}"#;

const DEFAULT_SOURCE: &str = r#"fn run(args) {
  ""
}"#;

impl ChatWindow {
    pub(crate) fn close_tool_form(&mut self) {
        self.tool_form_open = false;
        self.tool_form_readonly = false;
        self.editing_tool = None;
        self.tool_enabled = true;
        self.tool_slug_text = Some(String::new());
        self.tool_name_text = Some(String::new());
        self.tool_description_text = Some(String::new());
        self.tool_parameters_text = Some(DEFAULT_PARAMETERS.into());
        self.tool_source_text = Some(DEFAULT_SOURCE.into());
    }

    pub(crate) fn open_new_tool(&mut self) {
        self.close_tool_form();
        self.tool_form_open = true;
        self.tool_form_readonly = false;
    }

    pub(crate) fn view_tool(&mut self, id: Uuid) {
        let Some(row) = self.tools.iter().find(|row| row.id == id).cloned() else {
            return;
        };
        self.tool_form_open = true;
        self.tool_form_readonly = row.is_system();
        self.editing_tool = Some(row.id);
        self.tool_enabled = row.enabled;
        self.tool_slug_text = Some(row.slug);
        self.tool_name_text = Some(row.name);
        self.tool_description_text = Some(row.description);
        self.tool_parameters_text = Some(pretty_json(&row.parameters));
        self.tool_source_text = Some(row.source.unwrap_or_else(|| {
            if row.kind == "native" {
                "// Native tool implemented in Rust".into()
            } else {
                String::new()
            }
        }));
    }

    pub(crate) fn edit_tool(&mut self, id: Uuid) {
        let Some(row) = self.tools.iter().find(|row| row.id == id).cloned() else {
            return;
        };
        if row.is_system() {
            self.view_tool(id);
            return;
        }
        self.view_tool(id);
        self.tool_form_readonly = false;
    }

    pub(crate) fn save_tool(&mut self, cx: &mut Context<Self>) {
        if self.tool_form_readonly {
            return;
        }
        let slug = self.tool_slug_input.read(cx).value().trim().to_string();
        let name = self.tool_name_input.read(cx).value().trim().to_string();
        let description = self
            .tool_description_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let parameters = self
            .tool_parameters_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let source = self.tool_source_input.read(cx).value().to_string();
        if self.editing_tool.is_none() && !valid_slug(&slug) {
            self.banner =
                Some("Tool slug must be ascii alphanumeric, '_' or '-', up to 64 chars".into());
            cx.notify();
            return;
        }
        if name.is_empty() || name.chars().count() > 200 {
            self.banner = Some("Tool name is required".into());
            cx.notify();
            return;
        }
        if serde_json::from_str::<serde_json::Value>(&parameters).is_err() {
            self.banner = Some("Parameters must be valid JSON".into());
            cx.notify();
            return;
        }
        if source.trim().is_empty() {
            self.banner = Some("Rhai source is required".into());
            cx.notify();
            return;
        }
        self.bridge.send(Command::SaveTool(ToolDraft {
            id: self.editing_tool,
            slug,
            name,
            description,
            parameters,
            source,
            enabled: self.tool_enabled,
            update: self.editing_tool.is_some(),
            reset_form: true,
        }));
        cx.notify();
    }

    pub(crate) fn delete_tool(&mut self, id: Uuid) {
        if self
            .tools
            .iter()
            .find(|row| row.id == id)
            .is_some_and(|row| row.is_system())
        {
            return;
        }
        self.bridge.send(Command::DeleteTool(id));
    }

    pub(crate) fn save_system_prompt(&mut self, cx: &mut Context<Self>) {
        let text = self.system_prompt_input.read(cx).value();
        let content = text.trim();
        let content = if content.is_empty() {
            None
        } else {
            Some(content.to_string())
        };
        self.bridge.send(Command::SaveSystemPrompt(content));
        cx.notify();
    }
}

fn pretty_json(raw: &str) -> String {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| raw.to_string())
}
