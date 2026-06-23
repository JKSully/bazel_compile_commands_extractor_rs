use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExcludeHeaders {
    None,
    All,
    External,
}

impl ExcludeHeaders {
    pub fn parse(value: &str) -> Result<Self, ExtractorError> {
        match value {
            "" | "none" => Ok(Self::None),
            "all" => Ok(Self::All),
            "external" => Ok(Self::External),
            other => Err(ExtractorError::InvalidArgument(format!(
                "unsupported --exclude_headers value {other:?}; expected one of: none, all, external"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetSpec {
    pub target: String,
    pub flags: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractorConfig {
    pub targets: Vec<TargetSpec>,
    pub exclude_headers: ExcludeHeaders,
    pub exclude_external_sources: bool,
    pub runtime_bazel_flags: Vec<String>,
}

impl Default for ExtractorConfig {
    fn default() -> Self {
        Self {
            targets: vec![TargetSpec {
                target: "@//...".to_owned(),
                flags: String::new(),
            }],
            exclude_headers: ExcludeHeaders::None,
            exclude_external_sources: false,
            runtime_bazel_flags: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub enum ExtractorError {
    InvalidArgument(String),
    MissingEnvironment(&'static str),
    Io {
        context: String,
        source: io::Error,
    },
    Json {
        context: String,
        source: serde_json::Error,
    },
    BazelAqueryFailed {
        command: Vec<String>,
        stderr: String,
    },
    NoCompileCommands,
}

impl fmt::Display for ExtractorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArgument(message) => write!(formatter, "{message}"),
            Self::MissingEnvironment(name) => write!(
                formatter,
                "{name} was not found in the environment; invoke this tool with `bazel run`"
            ),
            Self::Io { context, source } => write!(formatter, "{context}: {source}"),
            Self::Json { context, source } => write!(formatter, "{context}: {source}"),
            Self::BazelAqueryFailed { command, stderr } => write!(
                formatter,
                "bazel aquery failed for command `{}`: {stderr}",
                command.join(" ")
            ),
            Self::NoCompileCommands => write!(
                formatter,
                "not writing compile_commands.json because no C-family compile commands were extracted"
            ),
        }
    }
}

impl std::error::Error for ExtractorError {}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AqueryOutput {
    #[serde(default)]
    pub actions: Vec<Action>,
    #[serde(default)]
    pub targets: Vec<Target>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    #[serde(default)]
    pub target_id: Option<u32>,
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default)]
    pub environment_variables: Vec<EnvironmentVariable>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub id: u32,
    pub label: String,
}

#[derive(Debug, Deserialize)]
pub struct EnvironmentVariable {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompileCommand {
    pub file: String,
    pub arguments: Vec<String>,
    pub directory: String,
}

pub fn parse_args(
    arguments: impl IntoIterator<Item = String>,
) -> Result<ExtractorConfig, ExtractorError> {
    let mut config = ExtractorConfig::default();
    let mut explicit_targets = Vec::new();
    let mut iterator = arguments.into_iter();

    while let Some(argument) = iterator.next() {
        match argument.as_str() {
            "--target" => {
                let value = iterator.next().ok_or_else(|| {
                    ExtractorError::InvalidArgument("--target requires TARGET=FLAGS".to_owned())
                })?;
                explicit_targets.push(parse_target_spec(&value));
            }
            "--exclude_headers" => {
                let value = iterator.next().ok_or_else(|| {
                    ExtractorError::InvalidArgument("--exclude_headers requires a value".to_owned())
                })?;
                config.exclude_headers = ExcludeHeaders::parse(&value)?;
            }
            "--exclude_external_sources" => config.exclude_external_sources = true,
            "--" => {
                config.runtime_bazel_flags.extend(iterator);
                break;
            }
            other => config.runtime_bazel_flags.push(other.to_owned()),
        }
    }

    if !explicit_targets.is_empty() {
        config.targets = explicit_targets;
    }

    Ok(config)
}

pub fn parse_target_spec(value: &str) -> TargetSpec {
    if let Some((target, flags)) = value.split_once('=') {
        TargetSpec {
            target: target.to_owned(),
            flags: flags.to_owned(),
        }
    } else {
        TargetSpec {
            target: value.to_owned(),
            flags: String::new(),
        }
    }
}

pub fn run(config: &ExtractorConfig) -> Result<(), ExtractorError> {
    let workspace = workspace_root()?;
    env::set_current_dir(&workspace).map_err(|source| ExtractorError::Io {
        context: format!("failed to change directory to {}", workspace.display()),
        source,
    })?;
    ensure_gitignore_entries_exist(&workspace)?;
    ensure_external_workspaces_link_exists(&workspace)?;

    let mut entries = Vec::new();
    for target in &config.targets {
        entries.extend(get_commands(&workspace, config, target)?);
    }

    if entries.is_empty() {
        return Err(ExtractorError::NoCompileCommands);
    }

    write_compile_commands(&workspace, &entries)
}

pub fn convert_compile_commands(
    aquery_output: &AqueryOutput,
    workspace: &Path,
    exclude_headers: &ExcludeHeaders,
    exclude_external_sources: bool,
) -> Vec<CompileCommand> {
    let labels_by_target_id = aquery_output
        .targets
        .iter()
        .map(|target| (target.id, target.label.as_str()))
        .collect::<BTreeMap<_, _>>();
    let directory = workspace.to_string_lossy().into_owned();
    let mut commands = Vec::new();
    let mut headers_written = BTreeSet::new();

    for action in &aquery_output.actions {
        if exclude_external_sources && action_is_external(action, &labels_by_target_id) {
            continue;
        }

        let files = get_files(
            action,
            exclude_headers,
            action_is_external(action, &labels_by_target_id),
        );
        for file in files.sources {
            if should_emit_file(&file) {
                commands.push(CompileCommand {
                    file,
                    arguments: patch_arguments(&action.arguments),
                    directory: directory.clone(),
                });
            }
        }

        for header in files.headers {
            if headers_written.insert(header.clone()) && should_emit_file(&header) {
                commands.push(CompileCommand {
                    file: header,
                    arguments: patch_arguments(&action.arguments),
                    directory: directory.clone(),
                });
            }
        }
    }

    commands
}

fn get_commands(
    workspace: &Path,
    config: &ExtractorConfig,
    target_spec: &TargetSpec,
) -> Result<Vec<CompileCommand>, ExtractorError> {
    eprintln!(">>> Analyzing commands used in {}", target_spec.target);
    let mut command = build_aquery_command(config, target_spec);
    let output = Command::new(&command[0])
        .args(&command[1..])
        .output()
        .map_err(|source| ExtractorError::Io {
            context: "failed to execute bazel aquery".to_owned(),
            source,
        })?;

    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(ExtractorError::BazelAqueryFailed { command, stderr });
    }

    let aquery_output =
        serde_json::from_slice::<AqueryOutput>(&output.stdout).map_err(|source| {
            ExtractorError::Json {
                context: "failed to parse bazel aquery jsonproto output".to_owned(),
                source,
            }
        })?;
    command.clear();

    Ok(convert_compile_commands(
        &aquery_output,
        workspace,
        &config.exclude_headers,
        config.exclude_external_sources,
    ))
}

fn build_aquery_command(config: &ExtractorConfig, target_spec: &TargetSpec) -> Vec<String> {
    let mut command = vec![
        "bazel".to_owned(),
        "aquery".to_owned(),
        format!(
            "mnemonic('(Objc|Cpp|Cuda)Compile',{})",
            target_statement(&target_spec.target, config.exclude_external_sources)
        ),
        "--output=jsonproto".to_owned(),
        "--include_artifacts=false".to_owned(),
        "--ui_event_filters=-info".to_owned(),
        "--noshow_progress".to_owned(),
        "--features=-compiler_param_file".to_owned(),
        "--features=-layering_check".to_owned(),
        "--host_features=-compiler_param_file".to_owned(),
        "--host_features=-layering_check".to_owned(),
    ];
    command.extend(split_flags(&target_spec.flags));
    command.extend(config.runtime_bazel_flags.clone());
    command
}

fn target_statement(target: &str, exclude_external_sources: bool) -> String {
    let deps = format!("deps({target})");
    if exclude_external_sources {
        format!("filter('^(//|@//)',{deps})")
    } else {
        deps
    }
}

fn split_flags(flags: &str) -> Vec<String> {
    flags
        .split_whitespace()
        .filter(|flag| !flag.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[derive(Debug, Default)]
struct Files {
    sources: Vec<String>,
    headers: Vec<String>,
}

fn get_files(action: &Action, exclude_headers: &ExcludeHeaders, external_action: bool) -> Files {
    let mut files = Files::default();
    let mut skip_next = false;

    for argument in &action.arguments {
        if skip_next {
            skip_next = false;
            continue;
        }

        if argument == "-o" || argument == "-MF" || argument == "-MT" || argument == "-MQ" {
            skip_next = true;
            continue;
        }

        let Some(path) = plausible_file_argument(argument) else {
            continue;
        };

        if is_source_file(path) {
            files.sources.push(path.to_owned());
        } else if should_include_header(path, exclude_headers, external_action) {
            files.headers.push(path.to_owned());
        }
    }

    files.sources.sort();
    files.sources.dedup();
    files.headers.sort();
    files.headers.dedup();
    files
}

fn plausible_file_argument(argument: &str) -> Option<&str> {
    if argument.starts_with('-') || argument.starts_with('@') {
        return None;
    }

    Path::new(argument).file_name().and_then(OsStr::to_str)?;
    Some(argument)
}

fn should_include_header(
    path: &str,
    exclude_headers: &ExcludeHeaders,
    external_action: bool,
) -> bool {
    if !is_header_file(path) {
        return false;
    }

    match exclude_headers {
        ExcludeHeaders::None => true,
        ExcludeHeaders::All => false,
        ExcludeHeaders::External => !external_action && !is_external_path(path),
    }
}

fn action_is_external(action: &Action, labels_by_target_id: &BTreeMap<u32, &str>) -> bool {
    action
        .target_id
        .and_then(|target_id| labels_by_target_id.get(&target_id))
        .is_some_and(|label| label.starts_with('@') && !label.starts_with("@//"))
}

fn patch_arguments(arguments: &[String]) -> Vec<String> {
    arguments
        .iter()
        .filter(|argument| argument.as_str() != "-fno-canonical-system-headers")
        .cloned()
        .collect()
}

fn should_emit_file(file: &str) -> bool {
    file != "external/bazel_tools/src/tools/launcher/dummy.cc"
}

fn is_source_file(path: &str) -> bool {
    matches!(
        extension(path),
        Some("c" | "cc" | "cpp" | "cxx" | "c++" | "m" | "mm" | "cu")
    )
}

fn is_header_file(path: &str) -> bool {
    matches!(
        extension(path),
        Some("h" | "hh" | "hpp" | "hxx" | "h++" | "inc" | "inl" | "ipp" | "cuh")
    )
}

fn extension(path: &str) -> Option<&str> {
    Path::new(path)
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .as_deref()
        .map(|extension| {
            // Keep the allocation inside this function while returning a static-like matchable value.
            match extension {
                "c" => "c",
                "cc" => "cc",
                "cpp" => "cpp",
                "cxx" => "cxx",
                "c++" => "c++",
                "m" => "m",
                "mm" => "mm",
                "cu" => "cu",
                "h" => "h",
                "hh" => "hh",
                "hpp" => "hpp",
                "hxx" => "hxx",
                "h++" => "h++",
                "inc" => "inc",
                "inl" => "inl",
                "ipp" => "ipp",
                "cuh" => "cuh",
                _ => "",
            }
        })
        .filter(|extension| !extension.is_empty())
}

fn is_external_path(path: &str) -> bool {
    path.starts_with("external/") || path.starts_with("../")
}

fn workspace_root() -> Result<PathBuf, ExtractorError> {
    env::var_os("BUILD_WORKSPACE_DIRECTORY")
        .map(PathBuf::from)
        .ok_or(ExtractorError::MissingEnvironment(
            "BUILD_WORKSPACE_DIRECTORY",
        ))
}

fn write_compile_commands(
    workspace: &Path,
    entries: &[CompileCommand],
) -> Result<(), ExtractorError> {
    let output = workspace.join("compile_commands.json");
    let json = serde_json::to_string_pretty(entries).map_err(|source| ExtractorError::Json {
        context: "failed to serialize compile_commands.json".to_owned(),
        source,
    })?;
    fs::write(&output, format!("{json}\n")).map_err(|source| ExtractorError::Io {
        context: format!("failed to write {}", output.display()),
        source,
    })
}

fn ensure_gitignore_entries_exist(workspace: &Path) -> Result<(), ExtractorError> {
    let gitignore = workspace.join(".gitignore");
    let entries = ["/compile_commands.json", "/external"];
    let existing = match fs::read_to_string(&gitignore) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(source) => {
            return Err(ExtractorError::Io {
                context: format!("failed to read {}", gitignore.display()),
                source,
            });
        }
    };

    let mut additions = Vec::new();
    for entry in entries {
        if !existing.lines().any(|line| line.trim() == entry) {
            additions.push(entry);
        }
    }

    if additions.is_empty() {
        return Ok(());
    }

    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    for addition in additions {
        updated.push_str(addition);
        updated.push('\n');
    }

    fs::write(&gitignore, updated).map_err(|source| ExtractorError::Io {
        context: format!("failed to update {}", gitignore.display()),
        source,
    })
}

fn ensure_external_workspaces_link_exists(workspace: &Path) -> Result<(), ExtractorError> {
    let external = workspace.join("external");
    if external.exists() {
        return Ok(());
    }

    let execution_root = Command::new("bazel")
        .args(["info", "execution_root"])
        .output()
        .map_err(|source| ExtractorError::Io {
            context: "failed to execute `bazel info execution_root`".to_owned(),
            source,
        })?;

    if !execution_root.status.success() {
        return Ok(());
    }

    let execution_root = String::from_utf8_lossy(&execution_root.stdout);
    let external_source = Path::new(execution_root.trim()).join("external");
    symlink_directory(&external_source, &external)
}

#[cfg(unix)]
fn symlink_directory(source: &Path, destination: &Path) -> Result<(), ExtractorError> {
    std::os::unix::fs::symlink(source, destination).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            ExtractorError::Io {
                context: format!("{} already exists", destination.display()),
                source: error,
            }
        } else {
            ExtractorError::Io {
                context: format!(
                    "failed to symlink {} to {}",
                    destination.display(),
                    source.display()
                ),
                source: error,
            }
        }
    })
}

#[cfg(windows)]
fn symlink_directory(source: &Path, destination: &Path) -> Result<(), ExtractorError> {
    std::os::windows::fs::symlink_dir(source, destination).map_err(|source| ExtractorError::Io {
        context: format!(
            "failed to symlink {} to {}",
            destination.display(),
            source.display()
        ),
        source,
    })
}
