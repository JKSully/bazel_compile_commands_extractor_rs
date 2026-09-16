use serde_json::Value;
use std::env;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn generates_compile_commands_from_a_real_bazel_workspace() -> Result<(), Box<dyn Error>> {
    let runfiles_root = required_environment_path("TEST_SRCDIR")?;
    let test_workspace = env::var("TEST_WORKSPACE")?;
    let source_root = runfiles_root.join(test_workspace);
    let temporary_root = required_environment_path("TEST_TMPDIR")?.join("bazel_e2e_test");
    let module_root = temporary_root.join("module");
    let fixture_root = module_root.join("tests/fixtures/bzlmod");

    if temporary_root.exists() {
        fs::remove_dir_all(&temporary_root)?;
    }
    fs::create_dir_all(&fixture_root)?;

    copy_module_files(&source_root, &module_root)?;
    copy_directory(&source_root.join("tests/fixtures/bzlmod"), &fixture_root)?;
    fs::rename(
        fixture_root.join("BUILD.fixture"),
        fixture_root.join("BUILD.bazel"),
    )?;

    let output_base = temporary_root.join("output-base");
    let output = Command::new("bazel")
        .arg(format!("--output_base={}", output_base.display()))
        .args(["run", "//:refresh_compile_commands"])
        .current_dir(&fixture_root)
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "nested Bazel invocation failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        )
        .into());
    }

    let compile_commands_path = fixture_root.join("compile_commands.json");
    let compile_commands =
        serde_json::from_slice::<Vec<Value>>(&fs::read(&compile_commands_path)?)?;

    if compile_commands.len() != 1 {
        return Err(format!(
            "expected one compile command, found {}",
            compile_commands.len(),
        )
        .into());
    }
    assert_has_compile_command(&compile_commands, "src/hello.cc", &fixture_root)?;

    Ok(())
}

fn required_environment_path(name: &str) -> Result<PathBuf, Box<dyn Error>> {
    env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{name} is not set by the Bazel test runner").into())
}

fn copy_module_files(source_root: &Path, module_root: &Path) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(module_root)?;
    for relative_path in [
        "BUILD.bazel",
        "Cargo.lock",
        "Cargo.toml",
        "LICENSE",
        "MODULE.bazel",
        "refresh_compile_commands.bzl",
    ] {
        copy_file(source_root, module_root, relative_path)?;
    }
    copy_directory(&source_root.join("src"), &module_root.join("src"))?;
    Ok(())
}

fn copy_file(
    source_root: &Path,
    destination_root: &Path,
    relative_path: &str,
) -> Result<(), Box<dyn Error>> {
    let destination = destination_root.join(relative_path);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source_root.join(relative_path), destination)?;
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else if source_path.is_file() {
            fs::copy(source_path, destination_path)?;
        }
    }
    Ok(())
}

fn assert_has_compile_command(
    entries: &[Value],
    expected_file: &str,
    workspace: &Path,
) -> Result<(), Box<dyn Error>> {
    let command = entries
        .iter()
        .find(|entry| entry["file"].as_str() == Some(expected_file))
        .ok_or_else(|| format!("missing compile command for {expected_file}"))?;

    if command["directory"].as_str() != workspace.to_str() {
        return Err(format!("compile command for {expected_file} has the wrong directory").into());
    }
    if !command["arguments"]
        .as_array()
        .is_some_and(|arguments| arguments.iter().any(|argument| argument == expected_file))
    {
        return Err(format!("compile arguments do not contain {expected_file}").into());
    }

    Ok(())
}
