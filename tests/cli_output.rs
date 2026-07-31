use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn dseuhid(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dseuhid"))
        .args(args)
        .output()
        .unwrap()
}

fn edgemap(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_edgemap"))
        .args(args)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).unwrap()
}

fn stderr(output: &Output) -> &str {
    std::str::from_utf8(&output.stderr).unwrap()
}

fn temp_dir(name: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "dseuhid-cli-{name}-{}-{unique}",
        std::process::id()
    ))
}

#[test]
fn dseuhid_help_uses_stdout() {
    let output = dseuhid(&["help"]);

    assert!(output.status.success());
    assert!(stdout(&output).contains("Usage: dseuhid [OPTIONS] [COMMAND]"));
    assert!(output.stderr.is_empty());
}

#[test]
fn dseuhid_version_uses_stdout() {
    let output = dseuhid(&["version"]);

    assert!(output.status.success());
    assert!(stdout(&output).starts_with("dseuhid "));
    assert!(output.stderr.is_empty());
}

#[test]
fn dseuhid_command_error_uses_stderr() {
    let output = dseuhid(&["unknown"]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        stderr(&output),
        "error: unknown command 'unknown'\nhint: run 'dseuhid help' for usage\n"
    );
}

#[test]
fn dseuhid_rejects_unknown_options_before_startup() {
    let output = dseuhid(&["--definitely-unknown"]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        stderr(&output),
        "error: unknown option '--definitely-unknown'\n\
         hint: run 'dseuhid help' for usage\n"
    );
}

#[test]
fn dseuhid_rejects_extra_positional_arguments() {
    let output = dseuhid(&["--config-path", "/tmp/config.toml", "extra"]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        stderr(&output),
        "error: unexpected argument 'extra'\n\
         hint: run 'dseuhid help' for usage\n"
    );
}

#[test]
fn dseuhid_rejects_duplicate_config_options() {
    let output = dseuhid(&["-c", "/tmp/one.toml", "--config-path", "/tmp/two.toml"]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        stderr(&output),
        "error: option '--config-path' may only be specified once\n\
         hint: run 'dseuhid help' for usage\n"
    );
}

#[test]
fn dseuhid_rejects_missing_config_option_value() {
    let output = dseuhid(&["--config-path"]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        stderr(&output),
        "error: option '--config-path' requires a path\n\
         hint: run 'dseuhid help' for usage\n"
    );
}

#[test]
fn edgemap_help_uses_stdout() {
    let output = edgemap(&["help"]);

    assert!(output.status.success());
    assert!(stdout(&output).contains("Usage: edgemap <COMMAND> [ARGS]"));
    assert!(output.stderr.is_empty());
}

#[test]
fn edgemap_capabilities_is_stable_parseable_toml() {
    let output = edgemap(&["capabilities"]);

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let parsed: toml::Value = toml::from_str(stdout(&output)).unwrap();
    assert_eq!(parsed["version"].as_integer(), Some(1));
    assert_eq!(parsed["output_devices"][0].as_str(), Some("auto"));
    assert_eq!(parsed["source_buttons"][0].as_str(), Some("cross"));
    assert_eq!(parsed["keyboard_keys"][0]["name"].as_str(), Some("a"));
    assert_eq!(parsed["keyboard_keys"][0]["code"].as_integer(), Some(30));
}

#[test]
fn edgemap_missing_command_uses_stderr() {
    let output = edgemap(&[]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("Usage: edgemap <COMMAND> [ARGS]"));
}

#[test]
fn create_config_without_path_reserves_stdout_for_toml() {
    let output = edgemap(&["create-config"]);

    assert!(output.status.success());
    assert!(stdout(&output).contains("# edgemap remap configuration"));
    assert!(!stdout(&output).contains("Created:"));
    assert!(output.stderr.is_empty());
}

#[test]
fn argument_error_uses_stderr() {
    let output = edgemap(&["switch-config", "one.toml", "extra"]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        stderr(&output),
        "error: too many arguments\nUsage: edgemap switch-config <PATH>\n"
    );
}

#[test]
fn create_and_validate_report_results_on_stdout() {
    let dir = temp_dir("single-config");
    let path = dir.join("config.toml");
    let path_arg = path.to_str().unwrap();

    let created = edgemap(&["create-config", path_arg]);
    assert!(created.status.success());
    assert_eq!(stdout(&created), format!("Created: {path_arg}\n"));
    assert!(created.stderr.is_empty());

    let validated = edgemap(&["validate", path_arg]);
    assert!(validated.status.success());
    assert_eq!(stdout(&validated), format!("Valid: {path_arg}\n"));
    assert!(validated.stderr.is_empty());

    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn create_config_never_overwrites_an_existing_file() {
    let dir = temp_dir("existing-config");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, "keep me\n").unwrap();
    let path_arg = path.to_str().unwrap();

    let output = edgemap(&["create-config", path_arg]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        stderr(&output),
        format!("error: config already exists: {path_arg}\n")
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep me\n");

    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn create_config_does_not_follow_a_dangling_symlink() {
    let dir = temp_dir("symlink-config");
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("target.toml");
    let link = dir.join("config.toml");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let link_arg = link.to_str().unwrap();

    let output = edgemap(&["create-config", link_arg]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        stderr(&output),
        format!("error: config already exists: {link_arg}\n")
    );
    assert!(!target.exists());
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());

    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn concurrent_create_config_has_exactly_one_winner() {
    let dir = temp_dir("concurrent-config");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    let path_arg = path.to_str().unwrap();

    let first = Command::new(env!("CARGO_BIN_EXE_edgemap"))
        .args(["create-config", path_arg])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let second = Command::new(env!("CARGO_BIN_EXE_edgemap"))
        .args(["create-config", path_arg])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let statuses = [
        first.wait_with_output().unwrap().status.success(),
        second.wait_with_output().unwrap().status.success(),
    ];

    assert_eq!(statuses.into_iter().filter(|success| *success).count(), 1);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        dseuhid::config::default_content()
    );

    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn batch_validation_keeps_result_table_on_stdout() {
    let xdg = temp_dir("batch-config");
    let config_dir = xdg.join("edgemap");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("valid.toml"),
        stdout(&edgemap(&["create-config"])),
    )
    .unwrap();
    std::fs::write(config_dir.join("broken.toml"), "not valid toml").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_edgemap"))
        .arg("validate")
        .env("XDG_CONFIG_HOME", &xdg)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(stdout(&output).contains("  OK    valid.toml"));
    assert!(stdout(&output).contains("  FAIL  broken.toml:"));
    assert!(stdout(&output).contains("Summary: 1/2 valid"));
    assert!(output.stderr.is_empty());

    std::fs::remove_dir_all(xdg).unwrap();
}
