//! Provider dispatch layer.
//!
//! Each provider lives in its own module (gitlab, github, azure) and all of
//! them produce the same canonical model (PipelineInfo / MergeRequestInfo).
//! This module builds the right client and routes the call to it.
//! Unsupported operations return a user-facing error message explaining
//! which provider doesn't support the requested action.

use std::collections::HashMap;

use anyhow::{anyhow, Result};

use crate::azure::Azure;
use crate::github::Github;
use crate::gitlab::{ApiJob, Gitlab};
use crate::model::{
    aggregate, DownstreamInfo, JobInfo, MergeRequestInfo, PipelineInfo, ProviderKind, StageInfo,
};
use crate::state::AppState;

pub enum Client {
    Gitlab(Gitlab),
    Github(Github),
    Azure(Azure),
}

/// Client ready for the given provider. Returns a user-facing error if
/// there's no token, or (for Azure) the org URL is missing.
pub fn client_for(state: &AppState, kind: ProviderKind) -> Result<Client, String> {
    let token = state
        .tokens
        .read()
        .get(&kind)
        .cloned()
        .ok_or_else(|| format!("Enter a token for {} in Settings first.", kind.label()))?;
    let config = state.config.read().clone();

    Ok(match kind {
        ProviderKind::Gitlab => {
            Client::Gitlab(Gitlab::new(state.http.clone(), config.gitlab_url, token))
        }
        ProviderKind::Github => {
            Client::Github(Github::new(state.http.clone(), config.github_url, token))
        }
        ProviderKind::Azure => {
            if config.azure_org_url.trim().is_empty() {
                return Err("Enter the organization URL for Azure in Settings first \
                     (https://dev.azure.com/organization)."
                    .to_string());
            }
            Client::Azure(Azure::new(state.http.clone(), config.azure_org_url, token))
        }
    })
}

impl Client {
    pub async fn current_user(&self) -> Result<String> {
        match self {
            Client::Gitlab(c) => c.current_user().await,
            Client::Github(c) => c.current_user().await,
            Client::Azure(c) => c.current_user().await,
        }
    }

    /// Latest pipeline + stage/job breakdown + open MR/PR list.
    pub async fn fetch(
        &self,
        id: &str,
        git_ref: Option<&str>,
        watch_mrs: bool,
    ) -> Result<(Option<PipelineInfo>, Vec<MergeRequestInfo>)> {
        match self {
            Client::Gitlab(c) => fetch_gitlab(c, id, git_ref, watch_mrs).await,
            Client::Github(c) => c.fetch(id, git_ref, watch_mrs).await,
            Client::Azure(c) => c.fetch(id, git_ref, watch_mrs).await,
        }
    }

    pub async fn retry_pipeline(&self, id: &str, pipeline_id: u64) -> Result<()> {
        match self {
            Client::Gitlab(c) => c.retry_pipeline(id, pipeline_id).await,
            Client::Github(c) => c.rerun(id, pipeline_id).await,
            Client::Azure(c) => c.retry(id, pipeline_id).await,
        }
    }

    pub async fn cancel_pipeline(&self, id: &str, pipeline_id: u64) -> Result<()> {
        match self {
            Client::Gitlab(c) => c.cancel_pipeline(id, pipeline_id).await,
            Client::Github(c) => c.cancel(id, pipeline_id).await,
            Client::Azure(c) => c.cancel(id, pipeline_id).await,
        }
    }

    pub async fn retry_job(&self, id: &str, job_id: u64) -> Result<()> {
        match self {
            Client::Gitlab(c) => c.retry_job(id, job_id).await,
            Client::Github(c) => c.rerun_job(id, job_id).await,
            Client::Azure(_) => Err(unsupported("Azure DevOps", "retrying a single job")),
        }
    }

    pub async fn cancel_job(&self, id: &str, job_id: u64) -> Result<()> {
        match self {
            Client::Gitlab(c) => c.cancel_job(id, job_id).await,
            Client::Github(_) => Err(unsupported("GitHub", "canceling a single job")),
            Client::Azure(_) => Err(unsupported("Azure DevOps", "canceling a single job")),
        }
    }

    pub async fn play_job(&self, id: &str, job_id: u64) -> Result<()> {
        match self {
            Client::Gitlab(c) => c.play_job(id, job_id).await,
            Client::Github(_) => Err(unsupported("GitHub", "manually starting a job")),
            Client::Azure(_) => Err(unsupported("Azure DevOps", "manually starting a job")),
        }
    }

    pub async fn job_trace(&self, id: &str, pipeline_id: u64, job_id: u64) -> Result<String> {
        match self {
            Client::Gitlab(c) => c.job_trace(id, job_id).await,
            Client::Github(c) => c.job_trace(id, job_id).await,
            Client::Azure(c) => c.job_trace(id, pipeline_id, job_id).await,
        }
    }
}

