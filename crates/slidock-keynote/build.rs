//! Compiles `proto/keynote.proto` with `protox` (pure Rust, no `protoc`) and generates, into `OUT_DIR`:
//! - `_.rs`, the `prost` message types;
//! - `schema_tables.rs`, every message's fields (number, name, kind, repeated) for the wire decoder in `iwa::schema`;
//! - `archives.rs`, the `Archive` enum (one variant per registered message the schema keeps), its decoders and the
//!   type registry from `proto/registry.tsv`.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Write as _;
use std::path::PathBuf;

use prost_types::field_descriptor_proto::{Label, Type};
use prost_types::{DescriptorProto, FileDescriptorSet};

type BoxError = Box<dyn Error>;

/// One message of the schema, by full name (`KN.SlideArchive`).
struct Message {
    fields: Vec<Field>,
}

struct Field {
    number: i32,
    name: String,
    kind: String,
    repeated: bool,
}

fn main() -> Result<(), BoxError> {
    println!("cargo:rerun-if-changed=proto/keynote.proto");
    println!("cargo:rerun-if-changed=proto/registry.tsv");
    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    let fds = protox::compile(["keynote.proto"], ["proto"])?;
    prost_build::Config::new()
        .out_dir(&out)
        .compile_fds(fds.clone())?;

    let messages = messages(&fds)?;
    let index: BTreeMap<&str, usize> = messages
        .keys()
        .enumerate()
        .map(|(i, name)| (name.as_str(), i))
        .collect();
    std::fs::write(
        out.join("schema_tables.rs"),
        schema_tables(&messages, &index)?,
    )?;
    let registry = std::fs::read_to_string("proto/registry.tsv")?;
    std::fs::write(out.join("archives.rs"), archives(&registry, &index)?)?;
    Ok(())
}

/// Every message nested in a namespace wrapper, sorted by full name.
fn messages(fds: &FileDescriptorSet) -> Result<BTreeMap<String, Message>, BoxError> {
    fn visit(
        prefix: &str,
        message: &DescriptorProto,
        out: &mut BTreeMap<String, Message>,
    ) -> Result<(), BoxError> {
        let name = format!("{prefix}.{}", message.name());
        let mut fields = Vec::new();
        for field in &message.field {
            let kind = match field.r#type() {
                Type::Message => {
                    let target = field.type_name().trim_start_matches('.');
                    format!("Message(\"{target}\")")
                }
                Type::Enum => "Enum".into(),
                Type::Bool => "Bool".into(),
                Type::Uint32 => "Uint32".into(),
                Type::Int32 => "Int32".into(),
                Type::Uint64 => "Uint64".into(),
                Type::Fixed32 => "Fixed32".into(),
                Type::Float => "Float".into(),
                Type::Double => "Double".into(),
                Type::String => "String".into(),
                Type::Bytes => "Bytes".into(),
                other => return Err(format!("{name}.{}: a {other:?} field", field.name()).into()),
            };
            fields.push(Field {
                number: field.number(),
                name: field.name().to_owned(),
                kind,
                repeated: field.label() == Label::Repeated,
            });
        }
        fields.sort_by_key(|field| field.number);
        for nested in &message.nested_type {
            visit(&name, nested, out)?;
        }
        out.insert(name, Message { fields });
        Ok(())
    }
    let mut out = BTreeMap::new();
    for file in &fds.file {
        for namespace in &file.message_type {
            if !namespace.field.is_empty() {
                return Err(format!("the namespace {} has fields", namespace.name()).into());
            }
            for message in &namespace.nested_type {
                visit(namespace.name(), message, &mut out)?;
            }
        }
    }
    Ok(out)
}

