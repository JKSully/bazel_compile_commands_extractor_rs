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
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub dep_set_of_files: Vec<DepSetOfFiles>,
    #[serde(default)]
    pub path_fragments: Vec<PathFragment>,
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
    #[serde(default)]
    pub input_dep_set_ids: Vec<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub id: u32,
    pub label: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub id: u32,
    pub path_fragment_id: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepSetOfFiles {
    pub id: u32,
    #[serde(default)]
    pub direct_artifact_ids: Vec<u32>,
    #[serde(default)]
    pub transitive_dep_set_ids: Vec<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathFragment {
    pub id: u32,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub parent_id: Option<u32>,
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
    let artifact_paths = artifact_paths_by_id(aquery_output);
    let dep_sets_by_id = aquery_output
        .dep_set_of_files
        .iter()
        .map(|dep_set| (dep_set.id, dep_set))
        .collect::<BTreeMap<_, _>>();
    let directory = workspace.to_string_lossy().into_owned();
    let mut commands = Vec::new();
    let mut headers_written = BTreeSet::new();

    for action in &aquery_output.actions {
        if exclude_external_sources && action_is_external(action, &labels_by_target_id) {
            continue;
        }

        let input_paths = action_input_paths(action, &dep_sets_by_id, &artifact_paths);
        let files = get_files(
            action,
            workspace,
            &input_paths,
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

fn artifact_paths_by_id(aquery_output: &AqueryOutput) -> BTreeMap<u32, String> {
    let path_fragments = aquery_output
        .path_fragments
        .iter()
        .map(|fragment| (fragment.id, fragment))
        .collect::<BTreeMap<_, _>>();

    aquery_output
        .artifacts
        .iter()
        .filter_map(|artifact| {
            path_from_fragment(artifact.path_fragment_id, &path_fragments)
                .map(|path| (artifact.id, path))
        })
        .collect()
}

fn path_from_fragment(id: u32, path_fragments: &BTreeMap<u32, &PathFragment>) -> Option<String> {
    let mut labels = Vec::new();
    let mut current_id = Some(id);

    while let Some(fragment_id) = current_id {
        let fragment = path_fragments.get(&fragment_id)?;
        if !fragment.label.is_empty() {
            labels.push(fragment.label.as_str());
        }
        current_id = fragment.parent_id;
    }

    labels.reverse();
    Some(labels.join("/"))
}

fn action_input_paths(
    action: &Action,
    dep_sets_by_id: &BTreeMap<u32, &DepSetOfFiles>,
    artifact_paths: &BTreeMap<u32, String>,
) -> Vec<String> {
    let mut artifact_ids = BTreeSet::new();
    let mut visited_dep_sets = BTreeSet::new();
    for dep_set_id in &action.input_dep_set_ids {
        collect_artifact_ids(
            *dep_set_id,
            dep_sets_by_id,
            &mut visited_dep_sets,
            &mut artifact_ids,
        );
    }

    artifact_ids
        .iter()
        .filter_map(|artifact_id| artifact_paths.get(artifact_id).cloned())
        .collect()
}

fn collect_artifact_ids(
    dep_set_id: u32,
    dep_sets_by_id: &BTreeMap<u32, &DepSetOfFiles>,
    visited_dep_sets: &mut BTreeSet<u32>,
    artifact_ids: &mut BTreeSet<u32>,
) {
    if !visited_dep_sets.insert(dep_set_id) {
        return;
    }

    let Some(dep_set) = dep_sets_by_id.get(&dep_set_id) else {
        return;
    };

    artifact_ids.extend(dep_set.direct_artifact_ids.iter().copied());
    for transitive_dep_set_id in &dep_set.transitive_dep_set_ids {
        collect_artifact_ids(
            *transitive_dep_set_id,
            dep_sets_by_id,
            visited_dep_sets,
            artifact_ids,
        );
    }
}

#[derive(Debug, Default)]
struct Files {
    sources: Vec<String>,
    headers: Vec<String>,
}

fn get_files(
    action: &Action,
    workspace: &Path,
    input_paths: &[String],
    exclude_headers: &ExcludeHeaders,
    external_action: bool,
) -> Files {
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
        } else {
            maybe_push_header(
                workspace,
                path,
                exclude_headers,
                external_action,
                &mut files.headers,
            );
        }
    }

    for path in input_paths {
        if is_source_file(path) {
            files.sources.push(path.to_owned());
        } else {
            maybe_push_header(
                workspace,
                path,
                exclude_headers,
                external_action,
                &mut files.headers,
            );
        }
    }

    if exclude_headers != &ExcludeHeaders::All {
        let include_directories = include_directories(&action.arguments);
        let mut discovered_headers = BTreeSet::new();
        for source in &files.sources {
            discover_headers_for_source(
                workspace,
                source,
                &include_directories,
                exclude_headers,
                external_action,
                &mut discovered_headers,
            );
        }
        files.headers.extend(discovered_headers);
    }

    files.sources.sort();
    files.sources.dedup();
    files.headers.sort();
    files.headers.dedup();
    files
}

fn maybe_push_header(
    workspace: &Path,
    path: &str,
    exclude_headers: &ExcludeHeaders,
    external_action: bool,
    headers: &mut Vec<String>,
) {
    let header = header_path_to_emit(workspace, path);
    if should_include_header(&header, exclude_headers, external_action) {
        headers.push(header);
    }
}

fn header_path_to_emit(workspace: &Path, path: &str) -> String {
    let Some(suffix) = virtual_include_suffix(path) else {
        return path.to_owned();
    };
    find_workspace_header_by_suffix(workspace, suffix).unwrap_or_else(|| path.to_owned())
}

fn virtual_include_suffix(path: &str) -> Option<&str> {
    let (_, after_virtual_includes) = path.split_once("/_virtual_includes/")?;
    let (_, suffix) = after_virtual_includes.split_once('/')?;
    Some(suffix)
}

fn find_workspace_header_by_suffix(workspace: &Path, suffix: &str) -> Option<String> {
    if !is_header_file(suffix) {
        return None;
    }
    let suffix = Path::new(suffix);
    find_workspace_header_by_suffix_from(workspace, workspace, suffix)
}

fn find_workspace_header_by_suffix_from(
    workspace: &Path,
    directory: &Path,
    suffix: &Path,
) -> Option<String> {
    let entries = fs::read_dir(directory).ok()?;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            if should_skip_header_search_directory(&path) {
                continue;
            }
            if let Some(header) = find_workspace_header_by_suffix_from(workspace, &path, suffix) {
                return Some(header);
            }
        } else if path.ends_with(suffix) {
            if let Some(header) = path_to_workspace_relative(workspace, &path) {
                return Some(header);
            }
        }
    }
    None
}

