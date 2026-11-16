//! Compiles `data/presets.json` (ECMA-376's preset shapes: adjustment defaults, guides and paths) into static Rust
//! data (`$OUT_DIR/presets.rs`). Every formula is parsed here and every name resolved to a value slot, so the
//! table holds no strings to parse at run time and a malformed preset fails the build.
//!
//! Slots: the built-in guides first ([`builtins::BUILTIN_NAMES`]), then the preset's `av` guides, then its `gd`
//! guides. A name refers to its latest definition before the formula that reads it, as an ordered map would.

#[allow(dead_code, reason = "shared with the library")]
#[path = "src/builtins.rs"]
mod builtins;
#[allow(dead_code, reason = "shared with the library")]
#[path = "src/literal.rs"]
mod literal;
#[allow(dead_code, reason = "shared with the library")]
#[path = "src/op.rs"]
mod op;

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Write as _;
use std::path::PathBuf;

use serde_json::{Map, Value};

use op::Op;

type BuildResult<T> = Result<T, Box<dyn Error>>;

/// Names in scope, each at its latest slot.
struct Scope(BTreeMap<String, u16>);

impl Scope {
    fn new() -> Scope {
        Scope(
            builtins::BUILTIN_NAMES
                .iter()
                .zip(0u16..)
                .map(|(name, slot)| ((*name).to_owned(), slot))
                .collect(),
        )
    }

    /// A token as an `Operand` expression.
    fn operand(&self, preset: &str, token: &str) -> BuildResult<String> {
        if let Some(slot) = self.0.get(token) {
            return Ok(format!("Operand::Slot({slot})"));
        }
        match literal::parse_literal(token) {
            Some(value) => Ok(format!("Operand::Lit({value:?})")),
            None => Err(format!("{preset}: unknown guide {token:?}").into()),
        }
    }
}

fn strings<'a>(value: &'a Value, what: &str) -> BuildResult<Vec<&'a str>> {
    value
        .as_array()
        .ok_or_else(|| format!("{what}: not an array"))?
        .iter()
        .map(|item| {
            item.as_str()
                .ok_or_else(|| format!("{what}: not a string").into())
        })
        .collect()
}

/// The `GuideDef`s of an `av` or `gd` list, defining each name in `scope` after its formula.
fn guides(
    out: &mut String,
    preset: &str,
    list: Option<&Value>,
    scope: &mut Scope,
    next: &mut u16,
) -> BuildResult<()> {
    out.push_str("&[");
    for guide in list.and_then(Value::as_array).into_iter().flatten() {
        let pair = strings(guide, preset)?;
        let [name, formula] = pair.as_slice() else {
            return Err(format!("{preset}: a guide is not [name, formula]").into());
        };
        let mut tokens = formula.split_whitespace();
        let op_name = tokens.next().unwrap_or("");
        let op =
            Op::parse(op_name).ok_or_else(|| format!("{preset}: unknown operator {op_name:?}"))?;
        let args: Vec<&str> = tokens.take(op.arity()).collect();
        if args.len() < op.arity() {
            return Err(format!("{preset}: {formula:?} lacks arguments").into());
        }
        let mut operands = Vec::new();
        for token in &args {
            operands.push(scope.operand(preset, token)?);
        }
        while operands.len() < 3 {
            operands.push("Operand::Lit(0.0)".to_owned());
        }
        write!(
            out,
            "GuideDef {{ name: {name:?}, op: Op::{op:?}, args: [{}] }},",
            operands.join(", ")
        )?;
        scope.0.insert((*name).to_owned(), *next);
        *next = next.checked_add(1).ok_or("too many guides")?;
    }
    out.push(']');
    Ok(())
}

fn command(out: &mut String, preset: &str, command: &Value, scope: &Scope) -> BuildResult<()> {
    let tokens = strings(command, preset)?;
    let (name, args) = tokens
        .split_first()
        .ok_or_else(|| format!("{preset}: empty command"))?;
    let (variant, arity) = match *name {
        "M" => ("M", 2),
        "L" => ("L", 2),
        "A" => ("A", 4),
        "Q" => ("Q", 4),
        "C" => ("C", 6),
        "Z" => ("Z", 0),
        other => return Err(format!("{preset}: unknown command {other:?}").into()),
    };
    if args.len() != arity {
        return Err(format!("{preset}: {name} takes {arity} arguments").into());
    }
    if arity == 0 {
        write!(out, "Command::{variant},")?;
        return Ok(());
    }
    let mut operands = Vec::new();
    for token in args {
        operands.push(scope.operand(preset, token)?);
    }
    write!(out, "Command::{variant}({}),", operands.join(", "))?;
    Ok(())
}

