//! Runs the `jenkins-cli` binary against the mock server built from the
//! OpenAPI spec.

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use support::{JenkinsMock, PASSWORD, USERNAME};
use tempfile::TempDir;

/// A temporary home directory holding `.cloudbees_credentials`.
struct Home {
    dir: TempDir,
}

impl Home {
    fn with_credentials(contents: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".cloudbees_credentials");
        fs::write(&path, contents).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        Self { dir }
    }

    fn valid() -> Self {
        Self::with_credentials(&format!("{USERNAME}:{PASSWORD}\n"))
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn credentials_file(&self) -> PathBuf {
        self.path().join(".cloudbees_credentials")
    }
}

/// The binary with a clean environment and `HOME` pointing at `home`.
fn cli(home: &Home) -> Command {
    let mut cmd = Command::cargo_bin("jenkins-cli").unwrap();
    cmd.env_clear()
        .env("HOME", home.path())
        .env("USERPROFILE", home.path());
    cmd
}

/// Runs the binary off the async runtime so the mock keeps serving.
async fn run(mut cmd: Command) -> assert_cmd::assert::Assert {
    tokio::task::spawn_blocking(move || cmd.assert())
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn reads_credentials_from_home_directory() {
    let mock = JenkinsMock::start().await;
    let home = Home::valid();
    let mut cmd = cli(&home);
    cmd.args(["--url", &mock.uri(), "whoami"]);
    run(cmd)
        .await
        .success()
        .stdout(predicate::str::contains("alice"))
        .stderr("");
}

#[tokio::test(flavor = "multi_thread")]
async fn url_can_come_from_the_environment() {
    let mock = JenkinsMock::start().await;
    let home = Home::valid();
    let mut cmd = cli(&home);
    cmd.env("JENKINS_URL", mock.uri()).args(["jobs", "list"]);
    run(cmd)
        .await
        .success()
        .stdout(predicate::str::contains("nightly-cleanup"));
}

#[tokio::test(flavor = "multi_thread")]
async fn credentials_file_flag_overrides_the_default() {
    let mock = JenkinsMock::start().await;
    let home = Home::with_credentials("nobody:wrong");
    let other = Home::valid();
    let mut cmd = cli(&home);
    cmd.args([
        "builds",
        "list",
        "deploy",
        "--url",
        &mock.uri(),
        "--credentials-file",
        other.credentials_file().to_str().unwrap(),
    ]);
    run(cmd)
        .await
        .success()
        .stdout(predicate::str::contains("#41"));
}

#[tokio::test(flavor = "multi_thread")]
async fn json_output() {
    let mock = JenkinsMock::start().await;
    let home = Home::valid();
    let mut cmd = cli(&home);
    cmd.args([
        "--url",
        &mock.uri(),
        "-o",
        "json",
        "nodes",
        "get",
        "linux-agent-1",
    ]);
    let output = run(cmd).await.success().get_output().stdout.clone();
    let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(value, mock.example("getComputer"));
}

#[tokio::test(flavor = "multi_thread")]
async fn build_log() {
    let mock = JenkinsMock::start_at("/my-controller").await;
    let home = Home::valid();
    let mut cmd = cli(&home);
    cmd.args(["--url", &mock.uri(), "builds", "log", "deploy"]);
    run(cmd)
        .await
        .success()
        .stdout(predicate::str::ends_with("Finished: SUCCESS\n"));
    assert_eq!(
        mock.requests().await[0].url.path(),
        "/my-controller/job/deploy/lastBuild/consoleText"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_credentials_fail_with_a_clear_message() {
    let mock = JenkinsMock::start().await;
    let home = Home::with_credentials(&format!("{USERNAME}:not-the-token"));
    let mut cmd = cli(&home);
    cmd.args(["--url", &mock.uri(), "info"]);
    run(cmd)
        .await
        .failure()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains("authentication failed (HTTP 401)"));
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_credentials_file() {
    let home = Home::valid();
    fs::remove_file(home.credentials_file()).unwrap();
    let mut cmd = cli(&home);
    cmd.args(["--url", "http://127.0.0.1:9/", "whoami"]);
    run(cmd)
        .await
        .failure()
        .stderr(predicate::str::contains("could not read credentials file"))
        .stderr(predicate::str::contains(".cloudbees_credentials"));
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_credentials_file() {
    let home = Home::with_credentials("just-a-token\n");
    let mut cmd = cli(&home);
    cmd.args(["--url", "http://127.0.0.1:9/", "whoami"]);
    run(cmd)
        .await
        .failure()
        .stderr(predicate::str::contains("no `:` separator"));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn warns_when_credentials_file_is_readable_by_others() {
    use std::os::unix::fs::PermissionsExt;
    let mock = JenkinsMock::start().await;
    let home = Home::valid();
    fs::set_permissions(home.credentials_file(), fs::Permissions::from_mode(0o644)).unwrap();
    let mut cmd = cli(&home);
    cmd.args(["--url", &mock.uri(), "whoami"]);
    run(cmd)
        .await
        .success()
        .stderr(predicate::str::contains("chmod 600"));
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_url() {
    let home = Home::valid();
    let mut cmd = cli(&home);
    cmd.arg("whoami");
    run(cmd)
        .await
        .failure()
        .stderr(predicate::str::contains("pass --url or set JENKINS_URL"));
}

#[tokio::test(flavor = "multi_thread")]
async fn not_found_exits_with_failure() {
    let mock = JenkinsMock::start().await;
    let home = Home::valid();
    let mut cmd = cli(&home);
    // Paths under an unknown controller are not in the spec, so the mock answers 404.
    let url = format!("{}no-such-controller/", mock.uri());
    cmd.args(["--url", &url, "whoami"]);
    run(cmd)
        .await
        .failure()
        .stderr(predicate::str::contains("not found (HTTP 404)"));
}

#[test]
fn help_lists_only_read_commands() {
    let home = Home::valid();
    cli(&home)
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("whoami"))
        .stdout(predicate::str::contains("builds"))
        .stdout(predicate::str::contains(".cloudbees_credentials"));
}

#[test]
fn invalid_build_selector_is_a_usage_error() {
    let home = Home::valid();
    cli(&home)
        .args([
            "--url",
            "http://127.0.0.1:9/",
            "builds",
            "get",
            "deploy",
            "newest",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("expected a build number"));
}