fn should_skip_header_search_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| {
            matches!(
                name,
                "bazel-bin" | "bazel-out" | "bazel-testlogs" | "bazel-local"
            )
        })
}

fn include_directories(arguments: &[String]) -> Vec<String> {
    let mut directories = vec![".".to_owned()];
    let mut iterator = arguments.iter();

    while let Some(argument) = iterator.next() {
        match argument.as_str() {
            "-I" | "-iquote" | "-isystem" | "-idirafter" => {
                if let Some(directory) = iterator.next() {
                    directories.push(directory.to_owned());
                }
            }
            _ if argument.starts_with("-I") && argument.len() > 2 => {
                directories.push(argument[2..].to_owned());
            }
            _ if argument.starts_with("-iquote") && argument.len() > "-iquote".len() => {
                directories.push(argument["-iquote".len()..].to_owned());
            }
            _ if argument.starts_with("-isystem") && argument.len() > "-isystem".len() => {
                directories.push(argument["-isystem".len()..].to_owned());
            }
            _ if argument.starts_with("-idirafter") && argument.len() > "-idirafter".len() => {
                directories.push(argument["-idirafter".len()..].to_owned());
            }
            _ => {}
        }
    }

    directories.sort();
    directories.dedup();
    directories
}

fn discover_headers_for_source(
    workspace: &Path,
    source: &str,
    include_directories: &[String],
    exclude_headers: &ExcludeHeaders,
    external_action: bool,
    discovered_headers: &mut BTreeSet<String>,
) {
    let source_path = workspace.join(source);
    let Some(source_directory) = Path::new(source).parent() else {
        return;
    };
    let mut include_roots = vec![source_directory.to_path_buf()];
    include_roots.extend(include_directories.iter().map(PathBuf::from));

    discover_headers_from_file(
        workspace,
        &source_path,
        &include_roots,
        exclude_headers,
        external_action,
        discovered_headers,
    );
}

fn discover_headers_from_file(
    workspace: &Path,
    file: &Path,
    include_roots: &[PathBuf],
    exclude_headers: &ExcludeHeaders,
    external_action: bool,
    discovered_headers: &mut BTreeSet<String>,
) {
    let Ok(contents) = fs::read_to_string(file) else {
        return;
    };

    let mut file_include_roots = Vec::new();
    if let Some(parent) = path_to_workspace_path(workspace, file)
        .and_then(|path| path.parent().map(Path::to_path_buf))
    {
        file_include_roots.push(parent);
    }
    file_include_roots.extend(include_roots.iter().cloned());

    for include in contents.lines().filter_map(parse_include) {
        let Some(header) = resolve_include(workspace, include, &file_include_roots) else {
            continue;
        };
        let header = header_path_to_emit(workspace, &header);
        if !should_include_header(&header, exclude_headers, external_action) {
            continue;
        }
        if discovered_headers.insert(header.clone()) {
            discover_headers_from_file(
                workspace,
                &workspace.join(&header),
                include_roots,
                exclude_headers,
                external_action,
                discovered_headers,
            );
        }
    }
}

fn parse_include(line: &str) -> Option<&str> {
    let line = line.trim_start();
    let line = line.strip_prefix('#')?.trim_start();
    let line = line.strip_prefix("include")?.trim_start();
    let include = line
        .strip_prefix('"')
        .and_then(|rest| rest.split_once('"').map(|(include, _)| include))
        .or_else(|| {
            line.strip_prefix('<')
                .and_then(|rest| rest.split_once('>').map(|(include, _)| include))
        })?;
    if include.is_empty() {
        None
    } else {
        Some(include)
    }
}

fn resolve_include(workspace: &Path, include: &str, include_roots: &[PathBuf]) -> Option<String> {
    for root in include_roots {
        let candidate = root.join(include);
        let candidate = normalize_relative_path(&candidate);
        let workspace_candidate = if candidate.is_absolute() {
            candidate.clone()
        } else {
            workspace.join(&candidate)
        };
        if !workspace_candidate.is_file() {
            continue;
        }
        if let Some(relative) = path_to_workspace_relative(workspace, &workspace_candidate) {
            return Some(relative);
        }
        if !candidate.is_absolute() {
            return candidate.to_str().map(ToOwned::to_owned);
        }
    }
    None
}

fn path_to_workspace_relative(workspace: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(workspace)
        .ok()
        .and_then(|relative| relative.to_str())
        .map(ToOwned::to_owned)
}

fn path_to_workspace_path(workspace: &Path, path: &Path) -> Option<PathBuf> {
    path.strip_prefix(workspace).ok().map(Path::to_path_buf)
}

fn normalize_relative_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
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
            })
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
