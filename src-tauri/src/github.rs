//! GitHub Actions REST client.
//!
//! The project id has the shape "owner/repo". GitHub has no stage concept;
//! a workflow run's jobs are shown grouped under a single "workflow" bucket.
//! Status is split across two fields (status + conclusion); `normalize`
//! collapses them into our canonical vocabulary.

use anyhow::{anyhow, Result};
use reqwest::{Client, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::model::{aggregate, JobInfo, MergeRequestInfo, PipelineInfo, StageInfo};

const USER_AGENT: &str = concat!("vitaline/", env!("CARGO_PKG_VERSION"));
const API_VERSION: &str = "2022-11-28";
const TRACE_TAIL_LINES: usize = 200;
const PR_PER_PAGE: u32 = 20;

pub struct Github {
    http: Client,
    /// https://api.github.com for github.com; https://host/api/v3 for GHES
    base: String,
    token: String,
}

#[derive(Debug, Deserialize)]
struct ApiUser {
    login: String,
}

#[derive(Debug, Deserialize)]
struct ApiRepo {
    full_name: String,
    html_url: String,
}

#[derive(Debug, Deserialize)]
struct RunsPage {
    workflow_runs: Vec<ApiRun>,
}

#[derive(Debug, Deserialize)]
struct ApiActor {
    login: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiRun {
    id: u64,
    #[serde(default)]
    display_title: Option<String>,
    #[serde(default)]
    head_branch: Option<String>,
    #[serde(default)]
    head_sha: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    html_url: String,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    run_started_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    actor: Option<ApiActor>,
}

#[derive(Debug, Deserialize)]
struct JobsPage {
    jobs: Vec<ApiJob>,
}

#[derive(Debug, Deserialize)]
struct ApiJob {
    id: u64,
    name: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    completed_at: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PrUser {
    login: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PrRef {
    #[serde(rename = "ref")]
    git_ref: String,
}

#[derive(Debug, Deserialize)]
struct ApiPr {
    number: u64,
    title: String,
    #[serde(default)]
    user: Option<PrUser>,
    head: PrRef,
    base: PrRef,
    html_url: String,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    draft: bool,
}

/// Converts GitHub's (status, conclusion) pair into our canonical status.
pub fn normalize(status: Option<&str>, conclusion: Option<&str>) -> String {
    let result = match status.unwrap_or("") {
        "queued" | "waiting" | "requested" | "pending" => "pending",
        "in_progress" => "running",
        "completed" => match conclusion.unwrap_or("") {
            "success" | "neutral" => "success",
            "failure" | "timed_out" | "startup_failure" => "failed",
            "cancelled" => "canceled",
            "skipped" | "stale" => "skipped",
            "action_required" => "manual",
            _ => "unknown",
        },
        _ => "unknown",
    };
    result.to_string()
}

fn secs_between(start: Option<&str>, end: Option<&str>) -> Option<f64> {
    let start = chrono::DateTime::parse_from_rfc3339(start?).ok()?;
    let end = chrono::DateTime::parse_from_rfc3339(end?).ok()?;
    let secs = (end - start).num_seconds();
    (secs >= 0).then_some(secs as f64)
}

impl Github {
    pub fn new(http: Client, base: String, token: String) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            token,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Response> {
        let resp = req
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.token),
            )
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", API_VERSION)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .send()
            .await
            .map_err(|e| anyhow!("Could not reach {}: {}", self.base, e))?;
        check(resp).await
    }

    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self.send(self.http.get(self.url(path))).await?;
        let body = resp.text().await?;
        serde_json::from_str(&body)
            .map_err(|e| anyhow!("Could not parse GitHub's response ({}): {}", path, e))
    }

    async fn post(&self, path: &str) -> Result<()> {
        self.send(self.http.post(self.url(path))).await?;
        Ok(())
    }

    pub async fn current_user(&self) -> Result<String> {
        let user: ApiUser = self.get_json("/user").await?;
        Ok(user.login)
    }

    /// Latest workflow run + jobs + open PRs.
    pub async fn fetch(
        &self,
        repo: &str,
        git_ref: Option<&str>,
        watch_prs: bool,
    ) -> Result<(Option<PipelineInfo>, Vec<MergeRequestInfo>)> {
        let repo_path = validate_repo(repo)?;
        let meta: ApiRepo = self.get_json(&format!("/repos/{repo_path}")).await?;

        let prs = if watch_prs {
            self.open_prs(&repo_path).await.unwrap_or_default()
        } else {
            Vec::new()
        };

        let branch_query = git_ref
            .map(|r| format!("&branch={}", urlencoding::encode(r)))
            .unwrap_or_default();
        let page: RunsPage = self
            .get_json(&format!(
                "/repos/{repo_path}/actions/runs?per_page=1{branch_query}"
            ))
            .await?;
        let Some(run) = page.workflow_runs.into_iter().next() else {
            return Ok((None, prs));
        };

        // If the job list can't be fetched, the run is still shown, just ungrouped.
        let jobs = self
            .get_json::<JobsPage>(&format!(
                "/repos/{repo_path}/actions/runs/{}/jobs?per_page=100",
                run.id
            ))
            .await
            .map(|p| p.jobs)
            .unwrap_or_default();

        let job_infos: Vec<JobInfo> = jobs
            .iter()
            .map(|job| JobInfo {
                id: job.id,
                name: job.name.clone(),
                stage: "workflow".to_string(),
                status: normalize(job.status.as_deref(), job.conclusion.as_deref()),
                allow_failure: false,
                duration: secs_between(job.started_at.as_deref(), job.completed_at.as_deref()),
                web_url: job.html_url.clone().unwrap_or_default(),
                finished_at: job.completed_at.clone(),
                // The downstream concept is GitLab-specific (bridge jobs).
                downstream: None,
            })
            .collect();

        let stages = if job_infos.is_empty() {
            Vec::new()
        } else {
            let status = aggregate(
                job_infos
                    .iter()
                    .map(|j| (j.status.as_str(), j.allow_failure)),
            );
            vec![StageInfo {
                name: "workflow".to_string(),
                status,
                jobs: job_infos,
            }]
        };

        let status = normalize(run.status.as_deref(), run.conclusion.as_deref());
        // Duration for a finished run: start -> last update. The GitHub run
        // object has no direct duration field; this is a reasonable approximation.
        let duration = if status != "running" && status != "pending" {
            secs_between(run.run_started_at.as_deref(), run.updated_at.as_deref())
        } else {
            None
        };

        Ok((
            Some(PipelineInfo {
                project_id: repo.to_string(),
                project_name: meta.full_name,
                project_url: meta.html_url,
                id: run.id,
                git_ref: run.head_branch.unwrap_or_else(|| "-".to_string()),
                sha: run.head_sha.unwrap_or_default(),
                status,
                source: run.event,
                web_url: run.html_url,
                created_at: run.created_at,
                duration,
                triggered_by: run.actor.and_then(|a| a.login),
                commit_title: run.display_title,
                stages,
            }),
            prs,
        ))
    }

    async fn open_prs(&self, repo_path: &str) -> Result<Vec<MergeRequestInfo>> {
        let prs: Vec<ApiPr> = self
            .get_json(&format!(
                "/repos/{repo_path}/pulls?state=open&per_page={PR_PER_PAGE}"
            ))
            .await?;
        Ok(prs
            .into_iter()
            .map(|pr| MergeRequestInfo {
                iid: pr.number,
                title: pr.title,
                author: pr.user.and_then(|u| u.login),
                source_branch: pr.head.git_ref,
                target_branch: pr.base.git_ref,
                web_url: pr.html_url,
                created_at: pr.created_at,
                draft: pr.draft,
            })
            .collect())
    }

    pub async fn rerun(&self, repo: &str, run_id: u64) -> Result<()> {
        let repo_path = validate_repo(repo)?;
        self.post(&format!("/repos/{repo_path}/actions/runs/{run_id}/rerun"))
            .await
    }

    pub async fn cancel(&self, repo: &str, run_id: u64) -> Result<()> {
        let repo_path = validate_repo(repo)?;
        self.post(&format!("/repos/{repo_path}/actions/runs/{run_id}/cancel"))
            .await
    }

    pub async fn rerun_job(&self, repo: &str, job_id: u64) -> Result<()> {
        let repo_path = validate_repo(repo)?;
        self.post(&format!("/repos/{repo_path}/actions/jobs/{job_id}/rerun"))
            .await
    }

    /// Job log comes back as plain text (reqwest follows the 302 redirect itself).
    pub async fn job_trace(&self, repo: &str, job_id: u64) -> Result<String> {
        let repo_path = validate_repo(repo)?;
        let resp = self
            .send(
                self.http
                    .get(self.url(&format!("/repos/{repo_path}/actions/jobs/{job_id}/logs"))),
            )
            .await?;
        let text = resp.text().await?;
        Ok(crate::gitlab::tail(&text, TRACE_TAIL_LINES))
    }
}

fn validate_repo(repo: &str) -> Result<String> {
    let mut parts = repo.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(name), None) if !owner.is_empty() && !name.is_empty() => {
            Ok(format!("{owner}/{name}"))
        }
        _ => Err(anyhow!(
            "GitHub project id must have the form \"owner/repo\", got: {repo}"
        )),
    }
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
                .and_then(|m| m.as_str().map(str::to_string))
        })
        .unwrap_or_else(|| body.chars().take(160).collect());
    let hint = match status {
        StatusCode::UNAUTHORIZED => " - token is invalid or expired",
        StatusCode::FORBIDDEN => {
            " - token lacks permission (needs repo/actions scope) or rate limited"
        }
        StatusCode::NOT_FOUND => " - repo doesn't exist or the token can't see it",
        _ => "",
    };
    Err(anyhow!("GitHub {}{}: {}", status.as_u16(), hint, detail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_normalization() {
        assert_eq!(normalize(Some("in_progress"), None), "running");
        assert_eq!(normalize(Some("queued"), None), "pending");
        assert_eq!(normalize(Some("completed"), Some("success")), "success");
        assert_eq!(normalize(Some("completed"), Some("failure")), "failed");
        assert_eq!(normalize(Some("completed"), Some("cancelled")), "canceled");
        assert_eq!(normalize(Some("completed"), Some("skipped")), "skipped");
        assert_eq!(
            normalize(Some("completed"), Some("action_required")),
            "manual"
        );
        assert_eq!(normalize(Some("completed"), Some("timed_out")), "failed");
        assert_eq!(normalize(None, None), "unknown");
    }

    #[test]
    fn repo_id_is_validated() {
        assert!(validate_repo("owner/repo").is_ok());
        assert!(validate_repo("owner-only").is_err());
        assert!(validate_repo("a/b/c").is_err());
        assert!(validate_repo("/repo").is_err());
    }

    #[test]
    fn duration_calculation() {
        assert_eq!(
            secs_between(Some("2026-08-28T10:00:00Z"), Some("2026-08-28T10:02:30Z")),
            Some(150.0)
        );
        assert_eq!(secs_between(None, Some("2026-08-28T10:00:00Z")), None);
    }
}