fn unsupported(provider: &str, what: &str) -> anyhow::Error {
    anyhow!("The {provider} API doesn't support {what}.")
}

// ------------------------------------------------------------------ gitlab --

/// Combines GitLab's multi-endpoint shape (project + pipeline + jobs + MRs)
/// into a single fetch. The other providers do this within their own module.
async fn fetch_gitlab(
    gitlab: &Gitlab,
    id: &str,
    git_ref: Option<&str>,
    watch_mrs: bool,
) -> Result<(Option<PipelineInfo>, Vec<MergeRequestInfo>)> {
    let meta = gitlab.project(id).await?;

    // If the MR list can't be fetched, show the pipeline anyway; MRs are secondary info.
    let merge_requests = if watch_mrs {
        gitlab
            .open_merge_requests(id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|mr| MergeRequestInfo {
                iid: mr.iid,
                title: mr.title,
                author: mr.author.and_then(|a| a.name.or(a.username)),
                source_branch: mr.source_branch,
                target_branch: mr.target_branch,
                web_url: mr.web_url,
                created_at: mr.created_at,
                draft: mr.draft,
            })
            .collect()
    } else {
        Vec::new()
    };

    let Some(pipeline) = gitlab.latest_pipeline(id, git_ref).await? else {
        return Ok((None, merge_requests));
    };

    // If the job list can't be fetched, show the pipeline anyway, just without stages.
    let mut jobs = gitlab.jobs(id, pipeline.id).await.unwrap_or_default();

    // Bridge (trigger) jobs come from a separate endpoint; `/jobs` never
    // returns them. Since they have the same shape, we append them straight
    // to the list -- that way they show up in their own stage, in the right
    // order, and carry the downstream pipeline's status along with them.
    let bridges = gitlab.bridges(id, pipeline.id).await.unwrap_or_default();

    // For each bridge, also fetch the downstream pipeline's own jobs. The
    // downstream pipeline can live in a different project, so we address it
    // by its numeric project_id. Sequential: a pipeline usually has only a
    // few bridges, and the poll interval is already 30s.
    let mut child_stages: HashMap<u64, Vec<StageInfo>> = HashMap::new();
    for bridge in &bridges {
        let Some(down) = &bridge.downstream_pipeline else {
            continue;
        };
        let Some(project_id) = down.project_id else {
            continue;
        };
        // If we don't have access (no permission on the other project), the
        // downstream pipeline still shows as a badge, just with an empty job breakdown.
        let child = gitlab
            .jobs(&project_id.to_string(), down.id)
            .await
            .unwrap_or_default();
        if !child.is_empty() {
            child_stages.insert(down.id, group_stages(&child));
        }
    }

    jobs.extend(bridges);
    let commit_title = jobs
        .iter()
        .find_map(|j| j.commit.as_ref().and_then(|c| c.title.clone()));

    Ok((
        Some(PipelineInfo {
            project_id: id.to_string(),
            project_name: meta.name_with_namespace,
            project_url: meta.web_url,
            id: pipeline.id,
            git_ref: pipeline.git_ref.unwrap_or_else(|| "-".to_string()),
            sha: pipeline.sha.unwrap_or_default(),
            status: pipeline.status,
            source: pipeline.source,
            web_url: pipeline.web_url,
            created_at: pipeline.created_at,
            duration: pipeline.duration,
            triggered_by: pipeline.user.and_then(|u| u.name.or(u.username)),
            commit_title,
            stages: {
                let mut stages = group_stages(&jobs);
                attach_downstream_stages(&mut stages, &child_stages);
                stages
            },
        }),
        merge_requests,
    ))
}

/// Fills in bridge jobs' `downstream.stages` field with the downstream
/// pipeline's jobs fetched earlier.
///
/// Not done inside `group_stages` because that function is a pure,
/// testable function that knows nothing about network requests; downstream
/// pipeline jobs come from separate requests.
fn attach_downstream_stages(stages: &mut [StageInfo], child_stages: &HashMap<u64, Vec<StageInfo>>) {
    for stage in stages {
        for job in &mut stage.jobs {
            let Some(down) = &mut job.downstream else {
                continue;
            };
            if let Some(child) = child_stages.get(&down.id) {
                down.stages = child.clone();
            }
        }
    }
}

