// These types are a 1:1 mirror of the serde structs in
// src-tauri/src/model.rs. The Rust side uses `rename_all = "camelCase"`;
// if you change one, change the other too.

export type PipelineStatus =
  | "created"
  | "waiting_for_resource"
  | "preparing"
  | "pending"
  | "running"
  | "success"
  | "failed"
  | "canceled"
  | "canceling"
  | "skipped"
  | "manual"
  | "scheduled"
  | "none"
  | "unknown";

export type ProviderKind = "gitlab" | "github" | "azure";

export interface WatchedProject {
  /**
   * GitLab: numeric id or "group/project"; GitHub: "owner/repo";
   * Azure: "Project" or "Project/DefinitionId".
   */
  id: string;
  provider: ProviderKind;
  /** Only watch this branch. Empty means the project's most recent pipeline. */
  gitRef: string | null;
  /** Short name shown in the notch. Empty means the name from GitLab. */
  label: string | null;
}

export interface AppConfig {
  /** e.g. https://gitlab.com or https://gitlab.company.com */
  gitlabUrl: string;
  /** https://api.github.com for github.com; https://host/api/v3 for GHES */
  githubUrl: string;
  /** https://dev.azure.com/organization — required if Azure is used. */
  azureOrgUrl: string;
  watched: WatchedProject[];
  pollSeconds: number;
  notifyOnFailure: boolean;
  notifyOnRecovery: boolean;
  /** Whether the notch should appear as just a pill at startup. */
  startCollapsed: boolean;
  /** macOS: stay on every Space and above full-screen apps. */
  showOnAllSpaces: boolean;
  /** How far the notch is offset down from the top of the screen (px). */
  topOffset: number;
  /** Also watch open merge requests (one extra request per project). */
  watchMergeRequests: boolean;
  notifyOnNewMergeRequest: boolean;
  /** Only notify about MRs opened against the project's watched branch. */
  notifyOnlyWatchedBranchMr: boolean;
}

export interface JobInfo {
  id: number;
  name: string;
  stage: string;
  status: PipelineStatus;
  allowFailure: boolean;
  /** seconds */
  duration: number | null;
  webUrl: string;
  finishedAt: string | null;
  /** The downstream pipeline this job triggered, if it's a bridge job; absent otherwise. */
  downstream?: DownstreamInfo;
}

/** The downstream (child or multi-project) pipeline triggered by a bridge job. */
export interface DownstreamInfo {
  id: number;
  status: PipelineStatus;
  webUrl: string;
  gitRef: string | null;
  /**
   * The downstream pipeline's own jobs. Shown read-only: these jobs may
   * belong to a different project that isn't itself in the watched list, so
   * retry/cancel/log commands don't work for them.
   */
  stages: StageInfo[];
}

export interface StageInfo {
  name: string;
  status: PipelineStatus;
  jobs: JobInfo[];
}

export interface PipelineInfo {
  projectId: string;
  projectName: string;
  projectUrl: string;
  id: number;
  gitRef: string;
  sha: string;
  status: PipelineStatus;
  source: string | null;
  webUrl: string;
  createdAt: string | null;
  /** seconds */
  duration: number | null;
  triggeredBy: string | null;
  commitTitle: string | null;
  stages: StageInfo[];
}

export interface MergeRequestInfo {
  iid: number;
  title: string;
  author: string | null;
  sourceBranch: string;
  targetBranch: string;
  webUrl: string;
  createdAt: string | null;
  draft: boolean;
}

export interface ProjectSnapshot {
  project: WatchedProject;
  pipeline: PipelineInfo | null;
  mergeRequests: MergeRequestInfo[];
  error: string | null;
}

export interface Snapshot {
  projects: ProjectSnapshot[];
  overall: PipelineStatus;
  /** ISO 8601 */
  fetchedAt: string;
  /** Whether a token has been entered and at least one project has been added. */
  configured: boolean;
}

export interface TokenState {
  present: boolean;
  /** The provider's username, if the token has been verified. */
  username: string | null;
}

export interface TokenStates {
  gitlab: TokenState;
  github: TokenState;
  azure: TokenState;
}

/** An announcement shown as scrolling text in the notch. */
export interface TickerItem {
  text: string;
  url: string | null;
}

/**
 * The screen's real notch dimensions (macOS, logical points).
 *
 * The area behind the notch is a physical hole: nothing drawn there is
 * visible. That's why the pill places its content in the "ears" to the
 * notch's left/right.
 */
export interface NotchMetrics {
  hasNotch: boolean;
  notchWidth: number;
  notchHeight: number;
  menuBarHeight: number;
}
