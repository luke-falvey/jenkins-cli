//! A small read-only client for the Jenkins Remote Access API.

use std::time::Duration;

use reqwest::StatusCode;
use serde_json::Value;
use thiserror::Error;
use url::Url;

use crate::credentials::Credentials;
use crate::models::BuildSelector;

/// Fields requested when listing the jobs in a folder or at the top level.
pub const JOBS_TREE: &str = "jobs[_class,name,url,color]";
/// Fields requested when listing views.
pub const VIEWS_TREE: &str = "views[_class,name,url]";
/// Fields requested when listing nodes.
pub const COMPUTERS_TREE: &str = "busyExecutors,totalExecutors,computer[_class,displayName,description,idle,offline,temporarilyOffline,offlineCauseReason,numExecutors,assignedLabels[name]]";
/// Fields requested when listing plugins.
pub const PLUGINS_TREE: &str = "plugins[shortName,longName,version,active,enabled,hasUpdate]";

/// Fields requested when listing the most recent `limit` builds of a job.
pub fn builds_tree(limit: u32) -> String {
    format!("builds[_class,number,url,result,building,timestamp,duration]{{0,{limit}}}")
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("invalid Jenkins URL `{url}`: {reason}")]
    InvalidUrl { url: String, reason: String },
    #[error(
        "authentication failed (HTTP 401) for user `{username}`; check the username and API token in your credentials file"
    )]
    Unauthorized { username: String },
    #[error("permission denied (HTTP 403): user `{username}` cannot read {url}")]
    Forbidden { username: String, url: Url },
    #[error("not found (HTTP 404): {url}")]
    NotFound { url: Url },
    #[error("unexpected HTTP {status} from {url}")]
    Status { status: StatusCode, url: Url },
    #[error("request to {url} failed")]
    Request {
        url: Url,
        #[source]
        source: reqwest::Error,
    },
    #[error("could not read the response from {url} as JSON")]
    Decode {
        url: Url,
        #[source]
        source: serde_json::Error,
    },
}

pub type Result<T> = std::result::Result<T, ClientError>;

/// Read-only Jenkins client that authenticates with HTTP basic auth.
#[derive(Debug, Clone)]
pub struct JenkinsClient {
    http: reqwest::Client,
    base: Url,
    credentials: Credentials,
}

impl JenkinsClient {
    /// `base_url` is the root of a Jenkins instance or CloudBees CI controller,
    /// for example `https://cloudbees.example.com/my-controller/`.
    pub fn new(base_url: &str, credentials: Credentials, timeout: Duration) -> Result<Self> {
        let invalid = |reason: String| ClientError::InvalidUrl {
            url: base_url.to_string(),
            reason,
        };
        let mut base = Url::parse(base_url).map_err(|e| invalid(e.to_string()))?;
        if !matches!(base.scheme(), "http" | "https") {
            return Err(invalid("the scheme must be http or https".into()));
        }
        if !base.username().is_empty() || base.password().is_some() {
            return Err(invalid(
                "do not put credentials in the URL; they are read from the credentials file".into(),
            ));
        }
        base.set_query(None);
        base.set_fragment(None);
        // Drop the trailing empty segment so paths can be appended cleanly.
        base.path_segments_mut()
            .map_err(|()| invalid("the URL cannot have a path".into()))?
            .pop_if_empty();

        let http = reqwest::Client::builder()
            .timeout(timeout)
            .user_agent(concat!("jenkins-cli/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|source| ClientError::Request {
                url: base.clone(),
                source,
            })?;
        Ok(Self {
            http,
            base,
            credentials,
        })
    }

    pub fn base_url(&self) -> &Url {
        &self.base
    }

