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

/// Which surface the app shows its status on.
///
/// Exactly one is on screen at a time -- `widget::apply_mode` shows one and
/// hides the other. Older config files without the field default to `Notch`,
/// which is the behavior the app had before the widget existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DisplayMode {
    #[default]
    Notch,
    Widget,
}

impl DisplayMode {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "notch" => Some(Self::Notch),
            "widget" => Some(Self::Widget),
            _ => None,
        }
    }
}

/// Where the widget window sits in the window stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WidgetLayer {
    /// Above every other window, the way the notch behaves.
    #[default]
    Front,
    /// Below every normal window: it lives on the desktop and never covers
    /// what you're working on.
    ///
    /// This is `set_always_on_bottom(true)`, which is NOT the literal
    /// wallpaper level -- on macOS it's NSWindow level -1 (tao's
    /// `BelowNormalWindowLevel`), on Windows `HWND_BOTTOM`. Desktop icons
    /// stay below it. The visible behavior is what matters: any app window
    /// covers it, so it's only in view when the desktop is.
    Desktop,
}

/// Geometry and appearance of the widget window.
///
/// Position is stored in LOGICAL points, not physical pixels: the same
/// numbers then land in the same visual spot when the display's scale factor
/// changes (or the widget is dragged to a monitor with a different one).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetConfig {
    /// Top-left corner. `None` until the widget has been placed once; the
    /// first launch derives a spot from the screen's work area (see
    /// `widget::placement`).
    #[serde(default)]
    pub x: Option<i32>,
    #[serde(default)]
    pub y: Option<i32>,
    #[serde(default = "default_widget_width")]
    pub width: u32,
    #[serde(default = "default_widget_height")]
    pub height: u32,
    #[serde(default)]
    pub layer: WidgetLayer,
    /// Background opacity, 0.35..=1.0. Only the panel's BACKGROUND fades;
    /// text and status colors stay fully opaque (see `--widget-opacity` in
    /// styles.css), so a see-through widget is still readable.
    #[serde(default = "default_widget_opacity")]
    pub opacity: f64,
}

/// Smallest size the widget can be dragged down to. Below this the header
/// tools wrap and the rows stop being readable.
pub const WIDGET_MIN_WIDTH: u32 = 260;
pub const WIDGET_MIN_HEIGHT: u32 = 120;
/// Upper bound purely as a guard against a nonsense value in a hand-edited
/// config file. Deliberately well beyond any usable widget size: the clamp
/// also runs over geometry the user produced by DRAGGING, so a bound anywhere
/// near a real screen's width would silently shrink their window on the next
/// launch.
pub const WIDGET_MAX_WIDTH: u32 = 4000;
pub const WIDGET_MAX_HEIGHT: u32 = 4000;
pub const WIDGET_MIN_OPACITY: f64 = 0.35;

fn default_widget_width() -> u32 {
    340
}

fn default_widget_height() -> u32 {
    300
}

fn default_widget_opacity() -> f64 {
    0.94
}

impl Default for WidgetConfig {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            width: default_widget_width(),
            height: default_widget_height(),
            layer: WidgetLayer::default(),
            opacity: default_widget_opacity(),
        }
    }
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
    /// Notch or widget; see `DisplayMode`.
    #[serde(default)]
    pub display_mode: DisplayMode,
    #[serde(default)]
    pub widget: WidgetConfig,
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
            display_mode: DisplayMode::default(),
            widget: WidgetConfig::default(),
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
        self.widget.width = self.widget.width.clamp(WIDGET_MIN_WIDTH, WIDGET_MAX_WIDTH);
        self.widget.height = self
            .widget
            .height
            .clamp(WIDGET_MIN_HEIGHT, WIDGET_MAX_HEIGHT);
        // `f64::clamp` propagates NaN instead of rejecting it, and a NaN
        // opacity reaches CSS as an invalid custom property -- the widget
        // would render with no background at all. A non-finite value here can
        // only come from a hand-edited config file, so fall back to opaque.
        self.widget.opacity = if self.widget.opacity.is_finite() {
            self.widget.opacity.clamp(WIDGET_MIN_OPACITY, 1.0)
        } else {
            1.0
        };
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

    /// A config file written before the widget existed has neither
    /// `displayMode` nor `widget`; it must still load, and it must keep the
    /// notch as the active surface.
    #[test]
    fn config_without_widget_fields_defaults_to_notch() {
        let config: AppConfig = serde_json::from_str(r#"{"gitlabUrl":"https://gitlab.com"}"#)
            .expect("a pre-widget config should still deserialize");
        assert_eq!(config.display_mode, DisplayMode::Notch);
        assert_eq!(config.widget.layer, WidgetLayer::Front);
        assert_eq!(config.widget.x, None);
        assert_eq!(config.widget.width, default_widget_width());
    }

    /// Everything on the widget's config except the field under test.
    fn with_widget(widget: WidgetConfig) -> AppConfig {
        AppConfig {
            widget,
            ..AppConfig::default()
        }
    }

    #[test]
    fn widget_geometry_is_clamped() {
        let config = with_widget(WidgetConfig {
            width: 10,
            height: 99_999,
            ..WidgetConfig::default()
        })
        .sanitized();
        assert_eq!(config.widget.width, WIDGET_MIN_WIDTH);
        assert_eq!(config.widget.height, WIDGET_MAX_HEIGHT);
    }

    #[test]
    fn widget_opacity_is_clamped() {
        let opacity_after = |opacity: f64| {
            with_widget(WidgetConfig {
                opacity,
                ..WidgetConfig::default()
            })
            .sanitized()
            .widget
            .opacity
        };

        assert_eq!(opacity_after(0.0), WIDGET_MIN_OPACITY);
        assert_eq!(opacity_after(4.0), 1.0);
        // NaN would reach CSS as an invalid value and leave the widget with
        // no background; it falls back to fully opaque instead.
        assert_eq!(opacity_after(f64::NAN), 1.0);
    }

    #[test]
    fn display_mode_round_trips_through_its_config_string() {
        assert_eq!(DisplayMode::parse("widget"), Some(DisplayMode::Widget));
        assert_eq!(DisplayMode::parse("notch"), Some(DisplayMode::Notch));
        assert_eq!(DisplayMode::parse("Widget"), None);
        // The `parse` strings and serde's `rename_all` have to agree, or the
        // tray would write a mode the config file can't read back.
        assert_eq!(
            serde_json::to_string(&DisplayMode::Widget).unwrap(),
            "\"widget\""
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
