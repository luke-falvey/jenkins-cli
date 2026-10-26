//! Runs the client and every command against the mock server built from the
//! OpenAPI spec.

mod support;

use std::time::Duration;

use jenkins_cli::cli::{
    BuildArgs, BuildsCommand, Command, JobsCommand, NodesCommand, OutputFormat, PluginsCommand,
    QueueCommand, ViewsCommand,
};
use jenkins_cli::client::{ClientError, JenkinsClient};
use jenkins_cli::credentials::Credentials;
use jenkins_cli::models::{self, BuildSelector};
use jenkins_cli::{Error, run};
use serde::de::DeserializeOwned;
use serde_json::Value;
use support::{JenkinsMock, PASSWORD, USERNAME};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

fn client_for(mock: &JenkinsMock) -> JenkinsClient {
    JenkinsClient::new(
        &mock.uri(),
        Credentials::new(USERNAME, PASSWORD),
        Duration::from_secs(5),
    )
    .unwrap()
}

async fn run_text(client: &JenkinsClient, command: Command) -> String {
    run_with(client, command, OutputFormat::Text).await
}

async fn run_with(client: &JenkinsClient, command: Command, format: OutputFormat) -> String {
    let mut out = Vec::new();
    run(client, &command, format, &mut out)
        .await
        .unwrap_or_else(|e| panic!("{command:?} failed: {e:?}"));
    String::from_utf8(out).unwrap()
}

fn build_args(job: &str, build: &str) -> BuildArgs {
    BuildArgs {
        job: job.to_string(),
        build: build.parse().unwrap(),
    }
}

/// One of each command the CLI offers.
fn all_commands() -> Vec<Command> {
    vec![
        Command::Whoami,
        Command::Info,
        Command::Jobs(JobsCommand::List { folder: None }),
        Command::Jobs(JobsCommand::List {
            folder: Some("team-a".into()),
        }),
        Command::Jobs(JobsCommand::Get {
            job: "deploy".into(),
        }),
        Command::Builds(BuildsCommand::List {
            job: "deploy".into(),
            limit: 5,
        }),
        Command::Builds(BuildsCommand::Get(build_args("deploy", "42"))),
        Command::Builds(BuildsCommand::Log(build_args("deploy", "lastBuild"))),
        Command::Queue(QueueCommand::List),
        Command::Queue(QueueCommand::Get { id: 1017 }),
        Command::Nodes(NodesCommand::List),
        Command::Nodes(NodesCommand::Get {
            name: "linux-agent-1".into(),
        }),
        Command::Views(ViewsCommand::List),
        Command::Views(ViewsCommand::Get {
            name: "release".into(),
        }),
        Command::Plugins(PluginsCommand::List { updates: false }),
    ]
}

// --- The spec and its examples ----------------------------------------------

fn assert_parses<T: DeserializeOwned>(spec: &Value, operation_id: &str) {
    for example in support::operations(spec)
        .into_iter()
        .find(|op| op.id == operation_id)
        .unwrap_or_else(|| panic!("no operation {operation_id}"))
        .examples
    {
        serde_json::from_value::<T>(example.value).unwrap_or_else(|e| {
            panic!(
                "{operation_id} example `{}` does not parse: {e}",
                example.name
            )
        });
    }
}

#[test]
fn every_spec_example_parses_into_its_model() {
    let spec = support::load_spec();
    assert_parses::<models::Controller>(&spec, "getController");
    assert_parses::<models::WhoAmI>(&spec, "getWhoAmI");
    assert_parses::<models::Job>(&spec, "getJob");
    assert_parses::<models::Build>(&spec, "getBuild");
    assert_parses::<models::Queue>(&spec, "getQueue");
    assert_parses::<models::QueueItem>(&spec, "getQueueItem");
    assert_parses::<models::ComputerSet>(&spec, "getComputers");
    assert_parses::<models::Computer>(&spec, "getComputer");
    assert_parses::<models::View>(&spec, "getView");
    assert_parses::<models::PluginManager>(&spec, "getPlugins");
    assert_parses::<String>(&spec, "getBuildConsoleText");
}

#[test]
fn spec_only_describes_read_operations() {
    let spec = support::load_spec();
    for (path, item) in spec["paths"].as_object().unwrap() {
        for verb in ["put", "post", "delete", "patch"] {
            assert!(item.get(verb).is_none(), "{verb} {path} is not read-only");
        }
    }
}

