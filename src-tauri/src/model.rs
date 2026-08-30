//! Data model shared with the frontend. Field names are converted to
//! camelCase; their counterparts live verbatim in `src/types.ts`.

use serde::{Deserialize, Serialize};

/// GitLab status strings (`success`, `failed`, `running`, ...) are carried
/// through as-is. Not converted to an enum because GitLab occasionally adds
/// new statuses, and we don't want an unknown value to blow up the whole
/// snapshot.
pub type Status = String;

/// Supported CI providers. Serialized lowercase in the config; older config
/// files without the field default to GitLab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    #[default]
    Gitlab,
    Github,
    Azure,
}

impl ProviderKind {
    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::Gitlab => "GitLab",
            ProviderKind::Github => "GitHub",
            ProviderKind::Azure => "Azure DevOps",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "gitlab" => Some(Self::Gitlab),
            "github" => Some(Self::Github),
            "azure" => Some(Self::Azure),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Bad,
    Busy,
    Warn,
    Idle,
}

pub fn tone_of(status: &str) -> Tone {
    match status {
        "success" => Tone::Ok,
        "failed" => Tone::Bad,
        "running" => Tone::Busy,
        "pending" | "created" | "preparing" | "waiting_for_resource" | "scheduled" => Tone::Warn,
        _ => Tone::Idle,
    }
}

/// Produces a single summary status from a group of job/pipeline statuses.
/// Failed jobs marked `allow_failure` count as success (GitLab does the same).
pub fn aggregate<'a, I>(items: I) -> Status
where
    I: IntoIterator<Item = (&'a str, bool)>,
{
    let (mut any, mut failed, mut running, mut pending) = (false, false, false, false);
    let (mut manual, mut canceled, mut success) = (false, false, false);

    for (status, allow_failure) in items {
        any = true;
        match status {
            "failed" => {
                if allow_failure {
                    success = true;
                } else {
                    failed = true;
                }
            }
            "running" | "canceling" => running = true,
            "pending" | "created" | "preparing" | "waiting_for_resource" | "scheduled" => {
                pending = true
            }
            "manual" => manual = true,
            "canceled" => canceled = true,
            "success" => success = true,
            _ => {}
        }
    }

    if !any {
        return "none".to_string();
    }
    // If anything is still running, the pipeline counts as "running"; a
    // failure elsewhere shows up afterward.
    let result = if running {
        "running"
    } else if failed {
        "failed"
    } else if pending {
        "pending"
    } else if canceled {
        "canceled"
    } else if manual {
        "manual"
    } else if success {
        "success"
    } else {
        "skipped"
    };
    result.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WatchedProject {
    /// GitLab: numeric id or "group/project"; GitHub: "owner/repo";
    /// Azure: "Project" or "Project/DefinitionId".
    pub id: String,
    #[serde(default)]
    pub provider: ProviderKind,
    #[serde(default)]
    pub git_ref: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub gitlab_url: String,
    /// https://api.github.com for github.com; https://host/api/v3 for GHES
    #[serde(default = "default_github_url")]
    pub github_url: String,
    /// https://dev.azure.com/organization — required if Azure is used.
    #[serde(default)]
    pub azure_org_url: String,
    #[serde(default)]
    pub watched: Vec<WatchedProject>,
    #[serde(default = "default_poll")]
    pub poll_seconds: u64,
    #[serde(default = "yes")]
    pub notify_on_failure: bool,
    #[serde(default = "yes")]
    pub notify_on_recovery: bool,
    #[serde(default = "yes")]
    pub start_collapsed: bool,
    #[serde(default = "yes")]
    pub show_on_all_spaces: bool,
    #[serde(default)]
    pub top_offset: i32,
    /// Also watch open merge requests (one extra request per project).
    #[serde(default = "yes")]
    pub watch_merge_requests: bool,
    #[serde(default = "yes")]
    pub notify_on_new_merge_request: bool,
    /// Only notify about MRs opened against the project's watched branch.
    ///
    /// Has no effect while the project's "branch" field is empty; when it's
    /// set, MRs whose `target_branch` doesn't match are silently skipped
    /// (they still show up in the panel, they just don't produce a
    /// notification/ticker entry).
    #[serde(default = "yes")]
    pub notify_only_watched_branch_mr: bool,
}

fn default_poll() -> u64 {
    30
}

fn default_github_url() -> String {
    "https://api.github.com".to_string()
}

fn yes() -> bool {
    true
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            gitlab_url: "https://gitlab.com".to_string(),
            github_url: default_github_url(),
            azure_org_url: String::new(),
            watched: Vec::new(),
            poll_seconds: default_poll(),
            notify_on_failure: true,
            notify_on_recovery: true,
            start_collapsed: true,
            show_on_all_spaces: true,
            top_offset: 0,
            watch_merge_requests: true,
            notify_on_new_merge_request: true,
            notify_only_watched_branch_mr: true,
        }
    }
}