    /// Builds a URL under the base URL. Each segment is percent-encoded.
    pub fn url<'a>(&self, segments: impl IntoIterator<Item = &'a str>) -> Url {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .expect("base URL was checked in new()")
            .extend(segments);
        url
    }

    /// URL of a job, given its full name such as `team-a/api-service`.
    pub fn job_url(&self, job_path: &str, rest: &[&str]) -> Url {
        self.url(job_segments(job_path).chain(rest.iter().copied()))
    }

    async fn send(&self, mut url: Url, query: &[(&str, &str)]) -> Result<reqwest::Response> {
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let response = self
            .http
            .get(url.clone())
            .basic_auth(&self.credentials.username, Some(&self.credentials.password))
            .send()
            .await
            .map_err(|source| ClientError::Request {
                url: url.clone(),
                source,
            })?;
        let username = self.credentials.username.clone();
        match response.status() {
            status if status.is_success() => Ok(response),
            StatusCode::UNAUTHORIZED => Err(ClientError::Unauthorized { username }),
            StatusCode::FORBIDDEN => Err(ClientError::Forbidden { username, url }),
            StatusCode::NOT_FOUND => Err(ClientError::NotFound { url }),
            status => Err(ClientError::Status { status, url }),
        }
    }

    /// GETs a URL and parses the body as JSON.
    pub async fn get_json(&self, url: Url, query: &[(&str, &str)]) -> Result<Value> {
        let response = self.send(url.clone(), query).await?;
        let body = response
            .bytes()
            .await
            .map_err(|source| ClientError::Request {
                url: url.clone(),
                source,
            })?;
        serde_json::from_slice(&body).map_err(|source| ClientError::Decode { url, source })
    }

    /// GETs a URL and returns the body as text.
    pub async fn get_text(&self, url: Url) -> Result<String> {
        let response = self.send(url.clone(), &[]).await?;
        response
            .text()
            .await
            .map_err(|source| ClientError::Request { url, source })
    }

    fn tree_query(tree: Option<&str>) -> Vec<(&str, &str)> {
        tree.map(|t| vec![("tree", t)]).unwrap_or_default()
    }

    /// `GET /whoAmI/api/json`
    pub async fn who_am_i(&self) -> Result<Value> {
        self.get_json(self.url(["whoAmI", "api", "json"]), &[])
            .await
    }

    /// `GET /api/json`
    pub async fn controller(&self, tree: Option<&str>) -> Result<Value> {
        self.get_json(self.url(["api", "json"]), &Self::tree_query(tree))
            .await
    }

    /// `GET /job/{jobPath}/api/json`
    pub async fn job(&self, job_path: &str, tree: Option<&str>) -> Result<Value> {
        self.get_json(
            self.job_url(job_path, &["api", "json"]),
            &Self::tree_query(tree),
        )
        .await
    }

    /// `GET /job/{jobPath}/{buildNumber}/api/json`
    pub async fn build(&self, job_path: &str, build: &BuildSelector) -> Result<Value> {
        let build = build.to_string();
        self.get_json(self.job_url(job_path, &[&build, "api", "json"]), &[])
            .await
    }

    /// `GET /job/{jobPath}/{buildNumber}/consoleText`
    pub async fn console_text(&self, job_path: &str, build: &BuildSelector) -> Result<String> {
        let build = build.to_string();
        self.get_text(self.job_url(job_path, &[&build, "consoleText"]))
            .await
    }

    /// `GET /queue/api/json`
    pub async fn queue(&self) -> Result<Value> {
        self.get_json(self.url(["queue", "api", "json"]), &[]).await
    }

    /// `GET /queue/item/{queueId}/api/json`
    pub async fn queue_item(&self, id: u64) -> Result<Value> {
        let id = id.to_string();
        self.get_json(self.url(["queue", "item", &id, "api", "json"]), &[])
            .await
    }

    /// `GET /computer/api/json`
    pub async fn computers(&self) -> Result<Value> {
        self.get_json(
            self.url(["computer", "api", "json"]),
            &[("tree", COMPUTERS_TREE)],
        )
        .await
    }

    /// `GET /computer/{nodeName}/api/json`
    pub async fn computer(&self, name: &str) -> Result<Value> {
        self.get_json(self.url(["computer", name, "api", "json"]), &[])
            .await
    }

    /// `GET /view/{viewName}/api/json`
    pub async fn view(&self, name: &str) -> Result<Value> {
        self.get_json(self.url(["view", name, "api", "json"]), &[])
            .await
    }

    /// `GET /pluginManager/api/json`
    pub async fn plugins(&self) -> Result<Value> {
        self.get_json(
            self.url(["pluginManager", "api", "json"]),
            &[("tree", PLUGINS_TREE)],
        )
        .await
    }
}

/// Turns `team-a/api-service` into `job/team-a/job/api-service`.
///
/// Empty parts (from leading, trailing or doubled slashes) are ignored.
pub fn job_segments(job_path: &str) -> impl Iterator<Item = &str> {
    job_path
        .split('/')
        .filter(|part| !part.is_empty())
        .flat_map(|part| ["job", part])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(base: &str) -> JenkinsClient {
        JenkinsClient::new(
            base,
            Credentials::new("alice", "token"),
            Duration::from_secs(5),
        )
        .unwrap()
    }

    #[test]
    fn builds_urls_with_or_without_trailing_slash() {
        for base in [
            "https://cb.example.com/my-controller",
            "https://cb.example.com/my-controller/",
        ] {
            assert_eq!(
                client(base).url(["api", "json"]).as_str(),
                "https://cb.example.com/my-controller/api/json"
            );
        }
        assert_eq!(
            client("https://jenkins.example.com")
                .url(["api", "json"])
                .as_str(),
            "https://jenkins.example.com/api/json"
        );
    }

    #[test]
    fn builds_job_urls_through_folders() {
        let c = client("https://cb.example.com/c/");
        assert_eq!(
            c.job_url("/team-a//api-service/", &["42", "api", "json"])
                .as_str(),
            "https://cb.example.com/c/job/team-a/job/api-service/42/api/json"
        );
    }

    #[test]
    fn encodes_names() {
        let c = client("https://cb.example.com/");
        assert_eq!(
            c.job_url("my folder/feature%2Fx", &["api", "json"])
                .as_str(),
            "https://cb.example.com/job/my%20folder/job/feature%252Fx/api/json"
        );
        assert_eq!(
            c.url(["view", "a/b?c", "api", "json"]).as_str(),
            "https://cb.example.com/view/a%2Fb%3Fc/api/json"
        );
    }

    #[test]
    fn drops_query_and_fragment_from_base() {
        assert_eq!(
            client("https://cb.example.com/c/?x=1#top")
                .url(["api", "json"])
                .as_str(),
            "https://cb.example.com/c/api/json"
        );
    }

    #[test]
    fn rejects_bad_base_urls() {
        let creds = Credentials::new("a", "b");
        let t = Duration::from_secs(1);
        for bad in [
            "not a url",
            "ftp://cb.example.com/",
            "https://bob:pw@cb.example.com/",
            "mailto:me@example.com",
        ] {
            let err = JenkinsClient::new(bad, creds.clone(), t).unwrap_err();
            assert!(
                matches!(err, ClientError::InvalidUrl { .. }),
                "{bad}: {err}"
            );
        }
    }

    #[test]
    fn builds_tree_has_range() {
        assert_eq!(
            builds_tree(5),
            "builds[_class,number,url,result,building,timestamp,duration]{0,5}"
        );
    }
}
