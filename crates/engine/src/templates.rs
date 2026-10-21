//! Text templates ("Mijn sjablonen" in Outlook), kept in `templates.json` next to the accounts
//! file. The compose window lists them in a menu and drops the chosen text into the message.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use merel_model::{Template, TemplateError};

use crate::Session;
use crate::commands::{CommandError, str_param};

/// What `templates.json` holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplatesFile {
    pub templates: Vec<Template>,
}

impl Session {
    pub fn set_templates(&mut self, templates: Vec<Template>) {
        self.templates = templates;
    }

    pub fn templates_data(&self) -> TemplatesFile {
        TemplatesFile { templates: self.templates.clone() }
    }

    /// Whether a command changed the templates since the last call.
    pub fn take_templates_dirty(&mut self) -> bool {
        std::mem::take(&mut self.templates_dirty)
    }

    pub fn template(&self, id: &str) -> Option<&Template> {
        self.templates.iter().find(|t| t.id == id)
    }

    /// Every template, by name.
    pub fn templates_sorted(&self) -> Vec<&Template> {
        let mut list: Vec<&Template> = self.templates.iter().collect();
        list.sort_by_cached_key(|t| t.name.to_lowercase());
        list
    }

    fn new_template_id(&mut self) -> String {
        self.next_template_id += 1;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        format!("t-{now}-{}", self.next_template_id)
    }
}

/// `template.save { id?, name, text }`: creates a template or replaces the one with `id`.
pub fn save(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let existing = match str_param(params, "id") {
        Ok(id) => Some(s.template(id).ok_or(CommandError::NotFound)?.clone()),
        Err(_) => None,
    };
    let template = Template {
        id: existing.as_ref().map(|t| t.id.clone()).unwrap_or_else(|| s.new_template_id()),
        name: str_param(params, "name").unwrap_or("").to_string(),
        text: str_param(params, "text").unwrap_or("").to_string(),
    }
    .validated()
    .map_err(|e| match e {
        TemplateError::NoName => CommandError::Param("name"),
        TemplateError::NoText | TemplateError::TooLong => CommandError::Param("text"),
    })?;
    let id = template.id.clone();
    match s.templates.iter_mut().find(|t| t.id == id) {
        Some(slot) => *slot = template,
        None => s.templates.push(template),
    }
    s.templates_dirty = true;
    Ok(())
}

/// `template.delete { id }`: removes the template.
pub fn delete(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id")?;
    let before = s.templates.len();
    s.templates.retain(|t| t.id != id);
    if s.templates.len() == before {
        return Err(CommandError::NotFound);
    }
    s.templates_dirty = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_template_is_saved_listed_by_name_changed_and_removed() {
        let mut s = Session::default();
        s.run("template.save", &json!({ "name": "Route", "text": "Kom via de A2.\nAfslag 9." })).unwrap();
        s.run("template.save", &json!({ "name": "Bedankt", "text": "Dank voor je bericht." })).unwrap();
        assert!(s.take_templates_dirty() && !s.take_templates_dirty());
        let names: Vec<&str> = s.templates_sorted().iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Bedankt", "Route"]);
        let id = s.templates_sorted()[1].id.clone();
        s.run("template.save", &json!({ "id": id, "name": "Routebeschrijving", "text": "Via de A2." })).unwrap();
        assert_eq!(s.templates.len(), 2);
        assert_eq!(s.template(&id).map(|t| (t.name.as_str(), t.text.as_str())), Some(("Routebeschrijving", "Via de A2.")));
        assert_eq!(s.run("template.save", &json!({ "name": "Leeg", "text": " " })), Err(CommandError::Param("text")));
        assert_eq!(s.run("template.save", &json!({ "name": "", "text": "x" })), Err(CommandError::Param("name")));
        assert_eq!(s.run("template.save", &json!({ "id": "nope", "name": "x", "text": "x" })), Err(CommandError::NotFound));
        s.run("template.delete", &json!({ "id": id })).unwrap();
        assert_eq!(s.templates.len(), 1);
        assert!(s.take_templates_dirty());
        assert_eq!(s.run("template.delete", &json!({ "id": id })), Err(CommandError::NotFound));
    }
}
