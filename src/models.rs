//! Typed views of the Jenkins JSON responses, matching the schemas in
//! `openapi/jenkins.yaml`. Every field has a default because the `tree`
//! query parameter, permissions and plugins all change which fields Jenkins
//! returns.

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ItemRef {
    #[serde(rename = "_class")]
    pub class: String,
    pub name: String,
    pub url: String,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BuildRef {
    pub number: u64,
    pub url: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Controller {
    pub mode: String,
    pub node_description: String,
    pub num_executors: u32,
    pub description: Option<String>,
    pub quieting_down: bool,
    pub use_security: bool,
    pub url: String,
    pub jobs: Vec<ItemRef>,
    pub views: Vec<ItemRef>,
    pub primary_view: Option<ItemRef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WhoAmI {
    pub name: String,
    pub anonymous: bool,
    pub authenticated: bool,
    pub authorities: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HealthReport {
    pub description: String,
    pub score: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Job {
    #[serde(rename = "_class")]
    pub class: String,
    pub name: String,
    pub full_name: String,
    pub display_name: String,
    pub description: Option<String>,
    pub url: String,
    pub buildable: Option<bool>,
    pub color: Option<String>,
    pub in_queue: Option<bool>,
    pub next_build_number: Option<u64>,
    pub health_report: Vec<HealthReport>,
    pub last_build: Option<BuildRef>,
    pub last_successful_build: Option<BuildRef>,
    pub last_failed_build: Option<BuildRef>,
    pub builds: Vec<BuildSummary>,
    pub jobs: Vec<ItemRef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BuildSummary {
    pub number: u64,
    pub url: String,
    pub result: Option<String>,
    pub building: bool,
    pub timestamp: i64,
    pub duration: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Cause {
    pub short_description: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Action {
    pub causes: Vec<Cause>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Artifact {
    pub file_name: String,
    pub relative_path: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Build {
    pub number: u64,
    pub full_display_name: String,
    pub description: Option<String>,
    pub url: String,
    pub building: bool,
    pub result: Option<String>,
    pub timestamp: i64,
    pub duration: i64,
    pub estimated_duration: i64,
    // Jenkins returns `null` or `{}` for actions it cannot serialize.
    #[serde(deserialize_with = "skip_nulls")]
    pub actions: Vec<Action>,
    pub artifacts: Vec<Artifact>,
}

impl Build {
    pub fn causes(&self) -> impl Iterator<Item = &str> {
        self.actions
            .iter()
            .flat_map(|a| &a.causes)
            .map(|c| c.short_description.as_str())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QueueItem {
    #[serde(rename = "_class")]
    pub class: String,
    pub id: u64,
    pub blocked: bool,
    pub buildable: bool,
    pub stuck: bool,
    pub cancelled: Option<bool>,
    pub in_queue_since: i64,
    pub why: Option<String>,
    pub task: ItemRef,
    pub executable: Option<BuildRef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Queue {
    pub items: Vec<QueueItem>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Label {
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Computer {
    pub display_name: String,
    pub description: Option<String>,
    pub idle: bool,
    pub offline: bool,
    pub temporarily_offline: bool,
    pub offline_cause_reason: Option<String>,
    pub num_executors: u32,
    pub assigned_labels: Vec<Label>,
}

impl Computer {
    pub fn status(&self) -> &'static str {
        match (self.offline, self.idle) {
            (true, _) => "offline",
            (false, true) => "idle",
            (false, false) => "busy",
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ComputerSet {
    pub busy_executors: u32,
    pub total_executors: u32,
    pub computer: Vec<Computer>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct View {
    pub name: String,
    pub description: Option<String>,
    pub url: String,
    pub jobs: Vec<ItemRef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Plugin {
    pub short_name: String,
    pub long_name: String,
    pub version: String,
    pub active: bool,
    pub enabled: bool,
    pub has_update: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PluginManager {
    pub plugins: Vec<Plugin>,
}

fn skip_nulls<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    let items = Option::<Vec<Option<T>>>::deserialize(deserializer)?;
    Ok(items.into_iter().flatten().flatten().collect())
}

/// Short, readable kind of a Jenkins item, from its Java class name.
pub fn item_kind(class: &str) -> &str {
    let short = class.rsplit(['.', '$']).next().unwrap_or(class);
    match short {
        "WorkflowJob" => "pipeline",
        "FreeStyleProject" => "freestyle",
        "Folder" => "folder",
        "WorkflowMultiBranchProject" => "multibranch",
        "OrganizationFolder" => "org-folder",
        "MatrixProject" => "matrix",
        "MavenModuleSet" => "maven",
        "ExternalJob" => "external",
        // CloudBees CI Operations Center items.
        "ManagedMaster" | "ConnectedMaster" => "controller",
        "" => "-",
        other => other,
    }
}

/// Readable status from a job's `color` field.
pub fn color_status(color: Option<&str>) -> String {
    let Some(color) = color else {
        return "-".to_string();
    };
    let (base, running) = match color.strip_suffix("_anime") {
        Some(base) => (base, true),
        None => (color, false),
    };
    let status = match base {
        "blue" => "success",
        "red" => "failure",
        "yellow" => "unstable",
        "grey" | "notbuilt" => "not built",
        "disabled" => "disabled",
        "aborted" => "aborted",
        other => other,
    };
    if running {
        format!("{status} (building)")
    } else {
        status.to_string()
    }
}

/// A build number or a permalink such as `lastBuild`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildSelector {
    Number(u64),
    Permalink(&'static str),
}

impl BuildSelector {
    pub const PERMALINKS: [&'static str; 7] = [
        "lastBuild",
        "lastCompletedBuild",
        "lastSuccessfulBuild",
        "lastStableBuild",
        "lastUnstableBuild",
        "lastFailedBuild",
        "lastUnsuccessfulBuild",
    ];
}

impl FromStr for BuildSelector {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(number) = s.parse() {
            return Ok(Self::Number(number));
        }
        if s == "last" {
            return Ok(Self::Permalink("lastBuild"));
        }
        Self::PERMALINKS
            .iter()
            .find(|p| p.eq_ignore_ascii_case(s))
            .map(|p| Self::Permalink(p))
            .ok_or_else(|| {
                format!(
                    "expected a build number or one of: last, {}",
                    Self::PERMALINKS.join(", ")
                )
            })
    }
}

impl fmt::Display for BuildSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(n) => write!(f, "{n}"),
            Self::Permalink(p) => f.write_str(p),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_selector_parses_numbers_and_permalinks() {
        assert_eq!("42".parse(), Ok(BuildSelector::Number(42)));
        assert_eq!("last".parse(), Ok(BuildSelector::Permalink("lastBuild")));
        assert_eq!(
            "lastsuccessfulbuild".parse(),
            Ok(BuildSelector::Permalink("lastSuccessfulBuild"))
        );
        assert!("-1".parse::<BuildSelector>().is_err());
        assert!("latest".parse::<BuildSelector>().is_err());
    }

    #[test]
    fn item_kinds() {
        assert_eq!(
            item_kind("org.jenkinsci.plugins.workflow.job.WorkflowJob"),
            "pipeline"
        );
        assert_eq!(
            item_kind("com.cloudbees.hudson.plugins.folder.Folder"),
            "folder"
        );
        assert_eq!(
            item_kind("com.cloudbees.opscenter.server.model.ManagedMaster"),
            "controller"
        );
        assert_eq!(item_kind("com.example.CustomJob"), "CustomJob");
        assert_eq!(item_kind(""), "-");
    }

    #[test]
    fn color_statuses() {
        assert_eq!(color_status(Some("blue")), "success");
        assert_eq!(color_status(Some("red_anime")), "failure (building)");
        assert_eq!(color_status(Some("notbuilt")), "not built");
        assert_eq!(color_status(None), "-");
    }

    #[test]
    fn build_skips_null_and_empty_actions() {
        let build: Build = serde_json::from_str(
            r#"{"number": 1, "actions": [null, {}, {"causes": [{"shortDescription": "Started by timer"}]}]}"#,
        )
        .unwrap();
        assert_eq!(build.causes().collect::<Vec<_>>(), ["Started by timer"]);
    }

    #[test]
    fn computer_status() {
        let mut c = Computer::default();
        assert_eq!(c.status(), "busy");
        c.idle = true;
        assert_eq!(c.status(), "idle");
        c.offline = true;
        assert_eq!(c.status(), "offline");
    }
}
