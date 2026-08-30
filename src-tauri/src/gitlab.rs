//! GitLab REST API v4 client.
//!
//! All HTTP traffic happens here, on the Rust side. The webview never sees
//! the token, and we don't have to deal with CORS.

use anyhow::{anyhow, Result};
use reqwest::{Client, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::Deserialize;

const USER_AGENT: &str = concat!("vitaline/", env!("CARGO_PKG_VERSION"));
/// Number of jobs fetched per page. For pipelines with more than this, the
/// last page gets cut off; in practice 100 jobs is plenty.
const JOBS_PER_PAGE: u32 = 100;
/// Number of lines shown from the tail of the log.
const TRACE_TAIL_LINES: usize = 200;
/// Number of open merge requests fetched at once.
const MR_PER_PAGE: u32 = 20;

pub struct Gitlab {
    http: Client,
    base: String,
    token: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiProject {
    pub name_with_namespace: String,
    pub web_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiUser {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiPipeline {
    pub id: u64,
    #[serde(rename = "ref", default)]
    pub git_ref: Option<String>,
    #[serde(default)]
    pub sha: Option<String>,
    pub status: String,
    #[serde(default)]
    pub source: Option<String>,
    pub web_url: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub user: Option<ApiUser>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiMergeRequest {
    pub iid: u64,
    pub title: String,
    #[serde(default)]
    pub author: Option<ApiUser>,
    #[serde(default)]
    pub source_branch: String,
    #[serde(default)]
    pub target_branch: String,
    pub web_url: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub draft: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiCommit {
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiJob {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub stage: String,
    pub status: String,
    #[serde(default)]
    pub allow_failure: bool,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub web_url: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub commit: Option<ApiCommit>,
    /// Only populated for records coming from the `/bridges` endpoint.
    #[serde(default)]
    pub downstream_pipeline: Option<ApiDownstream>,
}

/// The downstream (child/multi-project) pipeline triggered by a bridge job.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiDownstream {
    pub id: u64,
    pub status: String,
    #[serde(default)]
    pub web_url: Option<String>,
    #[serde(default, rename = "ref")]
    pub git_ref: Option<String>,
    /// The downstream pipeline can live in a different project (multi-project
    /// trigger); this numeric id is needed to fetch its jobs.
    #[serde(default)]
    pub project_id: Option<u64>,
}

impl Gitlab {
    pub fn new(http: Client, base: String, token: String) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            token,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v4{}", self.base, path)
    }

    /// Since the project id is also accepted as a path ("group/project"), we
    /// URL-encode it in full; `/` -> `%2F`.
    fn pid(id: &str) -> String {
        urlencoding::encode(id).into_owned()
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Response> {
        let resp = req
            .header("PRIVATE-TOKEN", &self.token)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .send()
            .await
            .map_err(|e| anyhow!("Could not reach {}: {}", self.base, friendly_transport(&e)))?;
        check(resp).await
    }

    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self.send(self.http.get(self.url(path))).await?;
        let body = resp.text().await?;
        serde_json::from_str(&body)
            .map_err(|e| anyhow!("Could not parse GitLab's response ({}): {}", path, e))
    }

    async fn post(&self, path: &str) -> Result<()> {
        self.send(self.http.post(self.url(path))).await?;
        Ok(())
    }

    /// Verifies the token and returns the username.
    pub async fn current_user(&self) -> Result<String> {
        let user: ApiUser = self.get_json("/user").await?;
        Ok(user
            .username
            .or(user.name)
            .unwrap_or_else(|| "(unnamed)".to_string()))
    }

    pub async fn project(&self, id: &str) -> Result<ApiProject> {
        self.get_json(&format!("/projects/{}", Self::pid(id))).await
    }

    /// Fetches the details of the given branch's pipeline (or the project's
    /// most recent one, if no branch is given).
    pub async fn latest_pipeline(
        &self,
        id: &str,
        git_ref: Option<&str>,
    ) -> Result<Option<ApiPipeline>> {
        let pid = Self::pid(id);

        if let Some(git_ref) = git_ref {
            // `pipelines/latest` returns full detail (duration, user) in one request.
            let path = format!(
                "/projects/{}/pipelines/latest?ref={}",
                pid,
                urlencoding::encode(git_ref)
            );
            return match self.get_json::<ApiPipeline>(&path).await {
                Ok(p) => Ok(Some(p)),
                // GitLab returns 403/404 if this branch has no pipeline at all.
                Err(e) if is_missing(&e) => Ok(None),
                Err(e) => Err(e),
            };
        }

        // No branch given: first find the most recent pipeline's id, then
        // fetch its detail (the list endpoint doesn't include duration/user).
        let list: Vec<ApiPipeline> = self
            .get_json(&format!(
                "/projects/{}/pipelines?per_page=1&order_by=id&sort=desc",
                pid
            ))
            .await?;
        let Some(head) = list.into_iter().next() else {
            return Ok(None);
        };
        match self
            .get_json::<ApiPipeline>(&format!("/projects/{}/pipelines/{}", pid, head.id))
            .await
        {
            Ok(p) => Ok(Some(p)),
            // If the detail fetch fails, the summary from the list is still useful.
            Err(_) => Ok(Some(head)),
        }
    }

    /// The pipeline's jobs, INCLUDING older retries (`include_retried=true`).
    ///
    /// Deliberate: a retried job gets a new, higher id. If we only fetched
    /// the current attempts, our heuristic for deriving stage order from job
    /// id would break -- a retried stage would jump to the end of the list.
    /// With older retries included, we have each job's FIRST attempt's id
    /// available, and the real flow order is preserved. Deduplication
    /// happens inside `group_stages`.
    pub async fn jobs(&self, id: &str, pipeline_id: u64) -> Result<Vec<ApiJob>> {
        self.get_json(&format!(
            "/projects/{}/pipelines/{}/jobs?per_page={}&include_retried=true",
            Self::pid(id),
            pipeline_id,
            JOBS_PER_PAGE
        ))
        .await
    }

    /// The pipeline's bridge (trigger) jobs and the downstream pipelines they trigger.
    ///
    /// NEEDED because `/pipelines/:id/jobs` never returns bridge jobs at
    /// all, so downstream/child pipelines used to be completely invisible in
    /// the UI. The returned records have the same shape as regular jobs, so
    /// they decode straight into `ApiJob` and get appended to the job list;
    /// the only difference is a populated `downstream_pipeline` field.
    pub async fn bridges(&self, id: &str, pipeline_id: u64) -> Result<Vec<ApiJob>> {
        self.get_json(&format!(
            "/projects/{}/pipelines/{}/bridges?per_page={}",
            Self::pid(id),
            pipeline_id,
            JOBS_PER_PAGE
        ))
        .await
    }

    /// The project's open merge requests, newest first.
    pub async fn open_merge_requests(&self, id: &str) -> Result<Vec<ApiMergeRequest>> {
        self.get_json(&format!(
            "/projects/{}/merge_requests?state=opened&order_by=created_at&sort=desc&per_page={}",
            Self::pid(id),
            MR_PER_PAGE
        ))
        .await
    }

    pub async fn job_trace(&self, id: &str, job_id: u64) -> Result<String> {
        let path = format!("/projects/{}/jobs/{}/trace", Self::pid(id), job_id);
        let resp = self.send(self.http.get(self.url(&path))).await?;
        let text = resp.text().await?;
        Ok(tail(&text, TRACE_TAIL_LINES))
    }

    pub async fn retry_pipeline(&self, id: &str, pipeline_id: u64) -> Result<()> {
        self.post(&format!(
            "/projects/{}/pipelines/{}/retry",
            Self::pid(id),
            pipeline_id
        ))
        .await
    }

    pub async fn cancel_pipeline(&self, id: &str, pipeline_id: u64) -> Result<()> {
        self.post(&format!(
            "/projects/{}/pipelines/{}/cancel",
            Self::pid(id),
            pipeline_id
        ))
        .await
    }

    pub async fn retry_job(&self, id: &str, job_id: u64) -> Result<()> {
        self.post(&format!(
            "/projects/{}/jobs/{}/retry",
            Self::pid(id),
            job_id
        ))
        .await
    }

    pub async fn cancel_job(&self, id: &str, job_id: u64) -> Result<()> {
        self.post(&format!(
            "/projects/{}/jobs/{}/cancel",
            Self::pid(id),
            job_id
        ))
        .await
    }

    pub async fn play_job(&self, id: &str, job_id: u64) -> Result<()> {
        self.post(&format!("/projects/{}/jobs/{}/play", Self::pid(id), job_id))
            .await
    }
}

/// An HTTP error returned by GitLab. The status code is kept so we can tell
/// "no pipeline on this branch" (404/403) apart from real errors.
#[derive(Debug)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

fn is_missing(err: &anyhow::Error) -> bool {
    err.downcast_ref::<ApiError>()
        .is_some_and(|e| e.status == 403 || e.status == 404)
}

async fn check(resp: Response) -> Result<Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }

    let body = resp.text().await.unwrap_or_default();
    let detail = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            v.get("message")
                .or_else(|| v.get("error"))
                .or_else(|| v.get("error_description"))
                .map(|m| {
                    m.as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| m.to_string())
                })
        })
        .unwrap_or_else(|| body.chars().take(160).collect());

    let hint = match status {
        StatusCode::UNAUTHORIZED => " - token is invalid or expired",
        StatusCode::FORBIDDEN => " - token lacks permission (needs read_api/api scope)",
        StatusCode::NOT_FOUND => " - project/pipeline doesn't exist or the token can't see it",
        StatusCode::TOO_MANY_REQUESTS => " - GitLab rate limit; increase the refresh interval",
        _ => "",
    };

    Err(anyhow::Error::new(ApiError {
        status: status.as_u16(),
        message: format!("GitLab {}{}: {}", status.as_u16(), hint, detail),
    }))
}

fn friendly_transport(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        "timed out".to_string()
    } else if err.is_connect() {
        "could not connect (check the address or VPN?)".to_string()
    } else {
        err.to_string()
    }
}

/// Returns the last `n` lines of the text.
pub(crate) fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= n {
        return text.to_string();
    }
    let skipped = lines.len() - n;
    let mut out = format!("… first {} lines skipped …\n", skipped);
    out.push_str(&lines[skipped..].join("\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_stays_as_is() {
        assert_eq!(tail("a\nb", 5), "a\nb");
    }

    #[test]
    fn long_text_is_truncated_to_tail() {
        let text = (1..=10)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let out = tail(&text, 3);
        assert!(out.starts_with("… first 7 lines skipped …"));
        assert!(out.ends_with("8\n9\n10"));
    }

    #[test]
    fn project_path_is_encoded() {
        assert_eq!(Gitlab::pid("group/sub/project"), "group%2Fsub%2Fproject");
        assert_eq!(Gitlab::pid("12345"), "12345");
    }
}
