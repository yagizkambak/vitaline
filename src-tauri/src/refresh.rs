//! Snapshot production: fetching data from providers, notifications/ticker, emitting events.

use std::collections::HashSet;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::model::{aggregate, AppConfig, ProjectSnapshot, Snapshot};
use crate::providers;
use crate::state::AppState;
use crate::tray;

pub const SNAPSHOT_EVENT: &str = "pipelines://updated";
/// Short announcements shown as a scrolling ticker inside the notch.
pub const TICKER_EVENT: &str = "notch://ticker";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TickerItem {
    pub text: String,
    pub url: Option<String>,
}

/// Walks every watched project and produces the current status. One
/// project's error doesn't affect the others; the error shows up on that
/// project's own card.
pub async fn build_snapshot(state: &AppState) -> Snapshot {
    let config: AppConfig = state.config.read().clone();
    let any_token = !state.tokens.read().is_empty();
    let configured = any_token && !config.watched.is_empty();

    let mut projects: Vec<ProjectSnapshot> = Vec::with_capacity(config.watched.len());

    // Fetched sequentially: the project count is small, so this doesn't
    // stress the providers' rate limits.
    for project in &config.watched {
        let entry = match providers::client_for(state, project.provider) {
            Err(message) => ProjectSnapshot {
                project: project.clone(),
                pipeline: None,
                merge_requests: Vec::new(),
                error: Some(message),
            },
            Ok(client) => {
                match client
                    .fetch(
                        &project.id,
                        project.git_ref.as_deref(),
                        config.watch_merge_requests,
                    )
                    .await
                {
                    Ok((pipeline, merge_requests)) => ProjectSnapshot {
                        project: project.clone(),
                        pipeline,
                        merge_requests,
                        error: None,
                    },
                    Err(err) => ProjectSnapshot {
                        project: project.clone(),
                        pipeline: None,
                        merge_requests: Vec::new(),
                        error: Some(err.to_string()),
                    },
                }
            }
        };
        projects.push(entry);
    }

    let overall = aggregate(projects.iter().map(|p| {
        let status = if p.error.is_some() {
            "failed"
        } else {
            p.pipeline.as_ref().map_or("none", |pl| pl.status.as_str())
        };
        (status, false)
    }));

    Snapshot {
        projects,
        overall,
        fetched_at: now_iso(),
        configured,
    }
}

/// Fetches the data, sends notifications, stores the status, and publishes
/// it to the frontend and tray. Called by both the poll loop and manual refresh.
pub async fn refresh_and_publish(app: &AppHandle) -> Snapshot {
    let snapshot = {
        let state = app.state::<AppState>();
        build_snapshot(&state).await
    };

    let state = app.state::<AppState>();
    notify_changes(app, &state, &snapshot);
    *state.snapshot.write() = snapshot.clone();

    let _ = app.emit(SNAPSHOT_EVENT, &snapshot);
    tray::update(app, &snapshot);

    snapshot
}

/// Sends a desktop notification on status changes. No notification on first
/// sight -- the app shouldn't re-announce old failures the moment it starts.
fn notify_changes(app: &AppHandle, state: &AppState, snapshot: &Snapshot) {
    let config = state.config.read().clone();
    notify_new_merge_requests(app, state, snapshot, &config);

    let mut last = state.last_status.lock();

    for entry in &snapshot.projects {
        let Some(pipeline) = &entry.pipeline else {
            continue;
        };
        let key = entry.project.id.clone();
        let previous = last.insert(key, pipeline.status.clone());

        let Some(previous) = previous else {
            continue; // first observation
        };
        if previous == pipeline.status {
            continue;
        }

        let title = entry
            .project
            .label
            .clone()
            .unwrap_or_else(|| pipeline.project_name.clone());

        let body = format!(
            "{} · {}",
            pipeline.git_ref,
            pipeline.commit_title.as_deref().unwrap_or("-")
        );

        if pipeline.status == "failed" && config.notify_on_failure {
            push(app, &format!("{title} — pipeline failed"), &body);
            ticker(
                app,
                format!("✗ {title}: pipeline failed ({})", pipeline.git_ref),
                Some(pipeline.web_url.clone()),
            );
        } else if pipeline.status == "success" && previous == "failed" && config.notify_on_recovery
        {
            push(app, &format!("{title} — pipeline recovered"), &body);
            ticker(
                app,
                format!("✓ {title}: pipeline recovered ({})", pipeline.git_ref),
                Some(pipeline.web_url.clone()),
            );
        }
    }
}

/// Notifies about newly opened MRs/PRs on watched projects.
///
/// The first pass only records the current MRs and doesn't send a
/// notification: the app shouldn't re-announce MRs that have been sitting
/// open for months every time it's launched.
fn notify_new_merge_requests(
    app: &AppHandle,
    state: &AppState,
    snapshot: &Snapshot,
    config: &AppConfig,
) {
    if !config.watch_merge_requests {
        return;
    }

    let mut seen = state.seen_merge_requests.lock();

    for entry in &snapshot.projects {
        // If the project errored, the MR list comes back empty; don't
        // mistake that for "all closed" and wipe the memory.
        if entry.error.is_some() {
            continue;
        }

        let current: HashSet<u64> = entry.merge_requests.iter().map(|mr| mr.iid).collect();
        let known = seen.insert(entry.project.id.clone(), current);

        let Some(known) = known else {
            continue; // first pass: just seed
        };
        if !config.notify_on_new_merge_request {
            continue;
        }

        let project_name = entry
            .project
            .label
            .clone()
            .or_else(|| entry.pipeline.as_ref().map(|p| p.project_name.clone()))
            .unwrap_or_else(|| entry.project.id.clone());

        // If a watched branch is set and the option is on, don't notify
        // about MRs opened against other branches. They still show up in the
        // panel; this only affects the notification.
        let watched_branch = config
            .notify_only_watched_branch_mr
            .then_some(entry.project.git_ref.as_deref())
            .flatten();

        for mr in entry.merge_requests.iter().filter(|mr| {
            !known.contains(&mr.iid)
                && watched_branch.map_or(true, |branch| mr.target_branch == branch)
        }) {
            let title = format!("{project_name} — new MR !{}", mr.iid);
            let body = format!(
                "{}{}\n{} → {}",
                if mr.draft { "[Draft] " } else { "" },
                mr.title,
                mr.source_branch,
                mr.target_branch
            );
            push(app, &title, &body);
            ticker(
                app,
                format!(
                    "⇄ {project_name} !{}: {}{} ({} → {})",
                    mr.iid,
                    if mr.draft { "[Draft] " } else { "" },
                    mr.title,
                    mr.source_branch,
                    mr.target_branch
                ),
                Some(mr.web_url.clone()),
            );
        }
    }
}

fn push(app: &AppHandle, title: &str, body: &str) {
    if let Err(err) = app.notification().builder().title(title).body(body).show() {
        crate::log::line(&format!("notification could not be sent: {err}"));
    }
}

/// Sends an announcement to the notch's scrolling ticker; the frontend queues it.
fn ticker(app: &AppHandle, text: String, url: Option<String>) {
    let _ = app.emit(TICKER_EVENT, TickerItem { text, url });
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}
