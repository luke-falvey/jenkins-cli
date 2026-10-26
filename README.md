# jenkins-cli

A read-only command line client for the Jenkins REST API on CloudBees CI,
written in Rust with [clap](https://docs.rs/clap) and
[reqwest](https://docs.rs/reqwest).

This first version only reads. It never sends anything other than `GET`
requests, so it cannot start, stop or change anything.

## Install

Download a static Linux binary for amd64 or arm64 from the
[releases page](https://github.com/luke-falvey/jenkins-cli/releases). The
binaries have no dependencies, so they run on any Linux distribution:

```sh
version=v0.1.0
arch=amd64   # or arm64
curl -fsSLO "https://github.com/luke-falvey/jenkins-cli/releases/download/$version/jenkins-cli-$version-linux-$arch.tar.gz"
curl -fsSLO "https://github.com/luke-falvey/jenkins-cli/releases/download/$version/SHA256SUMS"
sha256sum --check --ignore-missing SHA256SUMS
tar -xzf "jenkins-cli-$version-linux-$arch.tar.gz"
sudo install "jenkins-cli-$version-linux-$arch/jenkins-cli" /usr/local/bin/
```

Or build from source with Rust:

```sh
cargo install --path .
```

## Credentials

The CLI uses HTTP basic authentication only. It reads credentials from
`~/.cloudbees_credentials`, a file holding a single line:

```text
username:password
```

Use a Jenkins API token as the password (in CloudBees CI: your user menu →
**Security** → **API Token**). The password is everything after the first `:`,
so it may contain colons. Blank lines and lines starting with `#` are ignored.

Keep the file private:

```sh
chmod 600 ~/.cloudbees_credentials
```

The CLI prints a warning if other users can read it. To use a different file,
pass `--credentials-file PATH` or set `CLOUDBEES_CREDENTIALS_FILE`.

## Usage

Point the CLI at a controller with `--url` or the `JENKINS_URL` environment
variable. On CloudBees CI this is the controller's URL, including its path:

```sh
export JENKINS_URL=https://cloudbees.example.com/my-controller/
```

| Command | What it shows | Jenkins endpoint |
| --- | --- | --- |
| `whoami` | The user you are authenticated as | `/whoAmI/api/json` |
| `info` | Controller overview | `/api/json` |
| `jobs list [--folder F]` | Jobs at the top level or in a folder | `/api/json`, `/job/F/api/json` |
| `jobs get JOB` | One job, folder or multibranch project | `/job/JOB/api/json` |
| `builds list JOB [-n N]` | The last N builds (default 20) | `/job/JOB/api/json?tree=builds[...]{0,N}` |
| `builds get JOB [BUILD]` | One build | `/job/JOB/BUILD/api/json` |
| `builds log JOB [BUILD]` | A build's console log | `/job/JOB/BUILD/consoleText` |
| `queue list` | Items waiting in the queue | `/queue/api/json` |
| `queue get ID` | One queue item and the build it started | `/queue/item/ID/api/json` |
| `nodes list` | All nodes and their executors | `/computer/api/json` |
| `nodes get NAME` | One node (the built-in node is `(built-in)`) | `/computer/NAME/api/json` |
| `views list` | Top-level views | `/api/json?tree=views[...]` |
| `views get NAME` | One view and its jobs | `/view/NAME/api/json` |
| `plugins list [--updates]` | Installed plugins | `/pluginManager/api/json` |

- **Jobs in folders** use `/` between names: `team-a/api-service/main`. Type
  each name the way it appears in the Jenkins URL path, so a multibranch
  branch named `feature/login` is `team-a/api-service/feature%2Flogin`.
- **BUILD** is a build number or a permalink: `last`, `lastBuild`,
  `lastSuccessfulBuild`, `lastFailedBuild`, `lastStableBuild`,
  `lastUnstableBuild`, `lastUnsuccessfulBuild` or `lastCompletedBuild`. It
  defaults to `lastBuild`.
- **`-o json`** prints the JSON Jenkins returned instead of a table. It works
  with every command except `builds log`, which always prints plain text.
- `--timeout SECONDS` sets the request timeout (default 30).

Examples:

```console
$ jenkins-cli jobs list --folder team-a
NAME               TYPE         STATUS    URL
api-service        multibranch  -         https://cloudbees.example.com/my-controller/job/team-a/job/api-service/
integration-tests  pipeline     unstable  https://cloudbees.example.com/my-controller/job/team-a/job/integration-tests/

$ jenkins-cli builds list deploy -n 3
NUMBER  RESULT   STARTED               DURATION  URL
#42     SUCCESS  2025-10-25T13:46:40Z  1m 23s    https://cloudbees.example.com/my-controller/job/deploy/42/
#41     SUCCESS  2025-10-25T11:00:00Z  1m 19s    https://cloudbees.example.com/my-controller/job/deploy/41/
#40     FAILURE  2025-10-25T08:13:20Z  12s       https://cloudbees.example.com/my-controller/job/deploy/40/

$ jenkins-cli builds log team-a/integration-tests lastFailedBuild | tail -n 20

$ jenkins-cli -o json nodes list | jq '.computer[] | select(.offline) | .displayName'
```

The CLI exits with status 0 on success, 1 on errors (bad credentials,
missing resources, network problems) and 2 on usage errors.

Pointing the CLI at CloudBees CI Operations Center also works: `jobs list`
then lists the connected controllers as items of type `controller`.

## OpenAPI spec

Jenkins does not publish an OpenAPI document, so
[`openapi/jenkins.yaml`](openapi/jenkins.yaml) describes the endpoints this
CLI reads. Its examples are the test fixtures. Lint it with:

```sh
npx @redocly/cli lint openapi/jenkins.yaml
```

Two vendor extensions cover Jenkins details OpenAPI cannot express:

- `x-jenkins-job-path: true` on a path parameter means it can span folders
  (`team-a/job/api-service`).
- `x-mock-match` on an example limits it to requests with those path
  parameter values, so the mock can return a folder for `/job/team-a/` and a
  pipeline for other jobs.

## Tests

```sh
cargo test
```

- **Unit tests** in `src/` cover credentials parsing, URL building, build
  selectors and output formatting.
- **`tests/api.rs`** runs every command through the library against a mock
  server.
- **`tests/cli.rs`** runs the compiled binary against the mock server,
  including the credentials file, flags, environment variables and exit
  codes.

The mock server (`tests/support/mod.rs`) is built from the OpenAPI spec when
each test starts. It uses [wiremock](https://docs.rs/wiremock) to serve every
`GET` operation's `200` example, answers `401` when the basic-auth
credentials are missing or wrong, and `404` for paths the spec does not
describe. The tests also check that every example parses into the CLI's
models and that the commands together call every operation in the spec.

To try the CLI by hand against the same spec, run any OpenAPI mock server,
for example [Prism](https://github.com/stoplightio/prism):

```sh
npx @stoplight/prism-cli mock -p 4010 openapi/jenkins.yaml
JENKINS_URL=http://localhost:4010/ jenkins-cli builds list deploy
```

## Releasing

1. Set the new version in `Cargo.toml`, run `cargo build` to update
   `Cargo.lock`, and merge that change to `main`.
2. Tag the merge commit and push the tag:

   ```sh
   git tag v0.2.0
   git push origin v0.2.0
   ```

The [release workflow](.github/workflows/release.yml) then runs the CI
checks, and runs the tests and builds static (musl) binaries on native
amd64 and arm64 runners. It publishes a GitHub release with
`jenkins-cli-<tag>-linux-<arch>.tar.gz` archives and a `SHA256SUMS` file.
The workflow fails if the tag does not match the version in `Cargo.toml`.
A tag with a suffix, such as `v0.2.0-rc.1`, becomes a pre-release.

Pull requests that change the workflow, `Cargo.toml` or `Cargo.lock` run the
same builds without publishing anything.

## License

[AGPL-3.0](LICENSE)
