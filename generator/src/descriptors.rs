//! What the generator reads from the API's descriptors: its services and
//! packages, field behaviors, long-running operations, and the Rust SDK's
//! names for them.

use anyhow::Result;
use heck::{ToSnakeCase, ToUpperCamelCase};
use prost_reflect::{
    DescriptorPool, DynamicMessage, FieldDescriptor, MessageDescriptor, MethodDescriptor, Value,
};

pub(crate) const EMPTY: &str = "google.protobuf.Empty";
pub(crate) const FIELD_MASK: &str = "google.protobuf.FieldMask";
pub(crate) const DURATION: &str = "google.protobuf.Duration";
/// The service whose package holds every service's operation methods:
/// `origincloud.longrunning.{version}`. Each service serves them at its own
/// host, for the operations its methods answer with.
pub(crate) const OPERATIONS: &str = "longrunning";

// `google.api.FieldBehavior` values.
pub(crate) const REQUIRED: i32 = 2;
pub(crate) const OUTPUT_ONLY: i32 = 3;
pub(crate) const IMMUTABLE: i32 = 5;
pub(crate) const IDENTIFIER: i32 = 8;

/// The service a package names: `pvs` for `origincloud.pvs.v1beta1`.
pub(crate) fn service_of(package: &str) -> &str {
    package.split('.').nth(1).unwrap_or_default()
}
/// The Rust SDK module of a package: `::origincloud_pvs::v1beta1`, and
/// `::origincloud_core::longrunning::v1beta1` for the operations package,
/// which the core crate holds.
pub(crate) fn sdk_module(package: &str) -> String {
    let version = package.rsplit('.').next().unwrap_or_default();
    let service = service_of(package);
    if service == OPERATIONS {
        format!("::origincloud_core::{service}::{version}")
    } else {
        format!("::origincloud_{service}::{version}")
    }
}
/// The client type of a service: `VolumeClient` for `VolumeService`.
pub(crate) fn client_name(service: &prost_reflect::ServiceDescriptor) -> String {
    format!(
        "{}Client",
        service
            .name()
            .strip_suffix("Service")
            .unwrap_or(service.name())
    )
}
/// The services of `package`, in file order.
pub(crate) fn services_of(
    pool: &DescriptorPool,
    package: &str,
) -> Vec<prost_reflect::ServiceDescriptor> {
    let mut files: Vec<_> = pool
        .files()
        .filter(|f| f.package_name() == package)
        .collect();
    files.sort_by(|a, b| a.name().cmp(b.name()));
    files.iter().flat_map(|f| f.services()).collect()
}
pub(crate) fn message_extension(
    pool: &DescriptorPool,
    options: &DynamicMessage,
    name: &str,
) -> Option<DynamicMessage> {
    let ext = pool.get_extension_by_name(name)?;
    if !options.has_extension(&ext) {
        return None;
    }
    match options.get_extension(&ext).into_owned() {
        Value::Message(m) => Some(m),
        _ => None,
    }
}
pub(crate) fn behaviors(pool: &DescriptorPool, field: &FieldDescriptor) -> Vec<i32> {
    let Some(ext) = pool.get_extension_by_name("google.api.field_behavior") else {
        return Vec::new();
    };
    let options = field.options();
    if !options.has_extension(&ext) {
        return Vec::new();
    }
    match options.get_extension(&ext).into_owned() {
        Value::List(values) => values.iter().filter_map(Value::as_enum_number).collect(),
        _ => Vec::new(),
    }
}
/// The message an operation's result is, for a method answering with an
/// operation.
pub(crate) fn operation_result(
    pool: &DescriptorPool,
    m: &MethodDescriptor,
) -> Option<MessageDescriptor> {
    let info = message_extension(pool, &m.options(), "google.longrunning.operation_info")?;
    let name = info
        .get_field_by_name("response_type")?
        .as_str()?
        .to_owned();
    let name = if name.contains('.') {
        name
    } else {
        format!("{}.{name}", m.parent_file().package_name())
    };
    pool.get_message_by_name(&name)
}
/// The path of a type relative to its package's module: `Volume`, or
/// `volume::State` for one nested in a message.
pub(crate) fn rust_type_path(package: &str, full_name: &str) -> String {
    let local = full_name
        .strip_prefix(package)
        .and_then(|rest| rest.strip_prefix('.'))
        .unwrap_or(full_name);
    let parts: Vec<&str> = local.split('.').collect();
    let (last, outer) = parts.split_last().expect("a type has a name");
    let mut path: Vec<String> = outer.iter().map(|m| m.to_snake_case()).collect();
    path.push(last.to_upper_camel_case());
    path.join("::")
}
/// `code`, formatted, once it parses.
pub(crate) fn unparse(code: &str) -> Result<String> {
    let file = syn::parse_file(code)
        .map_err(|e| anyhow::anyhow!("the generated code doesn't parse: {e}\n{code}"))?;
    Ok(prettyplease::unparse(&file))
}