/// Splits jobs into stages (GitLab only; the others build their own breakdown).
///
/// The list also includes older retries (see `Gitlab::jobs`). For the same
/// (stage, name), the DISPLAY uses the most recent retry, but the ORDERING
/// is based on the first attempt's id; that way retrying a job changes
/// neither the stage order nor the job's position within its stage. Since
/// jobs are created in stage order, the first attempt's ids reflect the
/// pipeline's actual flow.
pub fn group_stages(jobs: &[ApiJob]) -> Vec<StageInfo> {
    struct Slot<'a> {
        /// The first attempt's id; the only thing ordering is based on.
        earliest: u64,
        /// The most recent attempt; the status/duration shown on screen come from this.
        latest: &'a ApiJob,
    }

    let mut slots: HashMap<(String, String), Slot> = HashMap::new();
    for job in jobs {
        let stage = if job.stage.is_empty() {
            "(no stage)".to_string()
        } else {
            job.stage.clone()
        };
        slots
            .entry((stage, job.name.clone()))
            .and_modify(|slot| {
                slot.earliest = slot.earliest.min(job.id);
                if job.id > slot.latest.id {
                    slot.latest = job;
                }
            })
            .or_insert(Slot {
                earliest: job.id,
                latest: job,
            });
    }

    let mut stage_first: HashMap<String, u64> = HashMap::new();
    for ((stage, _), slot) in &slots {
        stage_first
            .entry(stage.clone())
            .and_modify(|id| *id = (*id).min(slot.earliest))
            .or_insert(slot.earliest);
    }

    let mut order: Vec<String> = stage_first.keys().cloned().collect();
    order.sort_by_key(|stage| stage_first[stage]);

    order
        .into_iter()
        .map(|name| {
            let mut rows: Vec<(u64, JobInfo)> = slots
                .iter()
                .filter(|((stage, _), _)| *stage == name)
                .map(|(_, slot)| {
                    let job = slot.latest;
                    (
                        slot.earliest,
                        JobInfo {
                            id: job.id,
                            name: job.name.clone(),
                            stage: job.stage.clone(),
                            status: job.status.clone(),
                            allow_failure: job.allow_failure,
                            duration: job.duration,
                            web_url: job.web_url.clone().unwrap_or_default(),
                            finished_at: job.finished_at.clone(),
                            downstream: job.downstream_pipeline.as_ref().map(|d| {
                                DownstreamInfo {
                                    id: d.id,
                                    status: d.status.clone(),
                                    web_url: d.web_url.clone().unwrap_or_default(),
                                    git_ref: d.git_ref.clone(),
                                    // The downstream pipeline's jobs aren't
                                    // filled in here, but by
                                    // `attach_downstream_stages`: this keeps
                                    // `group_stages` a pure function that
                                    // makes no network requests.
                                    stages: Vec::new(),
                                }
                            }),
                        },
                    )
                })
                .collect();
            rows.sort_by_key(|(earliest, _)| *earliest);
            let jobs: Vec<JobInfo> = rows.into_iter().map(|(_, job)| job).collect();
            let status = aggregate(jobs.iter().map(|j| (j.status.as_str(), j.allow_failure)));
            StageInfo { name, status, jobs }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: u64, name: &str, stage: &str, status: &str) -> ApiJob {
        ApiJob {
            id,
            name: name.to_string(),
            stage: stage.to_string(),
            status: status.to_string(),
            allow_failure: false,
            duration: None,
            web_url: None,
            finished_at: None,
            commit: None,
            downstream_pipeline: None,
        }
    }

    #[test]
    fn retry_does_not_disturb_stage_order() {
        // build(1) -> test(2,3) flow; then the build job was retried with id 9.
        let jobs = vec![
            job(1, "compile", "build", "failed"),
            job(2, "unit", "test", "success"),
            job(3, "e2e", "test", "success"),
            job(9, "compile", "build", "running"), // retry: highest id
        ];
        let stages = group_stages(&jobs);

        let names: Vec<&str> = stages.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["build", "test"], "build should still come first");

        // The display uses the latest attempt (id 9, running); the older one is hidden.
        let build = &stages[0];
        assert_eq!(build.jobs.len(), 1);
        assert_eq!(build.jobs[0].id, 9);
        assert_eq!(build.jobs[0].status, "running");
        assert_eq!(build.status, "running");
    }

    #[test]
    fn retried_job_keeps_its_position_in_stage() {
        // test stage has unit(2), e2e(3); e2e was retried with id 8.
        // e2e should still come after unit.
        let jobs = vec![
            job(2, "unit", "test", "success"),
            job(3, "e2e", "test", "failed"),
            job(8, "e2e", "test", "success"),
        ];
        let stages = group_stages(&jobs);
        let names: Vec<&str> = stages[0].jobs.iter().map(|j| j.name.as_str()).collect();
        assert_eq!(names, ["unit", "e2e"]);
        assert_eq!(stages[0].jobs[1].id, 8);
    }

    #[test]
    fn stage_order_comes_from_first_attempt() {
        let jobs = vec![
            job(10, "a", "first", "success"),
            job(20, "b", "second", "success"),
            job(30, "c", "third", "success"),
            // the "first" stage's job was retried with a much higher id
            job(99, "a", "first", "success"),
        ];
        let stages = group_stages(&jobs);
        let names: Vec<&str> = stages.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["first", "second", "third"]);
    }
}
