//! Descriptions: the proto comments, in the words of the tools' JSON.
//!
//! - the API linter's directives, `(-- ... --)`, are removed;
//! - a field in backticks, such as `size_gib`, becomes its JSON name,
//!   `sizeGib`.

use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;

use heck::ToLowerCamelCase;
use prost_types::FileDescriptorSet;
use regex::Regex;

static LINTER_DIRECTIVE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)\(--.*?--\)").expect("a valid pattern"));
static BACKTICKED_FIELD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`([a-z][a-z0-9_]*)`").expect("a valid pattern"));

/// The leading comments of the API's files, by file and location path.
pub struct Comments {
    leading: HashMap<(String, Vec<i32>), String>,
}

impl Comments {
    pub fn new(fds: &FileDescriptorSet) -> Self {
        let mut leading = HashMap::new();
        for file in &fds.file {
            let Some(info) = &file.source_code_info else {
                continue;
            };
            for location in &info.location {
                if let Some(text) = &location.leading_comments {
                    leading.insert(
                        (file.name().to_owned(), location.path.clone()),
                        text.clone(),
                    );
                }
            }
        }
        Self { leading }
    }

    /// The leading comment at `path` in `file`; empty when there is none.
    pub fn leading(&self, file: &str, path: &[i32]) -> &str {
        self.leading
            .get(&(file.to_owned(), path.to_vec()))
            .map_or("", String::as_str)
    }
}

/// `comment` as a description: without linter directives and trailing
/// blank lines, the package's `fields` in backticks by their JSON names.
pub fn help(comment: &str, fields: &BTreeSet<String>) -> String {
    let text = LINTER_DIRECTIVE.replace_all(comment, "");
    let text = BACKTICKED_FIELD.replace_all(&text, |caps: &regex::Captures<'_>| {
        let field = &caps[1];
        if fields.contains(field) {
            format!("`{}`", field.to_lower_camel_case())
        } else {
            caps[0].to_owned()
        }
    });
    let mut lines: Vec<&str> = text
        .lines()
        .map(|l| l.strip_prefix(' ').unwrap_or(l))
        .collect();
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    lines
        .iter()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

/// `text` as `#[doc]` attributes, one per line.
pub fn attributes(text: &str, indent: &str) -> String {
    text.lines()
        .map(|line| {
            if line.is_empty() {
                format!("{indent}#[doc = \"\"]\n")
            } else {
                format!("{indent}#[doc = {:?}]\n", format!(" {line}"))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_become_json_names() {
        let fields = BTreeSet::from(["size_gib".to_owned(), "next_page_token".to_owned()]);
        assert_eq!(
            help(
                " A larger `size_gib` grows it; see `next_page_token` and `AVAILABLE`.\n\n (-- api-linter: x\n    y --)\n",
                &fields
            ),
            "A larger `sizeGib` grows it; see `nextPageToken` and `AVAILABLE`."
        );
    }
}
