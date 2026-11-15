//! The services whose full resource names a path field takes: those of the
//! type the field refers to, as the Rust SDK's generator decides them, so
//! that a tool builds the path of a call as the SDK's client does.

use std::collections::{BTreeMap, BTreeSet};

use prost_reflect::{DescriptorPool, DynamicMessage, FieldDescriptor, MessageDescriptor, Value};

use crate::descriptors::message_extension;

/// The domain every service's host is under.
const DOMAIN: &str = "origincloudapis.eu";

/// A path field of every resource service: `type: "*"`.
pub const ANY: &str = "*";

/// The resource types the API declares in its files (`origincloud/`).
pub struct Resources {
    /// Each type's patterns, by type.
    patterns: BTreeMap<String, Vec<String>>,
}

impl Resources {
    /// Every resource type the API declares, with `google.api.resource` on
    /// a message or `google.api.resource_definition` on a file.
    pub fn index(pool: &DescriptorPool) -> Resources {
        let mut patterns: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut add = |resource: &DynamicMessage| {
            let (resource_type, declared) = descriptor(resource);
            patterns.entry(resource_type).or_default().extend(declared);
        };
        for file in pool
            .files()
            .filter(|f| f.name().starts_with("origincloud/"))
        {
            if let Some(ext) = pool.get_extension_by_name("google.api.resource_definition") {
                let options = file.options();
                if options.has_extension(&ext) {
                    if let Value::List(definitions) = options.get_extension(&ext).into_owned() {
                        for definition in definitions {
                            if let Value::Message(definition) = definition {
                                add(&definition);
                            }
                        }
                    }
                }
            }
            let mut pending: Vec<MessageDescriptor> = file.messages().collect();
            while let Some(m) = pending.pop() {
                if let Some(resource) = message_extension(pool, &m.options(), "google.api.resource")
                {
                    add(&resource);
                }
                pending.extend(m.child_messages());
            }
        }
        Resources { patterns }
    }

    /// The short codes of the services whose full names the path field
    /// `field` takes, in order: the service of the type the field refers to.
    ///
    /// - `google.api.resource_reference` with `type: "*"`: every resource
    ///   service, [`ANY`].
    /// - With a `type`: that type's service.
    /// - With a `child_type`: the services of the declared types whose
    ///   pattern is a pattern of the child's without its last collection and
    ///   id, its parent; the child's own service when no declared type has
    ///   the parent pattern (`projects/{project}/zones/{zone}`).
    /// - The `name` of a resource, a message with `google.api.resource`:
    ///   that resource's service.
    /// - Otherwise: none, which the SDK's client reads as its own service.
    pub fn path_services(
        &self,
        pool: &DescriptorPool,
        field: &FieldDescriptor,
    ) -> Result<Vec<String>, String> {
        if let Some(reference) = field_extension(pool, field, "google.api.resource_reference") {
            let text = |name: &str| {
                reference
                    .get_field_by_name(name)
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .filter(|t| !t.is_empty())
            };
            if let Some(resource_type) = text("type") {
                if resource_type == ANY {
                    return Ok(vec![ANY.to_owned()]);
                }
                return Ok(vec![service_of(&resource_type)?]);
            }
            if let Some(child) = text("child_type") {
                let own = service_of(&child)?;
                return Ok(self.parent_services(&child).unwrap_or_else(|| vec![own]));
            }
        }
        if field.name() != "name" {
            return Ok(Vec::new());
        }
        match message_extension(
            pool,
            &field.parent_message().options(),
            "google.api.resource",
        ) {
            Some(resource) => Ok(vec![service_of(&descriptor(&resource).0)?]),
            None => Ok(Vec::new()),
        }
    }

    /// The services of the declared parents of `child`, in order; `None`
    /// when no declared type is its parent.
    fn parent_services(&self, child: &str) -> Option<Vec<String>> {
        let parents: BTreeSet<&str> = self
            .patterns
            .get(child)?
            .iter()
            .filter_map(|p| p.rsplitn(3, '/').nth(2))
            .collect();
        let services: BTreeSet<String> = self
            .patterns
            .iter()
            .filter(|(_, patterns)| patterns.iter().any(|p| parents.contains(p.as_str())))
            .filter_map(|(t, _)| service_of(t).ok())
            .collect();
        (!services.is_empty()).then(|| services.into_iter().collect())
    }
}

/// The message extension `name` on a field's options.
fn field_extension(
    pool: &DescriptorPool,
    field: &FieldDescriptor,
    name: &str,
) -> Option<DynamicMessage> {
    let ext = pool.get_extension_by_name(name)?;
    let options = field.options();
    if !options.has_extension(&ext) {
        return None;
    }
    match options.get_extension(&ext).into_owned() {
        Value::Message(m) => Some(m),
        _ => None,
    }
}

/// The `type` and `pattern`s of a `google.api.ResourceDescriptor`.
fn descriptor(resource: &DynamicMessage) -> (String, Vec<String>) {
    let resource_type = resource
        .get_field_by_name("type")
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let patterns = match resource.get_field_by_name("pattern").as_deref() {
        Some(Value::List(patterns)) => patterns
            .iter()
            .filter_map(|p| p.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    };
    (resource_type, patterns)
}

/// The short code of the service owning `resource_type`,
/// `{service}.origincloudapis.eu/{Kind}`.
fn service_of(resource_type: &str) -> Result<String, String> {
    resource_type
        .split_once('/')
        .and_then(|(domain, _)| domain.strip_suffix(DOMAIN)?.strip_suffix('.'))
        .filter(|s| {
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
        .map(str::to_owned)
        .ok_or_else(|| {
            format!("the resource type {resource_type:?} isn't {{service}}.{DOMAIN}/{{Kind}}")
        })
}
