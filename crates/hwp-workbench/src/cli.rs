//! CLI adapter: argument parsing, bounded stdin and shared service calls.
use clap::{Arg, ArgAction, ArgGroup, ArgMatches, Command};
use hwp_core::errors::{require, Result, WorkbenchError};
use hwp_core::workspace::Workspace;
use serde_json::{json, Value};
use std::io::{Read, Write};

const OPERATIONS: [(&str, &str); 14] = [
    (
        "convert",
        "Report an explicit, lossy format export; optionally save a new file.",
    ),
    (
        "session",
        "Read Agent link state and proposal outcomes without writing.",
    ),
    (
        "propose",
        "Write a validated patch for desktop review without changing the document.",
    ),
    (
        "inspect",
        "Inspect native text nodes and compatibility warnings.",
    ),
    ("extract", "Extract document text without node references."),
    (
        "validate",
        "Check bounded native container and parse integrity.",
    ),
    ("plan", "Validate a patch and show changes without writing."),
    (
        "apply",
        "Apply a reviewed patch to a new native document file.",
    ),
    (
        "save",
        "Replace the source after native verification, with a kept previous version.",
    ),
    ("read", "Read paged text with short node references."),
    (
        "search",
        "Find text and return native node IDs and offsets.",
    ),
    (
        "replace",
        "Preview term replacements or save a new document copy.",
    ),
    ("fill", "Preview form values or save a new document copy."),
    (
        "set",
        "Preview one text-node edit or save a new document copy.",
    ),
];
fn option(name: &'static str, help: &'static str) -> Arg {
    Arg::new(name).long(name).help(help)
}
fn flag(name: &'static str, help: &'static str) -> Arg {
    option(name, help).action(ArgAction::SetTrue)
}
fn root(required: bool) -> Arg {
    let arg = option("root", "Allowed workspace (default: current directory).");
    if required {
        arg.required(true)
    } else {
        arg.default_value(".")
    }
}
fn output(required: bool) -> Arg {
    option("output", "New same-format output path; must not exist.")
        .short('o')
        .required(required)
}
fn acknowledgement() -> Arg {
    flag(
        "acknowledge-risks",
        "Acknowledge compatibility warnings before saving.",
    )
}
fn target() -> Arg {
    option("target", "Agent target (default: all).")
        .value_parser(["claude", "codex", "all"])
        .default_value("all")
}
/// Build the public command parser; native worker execution stays hidden.
pub fn parser() -> Command {
    let mut parser = Command::new("hwp-workbench")
        .about("Local HWP/HWPX preservation editor — development alpha.")
        .version("0.1.0-alpha.1")
        .subcommand_required(true)
        .disable_help_subcommand(true)
        .subcommand(
            Command::new("capabilities").about("List supported operations and safety limits."),
        );
    for name in [
        "inspect", "extract", "validate", "plan", "apply", "save", "read", "search", "replace",
        "fill", "set", "propose", "session", "convert",
    ] {
        let help = OPERATIONS
            .iter()
            .find(|(operation, _)| *operation == name)
            .map(|(_, help)| *help)
            .unwrap_or("");
        let mut command = Command::new(name)
            .about(help)
            .arg(
                Arg::new("path")
                    .required(true)
                    .help("Source HWP/HWPX path inside the workspace."),
            )
            .arg(root(false));
        if matches!(name, "plan" | "apply" | "propose" | "save") {
            command = command.arg(
                option("patch", "JSON patch or edit-preview file inside workspace.").required(true),
            );
        }
        if matches!(name, "apply" | "replace" | "fill" | "set") {
            command = command.arg(output(name == "apply")).arg(acknowledgement());
        }
        if name == "save" {
            command = command.arg(acknowledgement()).arg(option("backup-dir", "Backup folder inside the workspace (default: beside the source in .hwp-workbench/backups)."));
        }
        if name == "convert" {
            command = command
                .arg(
                    option("to", "Requested export format.")
                        .required(true)
                        .value_parser(["hwp", "hwpx", "docx"]),
                )
                .arg(output(false))
                .arg(acknowledgement());
        }
        if name == "propose" {
            command = command
                .arg(
                    option("title", "Title shown to the person reviewing the proposal.")
                        .required(true),
                )
                .arg(option("author", "Proposal author (default: agent).").default_value("agent"))
                .arg(option("notes", "Optional review notes."));
        }
        if name == "inspect" {
            command = command.arg(flag(
                "compact",
                "Omit the structure view and package parts.",
            ));
        }
        if matches!(name, "extract" | "read") {
            command = command.arg(
                option("format", "Output format (default: json).")
                    .value_parser(["text", "json"])
                    .default_value("json"),
            );
        }
        if name == "read" {
            command = command
                .arg(option("offset", "First text-node index (default: 0).").default_value("0"))
                .arg(
                    option("limit", "Maximum text nodes (default: 200; maximum: 1000).")
                        .default_value("200"),
                );
        }
        if matches!(name, "search" | "replace") {
            command = command
                .arg(flag("regex", "Use bounded fixed-width regex syntax."))
                .arg(flag(
                    "case-sensitive",
                    "Match case exactly (replace default).",
                ))
                .arg(flag("ignore-case", "Ignore case (search default)."))
                .group(
                    ArgGroup::new("case")
                        .args(["case-sensitive", "ignore-case"])
                        .multiple(false),
                )
                .arg(
                    option("limit", "Maximum matches (maximum: 1000).")
                        .default_value(if name == "search" { "100" } else { "1000" }),
                );
        }
        if name == "search" {
            command = command
                .arg(Arg::new("query_text").help("Text to find; alternative to --query."))
                .arg(option(
                    "query",
                    "Text to find; alternative to positional query.",
                ));
        }
        if name == "replace" {
            command = command
                .arg(option("find", "Term or supported regex to find.").required(true))
                .arg(option("replace-with", "Literal replacement text.").required(true))
                .arg(
                    option(
                        "node-id",
                        "Full ID or short reference; repeat to restrict targets.",
                    )
                    .action(ArgAction::Append),
                );
        }
        if name == "fill" {
            command = command
                .arg(option(
                    "json",
                    "JSON label/value object file inside workspace.",
                ))
                .arg(
                    option("field", "LABEL=VALUE; repeat for each field.")
                        .action(ArgAction::Append),
                )
                .group(
                    ArgGroup::new("fields")
                        .args(["json", "field"])
                        .required(true)
                        .multiple(false),
                );
        }
        if name == "set" {
            command = command
                .arg(option("node-id", "Full node ID or short reference from read.").required(true))
                .arg(option("text", "New plain-text value for the node.").required(true));
        }
        parser = parser.subcommand(command);
    }
    let parser = parser
        .subcommand(
            Command::new("mcp")
                .about("Run the local stdio MCP server.")
                .arg(root(true)),
        )
        .subcommand(
            Command::new("mcp-config")
                .about("Print MCP client setup; does not run clients or edit configuration.")
                .arg(root(true))
                .arg(
                    option("client", "Client setup format to print.")
                        .required(true)
                        .value_parser(["claude-code", "codex", "claude-desktop", "json"]),
                ),
        )
        .subcommand(
            Command::new("open")
                .about("Open a native document in the desktop app for visual review.")
                .arg(
                    Arg::new("path")
                        .required(true)
                        .help("Existing regular .hwp/.hwpx file inside the workspace."),
                )
                .arg(root(false)),
        )
        .subcommand(
            Command::new("rpc")
                .about("Run one desktop JSON request from stdin.")
                .arg(root(true)),
        )
        .subcommand(
            Command::new("skill")
                .about("Install, inspect or remove the Claude Code and Codex skill.")
                .subcommand_required(true)
                .subcommand(
                    Command::new("install")
                        .about("Install the marked skill into selected agent folders.")
                        .arg(target())
                        .arg(flag(
                            "dry-run",
                            "Validate and report destinations without writing.",
                        )),
                )
                .subcommand(
                    Command::new("status")
                        .about("Show installed skill versions and paths as JSON."),
                )
                .subcommand(
                    Command::new("uninstall")
                        .about("Remove only folders marked as installed by this application.")
                        .arg(target()),
                ),
        )
        .subcommand(Command::new("__rhwp-worker").hide(true))
        .subcommand(
            Command::new("__desktop-source-open")
                .hide(true)
                .arg(root(true))
                .arg(
                    option(
                        "path",
                        "Validated document path for the internal desktop child.",
                    )
                    .required(true),
                ),
        );
    apply_help(parser)
}
fn apply_help(mut parser: Command) -> Command {
    // The frozen help snapshot is a checked-in JSON invariant.
    let help: Value = serde_json::from_str(include_str!("cli_help.json"))
        .expect("checked-in CLI help is valid JSON");
    if let Some(text) = help.get("root").and_then(Value::as_str) {
        parser = parser.override_help(text.to_owned());
    }
    for command in parser.get_subcommands_mut() {
        let name = command.get_name().to_owned();
        if let Some(text) = help.get(&name).and_then(Value::as_str) {
            *command = command.clone().override_help(text.to_owned());
        }
        for action in command.get_subcommands_mut() {
            let key = format!("{name} {}", action.get_name());
            if let Some(text) = help.get(&key).and_then(Value::as_str) {
                *action = action.clone().override_help(text.to_owned());
            }
        }
    }
    parser
}
fn string<'a>(args: &'a ArgMatches, name: &str) -> Option<&'a str> {
    args.try_get_one::<String>(name)
        .ok()
        .flatten()
        .map(String::as_str)
}
fn boolean(args: &ArgMatches, name: &str) -> bool {
    args.try_get_one::<bool>(name)
        .ok()
        .flatten()
        .copied()
        .unwrap_or(false)
}
fn json_file(root: &str, path: &str) -> Result<Value> {
    serde_json::from_slice(&Workspace::new(root)?.read(path)?)
        .map_err(|error| WorkbenchError::new("REQUEST_FAILED", crate::mcp::parse_error(&error)))
}
fn integer(args: &ArgMatches, name: &str) -> Result<Value> {
    let value = string(args, name).unwrap_or("0");
    value.parse::<i64>().map(Value::from).map_err(|_| {
        WorkbenchError::new(
            "BAD_ARGUMENTS",
            format!("argument --{name}: invalid int value: '{value}'"),
        )
    })
}
/// Translate CLI options into the same payload accepted by RPC and MCP.
pub fn payload(method: &str, args: &ArgMatches) -> Result<Value> {
    let root = string(args, "root").unwrap_or(".");
    let mut value = json!({"path":string(args,"path").unwrap_or("")});
    let object = value
        .as_object_mut()
        .ok_or_else(|| WorkbenchError::new("BAD_ARGUMENTS", "Invalid arguments."))?;
    if let Some(patch) = string(args, "patch") {
        object.insert("patch".to_owned(), json_file(root, patch)?);
    }
    if let Some(output) = string(args, "output") {
        object.insert("output".to_owned(), json!(output));
        object.insert(
            "acknowledgeRisks".to_owned(),
            json!(boolean(args, "acknowledge-risks")),
        );
    }
    if matches!(method, "save" | "convert") {
        object.insert(
            "acknowledgeRisks".to_owned(),
            json!(boolean(args, "acknowledge-risks")),
        );
    }
    for (option, key) in [
        ("backup-dir", "backupDirectory"),
        ("to", "to"),
        ("title", "title"),
        ("author", "author"),
        ("notes", "notes"),
    ] {
        if let Some(text) = string(args, option) {
            object.insert(key.to_owned(), json!(text));
        }
    }
    if method == "inspect" {
        object.insert("compact".to_owned(), json!(boolean(args, "compact")));
    }
    if method == "read" {
        object.insert("offset".to_owned(), integer(args, "offset")?);
        object.insert("limit".to_owned(), integer(args, "limit")?);
    }
    if matches!(method, "search" | "replace") {
        object.insert("regex".to_owned(), json!(boolean(args, "regex")));
        object.insert(
            "caseSensitive".to_owned(),
            json!(
                boolean(args, "case-sensitive")
                    || method == "replace" && !boolean(args, "ignore-case")
            ),
        );
        object.insert("limit".to_owned(), integer(args, "limit")?);
    }
    if method == "search" {
        let query = string(args, "query");
        let positional = string(args, "query_text");
        require(
            query.is_some() != positional.is_some(),
            "BAD_ARGUMENTS",
            "Supply a positional query or --query, but not both.",
        )?;
        object.insert(
            "query".to_owned(),
            json!(query.or(positional).unwrap_or("")),
        );
    }
    if method == "replace" {
        object.insert("find".to_owned(), json!(string(args, "find").unwrap_or("")));
        object.insert(
            "replaceWith".to_owned(),
            json!(string(args, "replace-with").unwrap_or("")),
        );
        if let Some(ids) = args.get_many::<String>("node-id") {
            object.insert("nodeIds".to_owned(), json!(ids.collect::<Vec<_>>()));
        }
    }
    if method == "fill" {
        let fields = if let Some(path) = string(args, "json") {
            json_file(root, path)?
        } else {
            let mut fields = serde_json::Map::new();
            for entry in args.get_many::<String>("field").into_iter().flatten() {
                let Some((label, text)) = entry.split_once('=') else {
                    return Err(WorkbenchError::new(
                        "BAD_ARGUMENTS",
                        "Each field requires LABEL=VALUE.",
                    ));
                };
                require(
                    !label.trim().is_empty(),
                    "BAD_ARGUMENTS",
                    "Each field requires LABEL=VALUE.",
                )?;
                require(
                    !fields.contains_key(label),
                    "BAD_ARGUMENTS",
                    &format!("Duplicate field label: {label}"),
                )?;
                fields.insert(label.to_owned(), json!(text));
            }
            Value::Object(fields)
        };
        object.insert("fields".to_owned(), fields);
    }
    if method == "set" {
        object.insert(
            "nodeId".to_owned(),
            json!(string(args, "node-id").unwrap_or("")),
        );
        object.insert("text".to_owned(), json!(string(args, "text").unwrap_or("")));
    }
    Ok(value)
}
fn execute(matches: &ArgMatches, input: &mut impl Read, output: &mut impl Write) -> Result<()> {
    let (method, args) = matches.subcommand().ok_or_else(|| {
        WorkbenchError::new(
            "BAD_ARGUMENTS",
            "the following arguments are required: command",
        )
    })?;
    let root = string(args, "root").unwrap_or(".");
    if method == "__desktop-source-open" {
        return crate::desktop_open::source_child_main(root, string(args, "path").unwrap_or(""));
    }
    if method == "mcp" {
        Workspace::new(root)?;
        return crate::mcp::serve(root, std::io::BufReader::new(input), output);
    }
    if method == "mcp-config" {
        writeln!(
            output,
            "{}",
            crate::mcp_config::setup(string(args, "client").unwrap_or("json"), root)?
        )
        .map_err(request_error)?;
        return Ok(());
    }
    let result = match method {
        "capabilities" => crate::service::capabilities(),
        "open" => crate::desktop_open::open_document(root, string(args, "path").unwrap_or(""))?,
        "skill" => {
            let (command, options) = args
                .subcommand()
                .ok_or_else(|| WorkbenchError::new("BAD_ARGUMENTS", "Unknown skill command."))?;
            crate::skill_install::run(
                command,
                string(options, "target").unwrap_or("all"),
                boolean(options, "dry-run"),
            )?
        }
        "rpc" => {
            let stream = crate::mcp::read_bounded_text(&mut std::io::BufReader::new(input), false)?;
            require(!stream.exceeded, "LIMIT", "Request exceeds limit.")?;
            let text = std::str::from_utf8(&stream.bytes).map_err(request_error)?;
            require(
                text.chars().count() <= crate::mcp::MAX_MESSAGE,
                "LIMIT",
                "Request exceeds limit.",
            )?;
            let request: Value = serde_json::from_str(text).map_err(|error| {
                WorkbenchError::new("REQUEST_FAILED", crate::mcp::parse_error(&error))
            })?;
            require(
                request.is_object(),
                "BAD_ARGUMENTS",
                "RPC request must be an object.",
            )?;
            let method = request
                .get("method")
                .ok_or_else(|| WorkbenchError::new("REQUEST_FAILED", "'method'"))?;
            let method = method
                .as_str()
                .ok_or_else(|| WorkbenchError::new("BAD_ARGUMENTS", "method must be a string."))?;
            crate::service::invoke(root, method, request.get("args").unwrap_or(&json!({})))?
        }
        _ => crate::service::invoke(root, method, &payload(method, args)?)?,
    };
    if matches!(method, "extract" | "read") && string(args, "format") == Some("text") {
        writeln!(
            output,
            "{}",
            result.get("text").and_then(Value::as_str).unwrap_or("")
        )
        .map_err(request_error)?;
        if method == "read" {
            writeln!(
                output,
                "sha256={} nextOffset={}",
                result.get("sha256").and_then(Value::as_str).unwrap_or(""),
                result.get("nextOffset").unwrap_or(&Value::Null)
            )
            .map_err(request_error)?;
        }
    } else {
        serde_json::to_writer_pretty(&mut *output, &result).map_err(request_error)?;
        writeln!(output).map_err(request_error)?;
    }
    Ok(())
}
fn request_error(error: impl std::fmt::Display) -> WorkbenchError {
    WorkbenchError::new("REQUEST_FAILED", error.to_string())
}
/// Testable CLI entry point; production uses the process streams.
pub fn run<I, T>(
    argv: I,
    mut input: impl Read,
    mut output: impl Write,
    mut errors: impl Write,
) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let result = match parser().try_get_matches_from(argv) {
        Ok(matches) if matches.subcommand_name() == Some("__rhwp-worker") => {
            return match hwp_verify::worker::run_worker(&mut input, &mut output) {
                Ok(hwp_verify::worker::WorkerStatus::Success) => 0,
                _ => 2,
            };
        }
        Ok(matches) => execute(&matches, &mut input, &mut output),
        Err(error) if error.kind() == clap::error::ErrorKind::DisplayVersion => {
            let version = crate::service::capabilities();
            let _ = writeln!(
                output,
                "{}",
                version
                    .get("version")
                    .and_then(Value::as_str)
                    .unwrap_or("0.1.0-alpha.1")
            );
            return 0;
        }
        Err(error) if error.kind() == clap::error::ErrorKind::DisplayHelp => {
            let _ = write!(output, "{error}");
            return 0;
        }
        Err(error) => Err(WorkbenchError::new("BAD_ARGUMENTS", error.to_string())),
    };
    match result {
        Ok(()) => 0,
        Err(error) => {
            let _ = writeln!(
                errors,
                "{}",
                crate::mcp::python_json(&json!({"error":error.as_json()}))
            );
            2
        }
    }
}
/// Run the native CLI.
pub fn main() -> i32 {
    run(
        std::env::args_os(),
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        std::io::stderr().lock(),
    )
}
