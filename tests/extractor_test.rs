use bazel_compile_commands_extractor::convert_compile_commands;
use bazel_compile_commands_extractor::parse_args;
use bazel_compile_commands_extractor::AqueryOutput;
use bazel_compile_commands_extractor::ExcludeHeaders;
use bazel_compile_commands_extractor::ExtractorConfig;
use bazel_compile_commands_extractor::TargetSpec;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;

#[test]
fn converts_aquery_compile_actions_to_compile_commands() {
    let aquery = serde_json::from_str::<AqueryOutput>(
        r#"
        {
          "targets": [
            {"id": 1, "label": "//app:lib"},
            {"id": 2, "label": "@dep//:lib"}
          ],
          "actions": [
            {
              "targetId": 1,
              "arguments": [
                "external/local_config_cc/cc_wrapper.sh",
                "-Iinclude",
                "app/main.cc",
                "app/main.h",
                "-o",
                "bazel-out/app/main.o",
                "-fno-canonical-system-headers"
              ]
            },
            {
              "targetId": 1,
              "arguments": ["clang++", "app/other.cc", "app/main.h", "-o", "ignored.o"]
            },
            {
              "targetId": 2,
              "arguments": ["clang++", "external/dep/dep.cc", "external/dep/dep.h"]
            }
          ]
        }
        "#,
    )
    .expect("test fixture should deserialize");

    let commands = convert_compile_commands(
        &aquery,
        Path::new("/workspace"),
        &ExcludeHeaders::External,
        false,
    );

    let files = commands
        .iter()
        .map(|command| command.file.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        files,
        vec![
            "app/main.cc",
            "app/main.h",
            "app/other.cc",
            "external/dep/dep.cc"
        ]
    );
    assert!(commands[0]
        .arguments
        .iter()
        .all(|argument| argument != "-fno-canonical-system-headers"));
    assert!(commands
        .iter()
        .all(|command| command.directory == "/workspace"));
}

#[test]
fn includes_declared_header_input_artifacts() {
    let aquery = serde_json::from_str::<AqueryOutput>(
        r#"
        {
          "artifacts": [
            {"id": 1, "pathFragmentId": 3}
          ],
          "actions": [
            {
              "targetId": 1,
              "arguments": ["clang++", "app/main.cc", "-o", "bazel-out/app/main.o"],
              "inputDepSetIds": [1]
            }
          ],
          "targets": [
            {"id": 1, "label": "//app:lib"}
          ],
          "depSetOfFiles": [
            {"id": 1, "directArtifactIds": [1]}
          ],
          "pathFragments": [
            {"id": 1, "label": "include"},
            {"id": 2, "label": "example", "parentId": 1},
            {"id": 3, "label": "greeting.h", "parentId": 2}
          ]
        }
        "#,
    )
    .expect("test fixture should deserialize");

    let commands = convert_compile_commands(
        &aquery,
        Path::new("/workspace"),
        &ExcludeHeaders::None,
        false,
    );
    let files = commands
        .iter()
        .map(|command| command.file.as_str())
        .collect::<Vec<_>>();

    assert_eq!(files, vec!["app/main.cc", "include/example/greeting.h"]);
}

#[test]
fn discovers_headers_included_by_sources() {
    let workspace = create_test_workspace("discovers_headers_included_by_sources")
        .expect("test workspace should be created");
    fs::create_dir_all(workspace.join("app/detail"))
        .expect("app detail directory should be created");
    fs::create_dir_all(workspace.join("include/public"))
        .expect("public include directory should be created");
    fs::write(
        workspace.join("app/main.cc"),
        "#include \"main.h\"\n#include <public/api.h>\n",
    )
    .expect("source should be written");
    fs::write(
        workspace.join("app/main.h"),
        "#include \"detail/detail.h\"\n",
    )
    .expect("header should be written");
    fs::write(workspace.join("app/detail/detail.h"), "\n")
        .expect("nested header should be written");
    fs::write(
        workspace.join("include/public/api.h"),
        "#include \"nested.h\"\n",
    )
    .expect("include header should be written");
    fs::write(workspace.join("include/public/nested.h"), "\n")
        .expect("nested include header should be written");

    let aquery = serde_json::from_str::<AqueryOutput>(
        r#"
        {
          "targets": [
            {"id": 1, "label": "//app:lib"}
          ],
          "actions": [
            {
              "targetId": 1,
              "arguments": ["clang++", "-Iinclude", "app/main.cc", "-o", "bazel-out/app/main.o"]
            }
          ]
        }
        "#,
    )
    .expect("test fixture should deserialize");

    let commands = convert_compile_commands(&aquery, &workspace, &ExcludeHeaders::None, false);
    let files = commands
        .iter()
        .map(|command| command.file.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        files,
        vec![
            "app/main.cc",
            "app/detail/detail.h",
            "app/main.h",
            "include/public/api.h",
            "include/public/nested.h",
        ]
    );
}

#[test]
fn excludes_external_source_actions_when_requested() {
    let aquery = serde_json::from_str::<AqueryOutput>(
        r#"
        {
          "targets": [
            {"id": 1, "label": "//app:lib"},
            {"id": 2, "label": "@dep//:lib"}
          ],
          "actions": [
            {"targetId": 1, "arguments": ["clang++", "app/main.cc"]},
            {"targetId": 2, "arguments": ["clang++", "external/dep/dep.cc"]}
          ]
        }
        "#,
    )
    .expect("test fixture should deserialize");

    let commands = convert_compile_commands(
        &aquery,
        Path::new("/workspace"),
        &ExcludeHeaders::None,
        true,
    );

    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].file, "app/main.cc");
}

fn create_test_workspace(name: &str) -> Result<PathBuf, io::Error> {
    let workspace = std::env::temp_dir().join(format!(
        "bazel_compile_commands_extractor_{name}_{}",
        std::process::id()
    ));
    match fs::remove_dir_all(&workspace) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::create_dir_all(&workspace)?;
    Ok(workspace)
}

#[test]
fn parses_generated_wrapper_arguments() {
    let config = parse_args([
        "--target".to_owned(),
        "//app:bin=--config=clang --cpu=k8".to_owned(),
        "--exclude_headers".to_owned(),
        "all".to_owned(),
        "--exclude_external_sources".to_owned(),
        "--".to_owned(),
        "--compilation_mode=dbg".to_owned(),
    ])
    .expect("arguments should parse");

    assert_eq!(
        config,
        ExtractorConfig {
            targets: vec![TargetSpec {
                target: "//app:bin".to_owned(),
                flags: "--config=clang --cpu=k8".to_owned(),
            }],
            exclude_headers: ExcludeHeaders::All,
            exclude_external_sources: true,
            runtime_bazel_flags: vec!["--compilation_mode=dbg".to_owned()],
        }
    );
}