#[tokio::test]
async fn commands_cover_every_operation_in_the_spec() {
    let mock = JenkinsMock::start().await;
    let client = client_for(&mock);
    for command in all_commands() {
        run_text(&client, command).await;
    }
    let requests = mock.requests().await;
    for op in support::operations(&mock.spec) {
        let pattern = regex::Regex::new(&op.path_regex("", &[])).unwrap();
        assert!(
            requests.iter().any(|r| pattern.is_match(r.url.path())),
            "no command calls {} (GET {})",
            op.id,
            op.path
        );
    }
    assert!(requests.iter().all(|r| r.method.as_str() == "GET"));
}

// --- Authentication ------------------------------------------------------------

#[tokio::test]
async fn sends_basic_auth_credentials() {
    let mock = JenkinsMock::start().await;
    let value = client_for(&mock).who_am_i().await.unwrap();
    assert_eq!(value, mock.example("getWhoAmI"));
}

#[tokio::test]
async fn wrong_password_is_unauthorized() {
    let mock = JenkinsMock::start().await;
    let client = JenkinsClient::new(
        &mock.uri(),
        Credentials::new(USERNAME, "wrong"),
        Duration::from_secs(5),
    )
    .unwrap();
    let err = client.who_am_i().await.unwrap_err();
    assert!(
        matches!(&err, ClientError::Unauthorized { username } if username == USERNAME),
        "{err:?}"
    );
    assert!(err.to_string().contains("credentials file"));
}

// --- Commands -----------------------------------------------------------------

#[tokio::test]
async fn whoami() {
    let mock = JenkinsMock::start().await;
    let out = run_text(&client_for(&mock), Command::Whoami).await;
    assert!(out.contains("alice"), "{out}");
    assert!(out.contains("authenticated, platform-team"), "{out}");
}

#[tokio::test]
async fn info() {
    let mock = JenkinsMock::start().await;
    let out = run_text(&client_for(&mock), Command::Info).await;
    assert!(
        out.contains("CloudBees CI controller for the platform team"),
        "{out}"
    );
    assert!(out.contains("Jobs:"), "{out}");
    assert!(out.contains("Primary view:  all"), "{out}");
}

#[tokio::test]
async fn jobs_list_top_level_asks_for_job_fields_only() {
    let mock = JenkinsMock::start().await;
    let out = run_text(
        &client_for(&mock),
        Command::Jobs(JobsCommand::List { folder: None }),
    )
    .await;

    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[0].starts_with("NAME"), "{out}");
    assert!(out.contains("team-a"), "{out}");
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("deploy") && l.contains("pipeline") && l.contains("success"))
    );
    assert!(lines.iter().any(|l| l.starts_with("nightly-cleanup")
        && l.contains("freestyle")
        && l.contains("failure (building)")));

    let requests = mock.requests().await;
    let query: Vec<_> = requests[0].url.query_pairs().collect();
    assert_eq!(requests[0].url.path(), "/api/json");
    assert_eq!(
        query,
        [("tree".into(), jenkins_cli::client::JOBS_TREE.into())]
    );
}

#[tokio::test]
async fn jobs_list_in_folder() {
    let mock = JenkinsMock::start().await;
    let out = run_text(
        &client_for(&mock),
        Command::Jobs(JobsCommand::List {
            folder: Some("team-a".into()),
        }),
    )
    .await;
    assert!(out.contains("api-service"), "{out}");
    assert!(out.contains("multibranch"), "{out}");
    assert!(out.contains("integration-tests"), "{out}");
    assert_eq!(mock.requests().await[0].url.path(), "/job/team-a/api/json");
}

#[tokio::test]
async fn jobs_get_in_nested_folder() {
    let mock = JenkinsMock::start().await;
    let out = run_text(
        &client_for(&mock),
        Command::Jobs(JobsCommand::Get {
            job: "team-a/services/deploy".into(),
        }),
    )
    .await;
    assert!(out.contains("Type:"), "{out}");
    assert!(out.contains("pipeline"), "{out}");
    assert!(out.contains("Last failure: #40"), "{out}");
    assert!(out.contains("Health:       80%"), "{out}");
    assert_eq!(
        mock.requests().await[0].url.path(),
        "/job/team-a/job/services/job/deploy/api/json"
    );
}

#[tokio::test]
async fn jobs_get_folder_lists_its_items() {
    let mock = JenkinsMock::start().await;
    let out = run_text(
        &client_for(&mock),
        Command::Jobs(JobsCommand::Get {
            job: "team-a".into(),
        }),
    )
    .await;
    assert!(out.contains("folder"), "{out}");
    assert!(out.contains("api-service"), "{out}");
    assert!(!out.contains("Last build"), "folders have no builds: {out}");
}