fn schema_tables(
    messages: &BTreeMap<String, Message>,
    index: &BTreeMap<&str, usize>,
) -> Result<String, BoxError> {
    let mut out = String::new();
    writeln!(
        out,
        "/// Every message of the trimmed schema, sorted by full name."
    )?;
    writeln!(out, "pub(crate) static MESSAGES: &[MessageSchema] = &[")?;
    let mut costs = BTreeMap::new();
    for (name, message) in messages {
        let cost = cost(name, messages, &mut costs, 0)?;
        writeln!(
            out,
            "    MessageSchema {{ name: \"{name}\", cost: {cost}, fields: &["
        )?;
        for field in &message.fields {
            let kind = match field.kind.strip_prefix("Message(\"") {
                Some(rest) => {
                    let target = rest.trim_end_matches("\")");
                    let at = index
                        .get(target)
                        .ok_or_else(|| format!("{name}.{} names {target}", field.name))?;
                    format!("Message({at})")
                }
                None => field.kind.clone(),
            };
            writeln!(
                out,
                "        FieldSchema {{ number: {}, name: \"{}\", kind: FieldKind::{kind}, repeated: {} }},",
                field.number, field.name, field.repeated
            )?;
        }
        writeln!(out, "    ] }},")?;
    }
    writeln!(out, "];")?;
    let archive_info = index
        .get("TSP.ArchiveInfo")
        .ok_or("the schema lacks TSP.ArchiveInfo")?;
    writeln!(out, "/// `TSP.ArchiveInfo`, every object's header.")?;
    writeln!(
        out,
        "pub(crate) const ARCHIVE_INFO: usize = {archive_info};"
    )?;
    Ok(out)
}

/// What a decoded message costs in memory, the same on every target: 24 bytes, plus 16 per scalar field, 24 per
/// string, bytes or repeated field, and an inline (singular) message field's own cost.
fn cost(
    name: &str,
    messages: &BTreeMap<String, Message>,
    costs: &mut BTreeMap<String, u64>,
    depth: usize,
) -> Result<u64, BoxError> {
    if let Some(&cost) = costs.get(name) {
        return Ok(cost);
    }
    if depth > 64 {
        return Err(format!("{name} holds itself inline").into());
    }
    let message = messages
        .get(name)
        .ok_or_else(|| format!("no message {name}"))?;
    let mut total = 24;
    for field in &message.fields {
        total += match field.kind.strip_prefix("Message(\"") {
            _ if field.repeated => 24,
            Some(rest) => cost(rest.trim_end_matches("\")"), messages, costs, depth + 1)?,
            None if field.kind == "String" || field.kind == "Bytes" => 24,
            None => 16,
        };
    }
    costs.insert(name.to_owned(), total);
    Ok(total)
}

