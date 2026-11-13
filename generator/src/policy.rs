//! The choices the API definitions don't express, from `tools.toml` beside
//! the generator: methods left without a tool, fields left out of a tool,
//! fields a tool requires, and changes that can't be undone.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// `tools.toml`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// Methods left without a tool.
    #[serde(default)]
    pub no_tool: Vec<NoTool>,
    /// Fields left out of a method's tool.
    #[serde(default)]
    pub exclude: Vec<Exclusion>,
    /// Fields a method's tool requires although their field behavior
    /// doesn't.
    #[serde(default)]
    pub require: Vec<Requirement>,
    /// Fields whose change can't be undone: the tool is destructive.
    #[serde(default)]
    pub one_way: Vec<OneWay>,
}

/// A method left without a tool.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoTool {
    /// The service, as its packages name it: `pvs`, or `longrunning` for
    /// the operation methods every service with operations has.
    pub service: String,
    /// The method, such as `GetVolume`.
    pub method: String,
    /// Why, for whoever reads the table.
    #[allow(dead_code)]
    pub why: String,
}

/// A field left out of a method's tool.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exclusion {
    /// The service, as its packages name it: `pvs`.
    pub service: String,
    /// The method, such as `ImportVolume`.
    pub method: String,
    /// The field, as a path from the request, such as
    /// `volume.source_snapshot`.
    pub field: String,
    /// Why, for whoever reads the table.
    #[allow(dead_code)]
    pub why: String,
}

/// A field a method's tool requires although its field behavior doesn't.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    pub service: String,
    pub method: String,
    /// The field, as a path from the request, such as `key_pair.public_key`.
    pub field: String,
    /// Why the tool requires it, for the agent reading its description.
    pub why: String,
}

/// A field whose change can't be undone.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OneWay {
    pub service: String,
    pub method: String,
    pub field: String,
    /// The verb of the `origin` CLI's refusal without `--yes`, `refusing to
    /// {verb} volume …`: the CLI's generator reads this table too, and the
    /// tools have no use for it.
    #[allow(dead_code)]
    pub verb: String,
    /// What can't be undone, for someone deciding whether to go on.
    pub consequence: String,
}

impl Policy {
    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("reading {}", path.display()))
    }

    pub fn no_tool(&self, service: &str, method: &str) -> bool {
        self.no_tool
            .iter()
            .any(|n| n.service == service && n.method == method)
    }

    pub fn excluded(&self, service: &str, method: &str, field: &str) -> bool {
        self.exclude
            .iter()
            .any(|e| e.service == service && e.method == method && e.field == field)
    }

    /// Whether the tool of `method` requires `field`, a path from its
    /// request: a field the table requires, or a message on the way to one.
    pub fn requires(&self, service: &str, method: &str, field: &str) -> bool {
        self.require.iter().any(|r| {
            r.service == service
                && r.method == method
                && (r.field == field
                    || r.field
                        .strip_prefix(field)
                        .is_some_and(|rest| rest.starts_with('.')))
        })
    }

    /// Every `(service, method, field)` the table names; the field is empty
    /// for a method left without a tool.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &str, &str)> {
        let no_tool = self
            .no_tool
            .iter()
            .map(|n| (n.service.as_str(), n.method.as_str(), ""));
        let exclude = self
            .exclude
            .iter()
            .map(|e| (e.service.as_str(), e.method.as_str(), e.field.as_str()));
        let require = self
            .require
            .iter()
            .map(|r| (r.service.as_str(), r.method.as_str(), r.field.as_str()));
        let one_way = self
            .one_way
            .iter()
            .map(|o| (o.service.as_str(), o.method.as_str(), o.field.as_str()));
        no_tool.chain(exclude).chain(require).chain(one_way)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_parses_and_answers_by_service() {
        let policy: Policy = toml::from_str(
            r#"
            [[no_tool]]
            service = "pvs"
            method = "ImportVolume"
            why = "z"

            [[exclude]]
            service = "pvs"
            method = "ImportVolume"
            field = "volume.source_snapshot"
            why = "y"

            [[require]]
            service = "pvs"
            method = "CreateVolume"
            field = "volume.size_gib"
            why = "x"

            [[one_way]]
            service = "pvs"
            method = "UpdateVolume"
            field = "volume.size_gib"
            verb = "grow"
            consequence = "w"
            "#,
        )
        .unwrap();
        assert!(policy.no_tool("pvs", "ImportVolume"));
        assert!(!policy.no_tool("longrunning", "WaitOperation"));
        assert!(policy.excluded("pvs", "ImportVolume", "volume.source_snapshot"));
        assert!(!policy.excluded("other", "ImportVolume", "volume.source_snapshot"));
        assert!(policy.requires("pvs", "CreateVolume", "volume.size_gib"));
        // The message on the way to a required field is required too.
        assert!(policy.requires("pvs", "CreateVolume", "volume"));
        assert!(!policy.requires("pvs", "CreateVolume", "volume.size"));
        assert!(!policy.requires("pvs", "CreateVolume", "vol"));
        assert!(!policy.requires("pvs", "UpdateVolume", "volume.size_gib"));
        assert_eq!(policy.one_way[0].verb, "grow");
        assert_eq!(policy.entries().count(), 4);
        // Every entry says why.
        assert!(
            toml::from_str::<Policy>("[[no_tool]]\nservice = \"pvs\"\nmethod = \"X\"\n").is_err()
        );
        // A one-way change names the CLI's verb, which the CLI reads from here.
        assert!(toml::from_str::<Policy>(
            "[[one_way]]\nservice = \"pvs\"\nmethod = \"X\"\nfield = \"y\"\nconsequence = \"z\"\n"
        )
        .is_err());
    }
}