#[tokio::test]
async fn builds_list_requests_a_limited_range() {
    let mock = JenkinsMock::start().await;
    let out = run_text(
        &client_for(&mock),
        Command::Builds(BuildsCommand::List {
            job: "deploy".into(),
            limit: 3,
        }),
    )
    .await;
    let rows: Vec<&str> = out.lines().skip(1).collect();
    assert_eq!(rows.len(), 3, "{out}");
    assert!(
        rows[0].starts_with("#42") && rows[0].contains("SUCCESS") && rows[0].contains("1m 23s")
    );
    assert!(rows[2].starts_with("#40") && rows[2].contains("FAILURE"));

    let requests = mock.requests().await;
    let tree = requests[0]
        .url
        .query_pairs()
        .find(|(k, _)| k == "tree")
        .unwrap()
        .1
        .into_owned();
    assert!(
        tree.starts_with("builds[") && tree.ends_with("]{0,3}"),
        "{tree}"
    );
}

#[tokio::test]
async fn builds_get() {
    let mock = JenkinsMock::start().await;
    let out = run_text(
        &client_for(&mock),
        Command::Builds(BuildsCommand::Get(build_args("deploy", "42"))),
    )
    .await;
    assert!(out.contains("deploy #42"), "{out}");
    assert!(out.contains("SUCCESS"), "{out}");
    assert!(out.contains("Started by user alice"), "{out}");
    assert!(out.contains("target/report.html"), "{out}");
    assert_eq!(
        mock.requests().await[0].url.path(),
        "/job/deploy/42/api/json"
    );
}

#[tokio::test]
async fn builds_get_by_permalink() {
    let mock = JenkinsMock::start().await;
    let client = client_for(&mock);
    client
        .build(
            "team-a/integration-tests",
            &BuildSelector::Permalink("lastFailedBuild"),
        )
        .await
        .unwrap();
    assert_eq!(
        mock.requests().await[0].url.path(),
        "/job/team-a/job/integration-tests/lastFailedBuild/api/json"
    );
}

#[tokio::test]
async fn builds_log_prints_plain_text_even_in_json_mode() {
    let mock = JenkinsMock::start().await;
    let out = run_with(
        &client_for(&mock),
        Command::Builds(BuildsCommand::Log(build_args("deploy", "42"))),
        OutputFormat::Json,
    )
    .await;
    assert_eq!(out, mock.example("getBuildConsoleText").as_str().unwrap());
    assert_eq!(
        mock.requests().await[0].url.path(),
        "/job/deploy/42/consoleText"
    );
}

#[tokio::test]
async fn queue_list() {
    let mock = JenkinsMock::start().await;
    let out = run_text(&client_for(&mock), Command::Queue(QueueCommand::List)).await;
    assert!(
        out.lines()
            .any(|l| l.starts_with("1018") && l.contains("buildable")),
        "{out}"
    );
    assert!(
        out.lines()
            .any(|l| l.starts_with("1019") && l.contains("blocked")),
        "{out}"
    );
    assert!(out.contains("Waiting for next available executor"), "{out}");
    assert!(out.contains("Build #12 is already in progress"), "{out}");
}

#[tokio::test]
async fn queue_get_shows_started_build() {
    let mock = JenkinsMock::start().await;
    let out = run_text(
        &client_for(&mock),
        Command::Queue(QueueCommand::Get { id: 1017 }),
    )
    .await;
    assert!(out.contains("left"), "{out}");
    assert!(out.contains("#42"), "{out}");
    assert_eq!(
        mock.requests().await[0].url.path(),
        "/queue/item/1017/api/json"
    );
}

#[tokio::test]
async fn nodes_list() {
    let mock = JenkinsMock::start().await;
    let out = run_text(&client_for(&mock), Command::Nodes(NodesCommand::List)).await;
    assert!(
        out.lines()
            .any(|l| l.starts_with("linux-agent-1") && l.contains("busy")),
        "{out}"
    );
    assert!(out.lines().any(|l| l.starts_with("windows-agent-1")
        && l.contains("offline")
        && l.contains("Disk replacement")));
    assert!(out.contains("1 of 6 executors busy"), "{out}");
    let requests = mock.requests().await;
    assert!(requests[0].url.query_pairs().any(|(k, _)| k == "tree"));
}

#[tokio::test]
async fn nodes_get_encodes_the_built_in_node_name() {
    let mock = JenkinsMock::start().await;
    let out = run_text(
        &client_for(&mock),
        Command::Nodes(NodesCommand::Get {
            name: "(built-in)".into(),
        }),
    )
    .await;
    assert!(out.contains("linux,docker"), "{out}");
    assert_eq!(
        mock.requests().await[0].url.path(),
        "/computer/(built-in)/api/json"
    );
}

#[tokio::test]
async fn views_list_and_get() {
    let mock = JenkinsMock::start().await;
    let client = client_for(&mock);
    let list = run_text(&client, Command::Views(ViewsCommand::List)).await;
    assert!(list.contains("all") && list.contains("release"), "{list}");

    let view = run_text(
        &client,
        Command::Views(ViewsCommand::Get {
            name: "release".into(),
        }),
    )
    .await;
    assert!(view.contains("Jobs that take part in a release"), "{view}");
    assert!(view.contains("deploy"), "{view}");
}