/// The registry's rows, and for each message the schema keeps, its `Archive` variant and decoder.
fn archives(registry: &str, index: &BTreeMap<&str, usize>) -> Result<String, BoxError> {
    let mut rows = Vec::new();
    for line in registry.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let (id, name) = line
            .split_once('\t')
            .ok_or_else(|| format!("registry line {line:?}"))?;
        rows.push((id.parse::<u32>()?, name.to_owned()));
    }
    rows.sort();
    let kept: BTreeMap<&str, (String, String)> = rows
        .iter()
        .filter(|(_, name)| index.contains_key(name.as_str()))
        .map(|(_, name)| Ok((name.as_str(), rust_names(name)?)))
        .collect::<Result<_, BoxError>>()?;

    let mut out = String::new();
    writeln!(
        out,
        "/// A decoded object: one variant per registered message the trimmed schema keeps."
    )?;
    writeln!(out, "#[derive(Clone, Debug, PartialEq)]")?;
    writeln!(out, "pub enum Archive {{")?;
    for (name, (variant, path)) in &kept {
        writeln!(out, "    /// `{name}`")?;
        writeln!(out, "    {variant}(Box<{path}>),")?;
    }
    writeln!(out, "}}")?;

    writeln!(out, "impl Archive {{")?;
    writeln!(out, "    /// The registered message name.")?;
    writeln!(out, "    pub fn type_name(&self) -> &'static str {{")?;
    writeln!(out, "        match self {{")?;
    for (name, (variant, _)) in &kept {
        writeln!(out, "            Archive::{variant}(_) => \"{name}\",")?;
    }
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(
        out,
        "    /// The message's wire bytes, as `prost` writes them."
    )?;
    writeln!(out, "    pub fn encode_to_vec(&self) -> Vec<u8> {{")?;
    writeln!(out, "        match self {{")?;
    for (variant, _) in kept.values() {
        writeln!(
            out,
            "            Archive::{variant}(message) => ::prost::Message::encode_to_vec(message.as_ref()),"
        )?;
    }
    writeln!(out, "        }}")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;

    for (name, (variant, path)) in &kept {
        let schema = index
            .get(name)
            .ok_or_else(|| format!("{name} is not in the schema"))?;
        writeln!(out, "impl FromArchive for {path} {{")?;
        writeln!(out, "    const TYPE_NAME: &'static str = \"{name}\";")?;
        writeln!(
            out,
            "    fn from_archive(archive: &Archive) -> Option<&Self> {{"
        )?;
        writeln!(out, "        match archive {{")?;
        writeln!(
            out,
            "            Archive::{variant}(message) => Some(message.as_ref()),"
        )?;
        if kept.len() > 1 {
            writeln!(out, "            _ => None,")?;
        }
        writeln!(out, "        }}")?;
        writeln!(out, "    }}")?;
        writeln!(out, "}}")?;
        writeln!(
            out,
            "fn decode_{}(bytes: &[u8]) -> Result<Archive, ::prost::DecodeError> {{",
            snake(variant)
        )?;
        writeln!(
            out,
            "    <{path} as ::prost::Message>::decode(bytes).map(|message| Archive::{variant}(Box::new(message)))"
        )?;
        writeln!(out, "}}")?;
        writeln!(
            out,
            "const KEPT_{}: Kept = Kept {{ schema: {schema}, decode: decode_{} }};",
            snake(variant).to_uppercase(),
            snake(variant)
        )?;
    }

    writeln!(
        out,
        "/// The whole registry, sorted by type id: names for every type, decoders for the kept ones."
    )?;
    writeln!(out, "pub(crate) static REGISTRY: &[Registered] = &[")?;
    for (id, name) in &rows {
        let kept = match kept.get(name.as_str()) {
            Some((variant, _)) => format!("Some(KEPT_{})", snake(variant).to_uppercase()),
            None => "None".into(),
        };
        writeln!(
            out,
            "    Registered {{ type_id: {id}, name: \"{name}\", kept: {kept} }},"
        )?;
    }
    writeln!(out, "];")?;
    Ok(out)
}

/// The `Archive` variant and the `prost` type path of a registered message (`KN.SlideArchive` →
/// `KnSlideArchive`, `crate::proto::kn::SlideArchive`).
fn rust_names(name: &str) -> Result<(String, String), BoxError> {
    let (namespace, message) = name
        .split_once('.')
        .ok_or_else(|| format!("{name} has no namespace"))?;
    if message.contains('.') {
        return Err(format!("{name} is a nested message").into());
    }
    let variant = format!("{}{}", upper_camel(namespace), upper_camel(message));
    let path = format!(
        "crate::proto::{}::{}",
        snake(namespace),
        upper_camel(message)
    );
    Ok((variant, path))
}

/// The words of an identifier, split as `heck` (and so `prost-build`) splits them: at a lower-to-upper step, and
/// before the last capital of a run followed by a lowercase letter.
fn words(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut words = Vec::new();
    let mut word = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            continue;
        }
        let previous = i.checked_sub(1).and_then(|p| chars.get(p)).copied();
        let next = chars.get(i + 1).copied();
        let boundary = c.is_uppercase()
            && previous.is_some_and(|p| {
                p.is_lowercase()
                    || p.is_ascii_digit()
                    || (p.is_uppercase() && next.is_some_and(char::is_lowercase))
            });
        if boundary && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        word.push(c);
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

fn upper_camel(name: &str) -> String {
    words(name)
        .iter()
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first
                    .to_uppercase()
                    .chain(chars.flat_map(char::to_lowercase))
                    .collect()
            })
        })
        .collect()
}

fn snake(name: &str) -> String {
    words(name)
        .iter()
        .map(|word| word.to_lowercase())
        .collect::<Vec<_>>()
        .join("_")
}
