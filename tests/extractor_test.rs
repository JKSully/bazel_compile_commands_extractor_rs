use bazel_compile_commands::convert_compile_commands;
use bazel_compile_commands::parse_args;
use bazel_compile_commands::AqueryOutput;
use bazel_compile_commands::ExcludeHeaders;
use bazel_compile_commands::ExtractorConfig;
use bazel_compile_commands::TargetSpec;
use std::path::Path;

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