#[tokio::test]
async fn plugins_list() {
    let mock = JenkinsMock::start().await;
    let client = client_for(&mock);
    let all = run_text(
        &client,
        Command::Plugins(PluginsCommand::List { updates: false }),
    )
    .await;
    assert_eq!(all.lines().count(), 5, "header plus 4 plugins: {all}");
    assert!(all.contains("Pipeline: Job"), "{all}");

    let updates = run_text(
        &client,
        Command::Plugins(PluginsCommand::List { updates: true }),
    )
    .await;
    assert_eq!(updates.lines().count(), 2, "{updates}");
    assert!(
        updates.contains("git") && updates.contains("available"),
        "{updates}"
    );
}

#[tokio::test]
async fn json_output_is_the_raw_response() {
    let mock = JenkinsMock::start().await;
    let client = client_for(&mock);
    let cases = [
        (Command::Whoami, mock.example("getWhoAmI")),
        (Command::Info, mock.example("getController")),
        (
            Command::Jobs(JobsCommand::List {
                folder: Some("team-a".into()),
            }),
            mock.named_example("getJob", "folder"),
        ),
        (Command::Queue(QueueCommand::List), mock.example("getQueue")),
        (
            Command::Builds(BuildsCommand::Get(build_args("deploy", "42"))),
            mock.example("getBuild"),
        ),
    ];
    for (command, expected) in cases {
        let out = run_with(&client, command, OutputFormat::Json).await;
        let actual: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(actual, expected);
    }
}

// --- CloudBees controller paths and errors -------------------------------------

#[tokio::test]
async fn works_under_a_controller_path() {
    let mock = JenkinsMock::start_at("/my-controller").await;
    let base = mock.uri();
    assert!(base.ends_with("/my-controller/"));
    for url in [base.clone(), base.trim_end_matches('/').to_string()] {
        let client = JenkinsClient::new(
            &url,
            Credentials::new(USERNAME, PASSWORD),
            Duration::from_secs(5),
        )
        .unwrap();
        let out = run_text(
            &client,
            Command::Jobs(JobsCommand::Get {
                job: "deploy".into(),
            }),
        )
        .await;
        assert!(out.contains("deploy"), "{out}");
    }
    let requests = mock.requests().await;
    assert!(
        requests
            .iter()
            .all(|r| r.url.path() == "/my-controller/job/deploy/api/json")
    );
}

#[tokio::test]
async fn http_errors_are_reported() {
    let mock = JenkinsMock::start().await;
    for (job, status) in [("missing", 404), ("secret", 403), ("broken", 500)] {
        Mock::given(method("GET"))
            .and(path(format!("/job/{job}/api/json")))
            .respond_with(ResponseTemplate::new(status))
            .with_priority(1)
            .mount(&mock.server)
            .await;
    }
    let client = client_for(&mock);

    let err = client.job("missing", None).await.unwrap_err();
    assert!(matches!(err, ClientError::NotFound { .. }), "{err:?}");
    assert!(err.to_string().contains("/job/missing/api/json"), "{err}");

    let err = client.job("secret", None).await.unwrap_err();
    assert!(matches!(err, ClientError::Forbidden { .. }), "{err:?}");

    let err = client.job("broken", None).await.unwrap_err();
    assert!(
        matches!(err, ClientError::Status { status, .. } if status.as_u16() == 500),
        "{err:?}"
    );
}

#[tokio::test]
async fn non_json_response_is_a_decode_error() {
    let mock = JenkinsMock::start().await;
    Mock::given(method("GET"))
        .and(path("/job/login-page/api/json"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("<html>Sign in</html>", "text/html"))
        .with_priority(1)
        .mount(&mock.server)
        .await;
    let mut out = Vec::new();
    let err = run(
        &client_for(&mock),
        &Command::Jobs(JobsCommand::Get {
            job: "login-page".into(),
        }),
        OutputFormat::Text,
        &mut out,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, Error::Client(ClientError::Decode { .. })),
        "{err:?}"
    );
}

#[tokio::test]
async fn unreachable_server_is_a_request_error() {
    // Grab a free port, then close it so nothing is listening.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let client = JenkinsClient::new(
        &format!("http://127.0.0.1:{port}/"),
        Credentials::new(USERNAME, PASSWORD),
        Duration::from_secs(2),
    )
    .unwrap();
    let err = client.who_am_i().await.unwrap_err();
    assert!(matches!(err, ClientError::Request { .. }), "{err:?}");
}