impl AppConfig {
    /// Clamps user-supplied values to sane ranges.
    pub fn sanitized(mut self) -> Self {
        self.gitlab_url = self.gitlab_url.trim().trim_end_matches('/').to_string();
        if self.gitlab_url.is_empty() {
            self.gitlab_url = "https://gitlab.com".to_string();
        }
        self.github_url = self.github_url.trim().trim_end_matches('/').to_string();
        if self.github_url.is_empty() {
            self.github_url = default_github_url();
        }
        self.azure_org_url = self.azure_org_url.trim().trim_end_matches('/').to_string();
        self.poll_seconds = self.poll_seconds.clamp(5, 3600);
        self.top_offset = self.top_offset.clamp(0, 400);
        self.watched.retain(|p| !p.id.trim().is_empty());
        for p in &mut self.watched {
            p.id = p.id.trim().trim_matches('/').to_string();
            p.git_ref = p.git_ref.take().filter(|s| !s.trim().is_empty());
            p.label = p.label.take().filter(|s| !s.trim().is_empty());
        }
        self
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobInfo {
    pub id: u64,
    pub name: String,
    pub stage: String,
    pub status: Status,
    pub allow_failure: bool,
    pub duration: Option<f64>,
    pub web_url: String,
    pub finished_at: Option<String>,
    /// The downstream pipeline this job triggered, if it's a bridge job; `None` otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downstream: Option<DownstreamInfo>,
}

/// The downstream (child or multi-project) pipeline triggered by a bridge job.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownstreamInfo {
    pub id: u64,
    pub status: Status,
    pub web_url: String,
    pub git_ref: Option<String>,
    /// The downstream pipeline's own jobs, split into stages.
    ///
    /// Shown read-only: these jobs may belong to a different project that
    /// isn't itself in the watched list, so retry/cancel/log commands don't
    /// work for them.
    pub stages: Vec<StageInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageInfo {
    pub name: String,
    pub status: Status,
    pub jobs: Vec<JobInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineInfo {
    pub project_id: String,
    pub project_name: String,
    pub project_url: String,
    pub id: u64,
    pub git_ref: String,
    pub sha: String,
    pub status: Status,
    pub source: Option<String>,
    pub web_url: String,
    pub created_at: Option<String>,
    pub duration: Option<f64>,
    pub triggered_by: Option<String>,
    pub commit_title: Option<String>,
    pub stages: Vec<StageInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeRequestInfo {
    pub iid: u64,
    pub title: String,
    pub author: Option<String>,
    pub source_branch: String,
    pub target_branch: String,
    pub web_url: String,
    pub created_at: Option<String>,
    pub draft: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSnapshot {
    pub project: WatchedProject,
    pub pipeline: Option<PipelineInfo>,
    #[serde(default)]
    pub merge_requests: Vec<MergeRequestInfo>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub projects: Vec<ProjectSnapshot>,
    pub overall: Status,
    pub fetched_at: String,
    pub configured: bool,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            projects: Vec::new(),
            overall: "none".to_string(),
            fetched_at: String::new(),
            configured: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenState {
    pub present: bool,
    pub username: Option<String>,
}

/// Token status for all three providers together; the settings screen shows this.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenStates {
    pub gitlab: TokenState,
    pub github: TokenState,
    pub azure: TokenState,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_list_gives_none() {
        assert_eq!(aggregate(Vec::<(&str, bool)>::new()), "none");
    }

    #[test]
    fn running_job_shadows_failed() {
        assert_eq!(
            aggregate([("failed", false), ("running", false)]),
            "running"
        );
    }

    #[test]
    fn allow_failure_absorbs_failure() {
        assert_eq!(aggregate([("failed", true), ("success", false)]), "success");
    }

    #[test]
    fn real_failure_wins() {
        assert_eq!(aggregate([("failed", false), ("success", false)]), "failed");
    }

    #[test]
    fn success_when_all_succeed() {
        assert_eq!(
            aggregate([("success", false), ("skipped", false)]),
            "success"
        );
    }

    #[test]
    fn tones() {
        assert_eq!(tone_of("success"), Tone::Ok);
        assert_eq!(tone_of("failed"), Tone::Bad);
        assert_eq!(tone_of("running"), Tone::Busy);
        assert_eq!(tone_of("pending"), Tone::Warn);
        assert_eq!(tone_of("something-else"), Tone::Idle);
    }
}
