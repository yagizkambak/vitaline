//! Azure DevOps (Pipelines) REST client.
//!
//! The project id has the shape "Project" (the project's most recent build)
//! or "Project/DefinitionId" (a specific pipeline definition). The
//! organization URL comes from settings: https://dev.azure.com/organization
//!
//! The stage/job tree is read from the Timeline API: records arrive in a
//! Stage -> Phase -> Job -> Task hierarchy; we attach Job records to their
//! Stage by walking the parentId chain. The log id is used as the job's
//! numeric id (record ids are GUIDs; the log id is already what `job_trace`
//! needs).

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use base64::Engine;
use reqwest::{Client, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::model::{aggregate, JobInfo, MergeRequestInfo, PipelineInfo, StageInfo};

const USER_AGENT: &str = concat!("vitaline/", env!("CARGO_PKG_VERSION"));
const API: &str = "api-version=7.1";
/// `_apis/connectionData` never graduated out of preview even at 7.1 --
/// calling it with the plain `api-version=7.1` returns a 400 ("under
/// preview, the -preview flag must be supplied"). Every other endpoint we
/// call (builds, timeline, pull requests) is stable at 7.1 and uses `API`
/// above; this exists solely for `current_user`.
const API_PREVIEW: &str = "api-version=7.1-preview.1";
const TRACE_TAIL_LINES: usize = 200;
const PR_TOP: u32 = 20;

pub struct Azure {
    http: Client,
    /// https://dev.azure.com/organization
    org: String,
    token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionData {
    authenticated_user: Option<AuthUser>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthUser {
    provider_display_name: Option<String>,
    custom_display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Page<T> {
    value: Vec<T>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiBuild {
    id: u64,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    source_branch: Option<String>,
    #[serde(default)]
    source_version: Option<String>,
    #[serde(default)]
    queue_time: Option<String>,
    #[serde(default)]
    start_time: Option<String>,
    #[serde(default)]
    finish_time: Option<String>,
    #[serde(default)]
    requested_for: Option<IdentityRef>,
    #[serde(default)]
    definition: Option<DefinitionRef>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(rename = "_links", default)]
    links: Option<Links>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdentityRef {
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DefinitionRef {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Links {
    web: Option<Href>,
}

#[derive(Debug, Deserialize)]
struct Href {
    href: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Timeline {
    records: Vec<Record>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub order: Option<u64>,
    #[serde(default)]
    pub log: Option<LogRef>,
    #[serde(default)]
    pub start_time: Option<String>,
    #[serde(default)]
    pub finish_time: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogRef {
    pub id: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiPr {
    pull_request_id: u64,
    title: String,
    #[serde(default)]
    created_by: Option<IdentityRef>,
    #[serde(default)]
    source_ref_name: Option<String>,
    #[serde(default)]
    target_ref_name: Option<String>,
    #[serde(default)]
    creation_date: Option<String>,
    #[serde(default)]
    is_draft: bool,
    #[serde(default)]
    repository: Option<PrRepo>,
}

#[derive(Debug, Deserialize)]
struct PrRepo {
    name: Option<String>,
}

/// Converts Azure's (state, result) pair into our canonical status.
pub fn normalize(state: Option<&str>, result: Option<&str>) -> String {
    let out = match state.unwrap_or("") {
        "inProgress" => "running",
        "cancelling" => "canceling",
        "pending" | "notStarted" | "postponed" | "none" => "pending",
        "completed" => match result.unwrap_or("") {
            "succeeded" => "success",
            // Succeeded with warnings: counts as success, like GitLab's allow_failure.
            "succeededWithIssues" | "partiallySucceeded" => "success",
            "failed" => "failed",
            "canceled" | "abandoned" => "canceled",
            "skipped" => "skipped",
            _ => "unknown",
        },
        _ => "unknown",
    };
    out.to_string()
}

fn secs_between(start: Option<&str>, end: Option<&str>) -> Option<f64> {
    let start = chrono::DateTime::parse_from_rfc3339(start?).ok()?;
    let end = chrono::DateTime::parse_from_rfc3339(end?).ok()?;
    let secs = (end - start).num_seconds();
    (secs >= 0).then_some(secs as f64)
}

/// "refs/heads/main" -> "main"
fn short_ref(full: &str) -> String {
    full.trim_start_matches("refs/heads/").to_string()
}

/// Builds the stage list from timeline records. Job records are attached to
/// their nearest Stage ancestor by walking the parentId chain (Job -> Phase
/// -> Stage); on older single-stage pipelines with no Stage record,
/// everything falls under "(pipeline)".
pub fn stages_from_timeline(records: &[Record]) -> Vec<StageInfo> {
    let by_id: HashMap<&str, &Record> = records.iter().map(|r| (r.id.as_str(), r)).collect();

    let stage_of = |record: &Record| -> Option<&Record> {
        let mut current = record;
        for _ in 0..8 {
            let parent = by_id.get(current.parent_id.as_deref()?)?;
            if parent.kind == "Stage" {
                return Some(parent);
            }
            current = parent;
        }
        None
    };

    let mut stages: Vec<&Record> = records.iter().filter(|r| r.kind == "Stage").collect();
    stages.sort_by_key(|s| s.order.unwrap_or(u64::MAX));

    let mut buckets: HashMap<&str, Vec<&Record>> = HashMap::new();
    let mut orphans: Vec<&Record> = Vec::new();
    for job in records.iter().filter(|r| r.kind == "Job") {
        match stage_of(job) {
            Some(stage) => buckets.entry(stage.id.as_str()).or_default().push(job),
            None => orphans.push(job),
        }
    }

    let build_jobs = |mut jobs: Vec<&Record>| -> Vec<JobInfo> {
        jobs.sort_by_key(|j| j.order.unwrap_or(u64::MAX));
        jobs.into_iter()
            .map(|job| JobInfo {
                // Log id as the numeric id: it's also what job_trace needs.
                id: job.log.as_ref().map(|l| l.id).unwrap_or(0),
                name: job.name.clone(),
                stage: String::new(),
                status: normalize(job.state.as_deref(), job.result.as_deref()),
                allow_failure: false,
                duration: secs_between(job.start_time.as_deref(), job.finish_time.as_deref()),
                web_url: String::new(),
                finished_at: job.finish_time.clone(),
                // The downstream concept is GitLab-specific (bridge jobs).
                downstream: None,
            })
            .collect()
    };

    let mut out: Vec<StageInfo> = stages
        .into_iter()
        .map(|stage| {
            let mut jobs = build_jobs(buckets.remove(stage.id.as_str()).unwrap_or_default());
            for job in &mut jobs {
                job.stage = stage.name.clone();
            }
            // The stage's own status is in the record; use that instead of
            // aggregating it from the jobs.
            let status = normalize(stage.state.as_deref(), stage.result.as_deref());
            StageInfo {
                name: stage.name.clone(),
                status,
                jobs,
            }
        })
        .collect();

    if !orphans.is_empty() {
        let mut jobs = build_jobs(orphans);
        for job in &mut jobs {
            job.stage = "(pipeline)".to_string();
        }
        let status = aggregate(jobs.iter().map(|j| (j.status.as_str(), j.allow_failure)));
        out.push(StageInfo {
            name: "(pipeline)".to_string(),
            status,
            jobs,
        });
    }

    out
}

/// Parses a "Project" or "Project/DefinitionId" id.
fn parse_id(id: &str) -> Result<(String, Option<u64>)> {
    let mut parts = id.splitn(2, '/');
    let project = parts.next().unwrap_or("").trim();
    if project.is_empty() {
        return Err(anyhow!("Azure project id cannot be empty"));
    }
    let definition = match parts.next() {
        None => None,
        Some(raw) => Some(raw.trim().parse::<u64>().map_err(|_| {
            anyhow!("Azure id must be \"Project\" or \"Project/DefinitionId\", got: {id}")
        })?),
    };
    Ok((project.to_string(), definition))
}

impl Azure {
    pub fn new(http: Client, org: String, token: String) -> Self {
        Self {
            http,
            org: org.trim_end_matches('/').to_string(),
            token,
        }
    }

    fn auth(&self) -> String {
        // A PAT goes over Basic auth with an empty username.
        let encoded = base64::engine::general_purpose::STANDARD.encode(format!(":{}", self.token));
        format!("Basic {encoded}")
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Response> {
        let resp = req
            .header(reqwest::header::AUTHORIZATION, self.auth())
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .send()
            .await
            .map_err(|e| anyhow!("Could not reach {}: {}", self.org, e))?;
        check(resp).await
    }

    async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        let resp = self.send(self.http.get(url)).await?;
        let body = resp.text().await?;
        serde_json::from_str(&body).map_err(|e| anyhow!("Could not parse Azure's response: {}", e))
    }

    pub async fn current_user(&self) -> Result<String> {
        let data: ConnectionData = self
            .get_json(&format!("{}/_apis/connectionData?{API_PREVIEW}", self.org))
            .await?;
        Ok(data
            .authenticated_user
            .and_then(|u| u.custom_display_name.or(u.provider_display_name))
            .unwrap_or_else(|| "(unnamed)".to_string()))
    }

    pub async fn fetch(
        &self,
        id: &str,
        git_ref: Option<&str>,
        watch_prs: bool,
    ) -> Result<(Option<PipelineInfo>, Vec<MergeRequestInfo>)> {
        let (project, definition) = parse_id(id)?;
        let project_enc = urlencoding::encode(&project).into_owned();

        let prs = if watch_prs {
            self.open_prs(&project_enc, &project)
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let mut url = format!(
            "{}/{}/_apis/build/builds?$top=1&queryOrder=queueTimeDescending&{API}",
            self.org, project_enc
        );
        if let Some(def) = definition {
            url.push_str(&format!("&definitions={def}"));
        }
        if let Some(git_ref) = git_ref {
            url.push_str(&format!(
                "&branchName=refs/heads/{}",
                urlencoding::encode(git_ref)
            ));
        }
        let page: Page<ApiBuild> = self.get_json(&url).await?;
        let Some(build) = page.value.into_iter().next() else {
            return Ok((None, prs));
        };

        // If the timeline can't be fetched, the build is still shown, just without stages.
        let stages = self
            .get_json::<Timeline>(&format!(
                "{}/{}/_apis/build/builds/{}/timeline?{API}",
                self.org, project_enc, build.id
            ))
            .await
            .map(|t| stages_from_timeline(&t.records))
            .unwrap_or_default();

        let definition_name = build
            .definition
            .as_ref()
            .and_then(|d| d.name.clone())
            .unwrap_or_else(|| project.clone());

        Ok((
            Some(PipelineInfo {
                project_id: id.to_string(),
                project_name: format!("{project} / {definition_name}"),
                project_url: format!("{}/{}", self.org, project_enc),
                id: build.id,
                git_ref: build
                    .source_branch
                    .as_deref()
                    .map(short_ref)
                    .unwrap_or_else(|| "-".to_string()),
                sha: build.source_version.unwrap_or_default(),
                status: normalize(build.status.as_deref(), build.result.as_deref()),
                source: build.reason,
                web_url: build
                    .links
                    .and_then(|l| l.web)
                    .and_then(|w| w.href)
                    .unwrap_or_else(|| format!("{}/{}", self.org, project_enc)),
                created_at: build.queue_time,
                duration: secs_between(build.start_time.as_deref(), build.finish_time.as_deref()),
                triggered_by: build.requested_for.and_then(|u| u.display_name),
                commit_title: None,
                stages,
            }),
            prs,
        ))
    }

    async fn open_prs(&self, project_enc: &str, project: &str) -> Result<Vec<MergeRequestInfo>> {
        let page: Page<ApiPr> = self
            .get_json(&format!(
                "{}/{}/_apis/git/pullrequests?searchCriteria.status=active&$top={PR_TOP}&{API}",
                self.org, project_enc
            ))
            .await?;
        Ok(page
            .value
            .into_iter()
            .map(|pr| {
                let repo = pr
                    .repository
                    .and_then(|r| r.name)
                    .unwrap_or_else(|| project.to_string());
                let web_url = format!(
                    "{}/{}/_git/{}/pullrequest/{}",
                    self.org,
                    project_enc,
                    urlencoding::encode(&repo),
                    pr.pull_request_id
                );
                MergeRequestInfo {
                    iid: pr.pull_request_id,
                    title: pr.title,
                    author: pr.created_by.and_then(|u| u.display_name),
                    source_branch: pr
                        .source_ref_name
                        .as_deref()
                        .map(short_ref)
                        .unwrap_or_default(),
                    target_branch: pr
                        .target_ref_name
                        .as_deref()
                        .map(short_ref)
                        .unwrap_or_default(),
                    web_url,
                    created_at: pr.creation_date,
                    draft: pr.is_draft,
                }
            })
            .collect())
    }

    pub async fn retry(&self, id: &str, build_id: u64) -> Result<()> {
        let (_, _) = parse_id(id)?; // early validation
        let (project, _) = parse_id(id)?;
        let url = format!(
            "{}/{}/_apis/build/builds/{}?retry=true&{API}",
            self.org,
            urlencoding::encode(&project),
            build_id
        );
        self.send(self.http.patch(url).json(&serde_json::json!({})))
            .await?;
        Ok(())
    }

    pub async fn cancel(&self, id: &str, build_id: u64) -> Result<()> {
        let (project, _) = parse_id(id)?;
        let url = format!(
            "{}/{}/_apis/build/builds/{}?{API}",
            self.org,
            urlencoding::encode(&project),
            build_id
        );
        self.send(
            self.http
                .patch(url)
                .json(&serde_json::json!({ "status": "cancelling" })),
        )
        .await?;
        Ok(())
    }

    /// Job id = log id (see stages_from_timeline).
    pub async fn job_trace(&self, id: &str, build_id: u64, log_id: u64) -> Result<String> {
        if log_id == 0 {
            return Err(anyhow!(
                "No log yet for this job (it may not have started)."
            ));
        }
        let (project, _) = parse_id(id)?;
        let url = format!(
            "{}/{}/_apis/build/builds/{}/logs/{}?{API}",
            self.org,
            urlencoding::encode(&project),
            build_id,
            log_id
        );
        let resp = self.send(self.http.get(url)).await?;
        let text = resp.text().await?;
        Ok(crate::gitlab::tail(&text, TRACE_TAIL_LINES))
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
        StatusCode::UNAUTHORIZED => " - PAT is invalid or expired",
        StatusCode::FORBIDDEN => " - PAT lacks permission (needs Build read & execute, Code read)",
        StatusCode::NOT_FOUND => " - project/organization doesn't exist or the PAT can't see it",
        _ => "",
    };
    Err(anyhow!(
        "Azure DevOps {}{}: {}",
        status.as_u16(),
        hint,
        detail
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_normalization() {
        assert_eq!(normalize(Some("inProgress"), None), "running");
        assert_eq!(normalize(Some("notStarted"), None), "pending");
        assert_eq!(normalize(Some("completed"), Some("succeeded")), "success");
        assert_eq!(
            normalize(Some("completed"), Some("partiallySucceeded")),
            "success"
        );
        assert_eq!(normalize(Some("completed"), Some("failed")), "failed");
        assert_eq!(normalize(Some("completed"), Some("canceled")), "canceled");
        assert_eq!(normalize(Some("cancelling"), None), "canceling");
    }

    #[test]
    fn id_parsing() {
        assert_eq!(parse_id("Project").unwrap(), ("Project".to_string(), None));
        assert_eq!(
            parse_id("Project/42").unwrap(),
            ("Project".to_string(), Some(42))
        );
        assert!(parse_id("").is_err());
        assert!(parse_id("Project/abc").is_err());
    }

    fn record(id: &str, parent: Option<&str>, kind: &str, name: &str, order: u64) -> Record {
        Record {
            id: id.to_string(),
            parent_id: parent.map(str::to_string),
            kind: kind.to_string(),
            name: name.to_string(),
            state: Some("completed".to_string()),
            result: Some("succeeded".to_string()),
            order: Some(order),
            log: Some(LogRef { id: order + 100 }),
            start_time: None,
            finish_time: None,
        }
    }

    #[test]
    fn timeline_stage_tree() {
        // Stage(s1) -> Phase(p1) -> Job(j1), Stage(s2) -> Phase(p2) -> Job(j2)
        let records = vec![
            record("s2", None, "Stage", "Deploy", 2),
            record("s1", None, "Stage", "Build", 1),
            record("p1", Some("s1"), "Phase", "Build phase", 1),
            record("j1", Some("p1"), "Job", "Compile", 1),
            record("p2", Some("s2"), "Phase", "Deploy phase", 1),
            record("j2", Some("p2"), "Job", "Publish", 1),
        ];
        let stages = stages_from_timeline(&records);
        let names: Vec<&str> = stages.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            ["Build", "Deploy"],
            "should be ordered by the order field"
        );
        assert_eq!(stages[0].jobs[0].name, "Compile");
        assert_eq!(stages[0].jobs[0].stage, "Build");
        assert_eq!(stages[1].jobs[0].name, "Publish");
        // Job id comes from the log id.
        assert_eq!(stages[0].jobs[0].id, 101);
    }

    #[test]
    fn stageless_timeline_falls_into_pipeline_group() {
        let records = vec![
            record("p1", None, "Phase", "phase", 1),
            record("j1", Some("p1"), "Job", "work", 1),
        ];
        let stages = stages_from_timeline(&records);
        assert_eq!(stages.len(), 1);
        assert_eq!(stages[0].name, "(pipeline)");
        assert_eq!(stages[0].jobs.len(), 1);
    }
}