fn dimension(path: &Map<String, Value>, key: &str, preset: &str) -> BuildResult<String> {
    match path.get(key) {
        None => Ok("None".to_owned()),
        Some(value) => {
            let value = value
                .as_f64()
                .ok_or_else(|| format!("{preset}: path {key} is not a number"))?;
            Ok(format!("Some({value:?})"))
        }
    }
}

fn path(out: &mut String, preset: &str, path: &Value, scope: &Scope) -> BuildResult<()> {
    let path = path
        .as_object()
        .ok_or_else(|| format!("{preset}: a path is not an object"))?;
    if let Some(key) = path
        .keys()
        .find(|key| !["w", "h", "fill", "stroke", "d"].contains(&key.as_str()))
    {
        return Err(format!("{preset}: unknown path key {key:?}").into());
    }
    let fill = match path.get("fill").map(|fill| fill.as_str()) {
        None => "Norm",
        Some(Some("none")) => "None",
        Some(Some("lighten")) => "Lighten",
        Some(Some("lightenLess")) => "LightenLess",
        Some(Some("darken")) => "Darken",
        Some(Some("darkenLess")) => "DarkenLess",
        Some(other) => return Err(format!("{preset}: unknown fill {other:?}").into()),
    };
    let stroke = match path.get("stroke") {
        None => true,
        Some(Value::Bool(false)) => false,
        Some(other) => return Err(format!("{preset}: unknown stroke {other}").into()),
    };
    write!(
        out,
        "PathDef {{ w: {}, h: {}, fill: PathFill::{fill}, stroke: {stroke}, commands: &[",
        dimension(path, "w", preset)?,
        dimension(path, "h", preset)?,
    )?;
    let commands = path
        .get("d")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{preset}: a path has no commands"))?;
    for c in commands {
        command(out, preset, c, scope)?;
    }
    out.push_str("] },");
    Ok(())
}

fn preset(out: &mut String, name: &str, definition: &Value) -> BuildResult<()> {
    let definition = definition
        .as_object()
        .ok_or_else(|| format!("{name}: not an object"))?;
    if let Some(key) = definition
        .keys()
        .find(|key| !["av", "gd", "paths"].contains(&key.as_str()))
    {
        return Err(format!("{name}: unknown key {key:?}").into());
    }
    let mut scope = Scope::new();
    let mut next = u16::try_from(builtins::BUILTIN_NAMES.len())?;
    write!(out, "PresetDef {{ av: ")?;
    guides(out, name, definition.get("av"), &mut scope, &mut next)?;
    out.push_str(", gd: ");
    guides(out, name, definition.get("gd"), &mut scope, &mut next)?;
    write!(out, ", slots: {next}, paths: &[")?;
    let paths = definition
        .get("paths")
        .and_then(Value::as_array)
        .filter(|paths| !paths.is_empty())
        .ok_or_else(|| format!("{name}: no paths"))?;
    for p in paths {
        path(out, name, p, &scope)?;
    }
    out.push_str("] },\n");
    Ok(())
}

fn main() -> BuildResult<()> {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let source = manifest.join("data/presets.json");
    println!("cargo::rerun-if-changed={}", source.display());
    for file in ["src/builtins.rs", "src/literal.rs", "src/op.rs"] {
        println!("cargo::rerun-if-changed={}", manifest.join(file).display());
    }
    let table: Map<String, Value> = serde_json::from_str(&std::fs::read_to_string(&source)?)?;
    let mut names: Vec<&String> = table.keys().collect();
    names.sort();

    let mut out = String::new();
    writeln!(
        out,
        "/// The preset names, sorted by code unit; `PRESETS` holds their definitions in the same order."
    )?;
    writeln!(out, "pub(crate) static NAMES: [&str; {}] = [", names.len())?;
    for name in &names {
        writeln!(out, "    {name:?},")?;
    }
    writeln!(out, "];\n")?;
    writeln!(
        out,
        "pub(crate) static PRESETS: [PresetDef; {}] = [",
        names.len()
    )?;
    for name in &names {
        let definition = table.get(*name).ok_or("missing preset")?;
        preset(&mut out, name, definition)?;
    }
    writeln!(out, "];")?;
    let target = PathBuf::from(std::env::var("OUT_DIR")?).join("presets.rs");
    std::fs::write(target, out)?;
    Ok(())
}
