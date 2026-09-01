import { useEffect, useState } from "react";
import { useTauriEvent } from "./hooks/useTauriEvent";
import {
  clearToken,
  errorText,
  getConfig,
  getTokenStates,
  logPath,
  onConfig,
  saveConfig,
  setToken,
} from "./lib/api";
import type {
  AppConfig,
  ProviderKind,
  TokenState,
  TokenStates,
  WatchedProject,
} from "./types";

const EMPTY_PROJECT: WatchedProject = {
  id: "",
  provider: "gitlab",
  gitRef: null,
  label: null,
};

const PROVIDER_LABELS: Record<ProviderKind, string> = {
  gitlab: "GitLab",
  github: "GitHub",
  azure: "Azure DevOps",
};

const ID_PLACEHOLDERS: Record<ProviderKind, string> = {
  gitlab: "group/project",
  github: "owner/repo",
  azure: "Project/DefinitionId",
};

interface TokenSectionProps {
  provider: ProviderKind;
  state: TokenState | undefined;
  busy: boolean;
  hint: React.ReactNode;
  onSave: (provider: ProviderKind, token: string) => Promise<void>;
  onClear: (provider: ProviderKind) => Promise<void>;
}

function TokenSection({
  provider,
  state,
  busy,
  hint,
  onSave,
  onClear,
}: TokenSectionProps) {
  const [input, setInput] = useState("");

  return (
    <>
      <label>
        <span>{PROVIDER_LABELS[provider]} token</span>
        <input
          type="password"
          value={input}
          autoComplete="off"
          placeholder={
            state?.present
              ? "saved — enter a new token to replace it"
              : "token"
          }
          onChange={(e) => setInput(e.target.value)}
        />
        <small>{hint}</small>
      </label>
      <div className="row">
        <button
          type="button"
          disabled={busy || input.trim().length === 0}
          onClick={() => {
            void onSave(provider, input.trim()).then(() => setInput(""));
          }}
        >
          Verify and save
        </button>
        {state?.present && (
          <button
            type="button"
            disabled={busy}
            onClick={() => void onClear(provider)}
          >
            Remove
          </button>
        )}
        <span className="hint">
          {state?.present
            ? state.username
              ? `Connected: ${state.username}`
              : "Token saved"
            : "No token"}
        </span>
      </div>
    </>
  );
}

export function Settings() {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [tokens, setTokens] = useState<TokenStates | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [logFile, setLogFile] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([getConfig(), getTokenStates()])
      .then(([c, t]) => {
        setConfig(c);
        setTokens(t);
      })
      .catch((e) => setError(errorText(e)));
    logPath()
      .then(setLogFile)
      .catch(() => setLogFile(null));
  }, []);

  /**
   * The mode can also be switched from the tray or from either surface while
   * this window is open. Only `displayMode` is taken from the event on
   * purpose: adopting the whole config would throw away whatever the user has
   * typed here but not saved yet.
   */
  useTauriEvent(onConfig, (incoming: AppConfig) =>
    setConfig((cur) =>
      cur && cur.displayMode !== incoming.displayMode
        ? { ...cur, displayMode: incoming.displayMode }
        : cur,
    ),
  );

  if (!config) {
    return (
      <div className="settings">
        {error ? <p className="err">{error}</p> : <p>Loading…</p>}
      </div>
    );
  }

  const patch = (next: Partial<AppConfig>) => setConfig({ ...config, ...next });

  const patchProject = (index: number, next: Partial<WatchedProject>) => {
    const watched = config.watched.map((p, i) =>
      i === index ? { ...p, ...next } : p,
    );
    patch({ watched });
  };

  /** Reorders a project; this is also the order the notch shows the cards in. */
  const moveProject = (index: number, direction: -1 | 1) => {
    const target = index + direction;
    if (target < 0 || target >= config.watched.length) return;
    const watched = [...config.watched];
    [watched[index], watched[target]] = [watched[target], watched[index]];
    patch({ watched });
  };

  const cleanedConfig = (): AppConfig => ({
    ...config,
    gitlabUrl: config.gitlabUrl.trim().replace(/\/+$/, ""),
    githubUrl: config.githubUrl.trim().replace(/\/+$/, ""),
    azureOrgUrl: config.azureOrgUrl.trim().replace(/\/+$/, ""),
    pollSeconds: Math.max(5, Math.round(config.pollSeconds)),
    watched: config.watched
      .map((p) => ({
        id: p.id.trim(),
        provider: p.provider,
        gitRef: p.gitRef && p.gitRef.trim() ? p.gitRef.trim() : null,
        label: p.label && p.label.trim() ? p.label.trim() : null,
      }))
      .filter((p) => p.id.length > 0),
  });

  const persistVisibleConfig = async () => {
    const saved = await saveConfig(cleanedConfig());
    setConfig(saved);
    return saved;
  };

  const run = async (label: string, fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    setStatus(null);
    try {
      await fn();
      setStatus(label);
    } catch (e) {
      setError(errorText(e));
      throw e;
    } finally {
      setBusy(false);
    }
  };

  const onSave = () =>
    run("Settings saved.", async () => {
      await persistVisibleConfig();
    }).catch(() => {});

  const onSaveToken = (provider: ProviderKind, token: string) =>
    run("Token verified and saved to the keychain.", async () => {
      await persistVisibleConfig();
      const next = await setToken(provider, token);
      setTokens((cur) => (cur ? { ...cur, [provider]: next } : cur));
    }).catch(() => {});

  const onClearToken = (provider: ProviderKind) =>
    run("Token removed.", async () => {
      const next = await clearToken(provider);
      setTokens((cur) => (cur ? { ...cur, [provider]: next } : cur));
    }).catch(() => {});

  return (
    <div className="settings">
      <h1>Vitaline — Settings</h1>

      <section>
        <h2>GitLab</h2>
        <label>
          <span>Server address</span>
          <input
            type="url"
            value={config.gitlabUrl}
            placeholder="https://gitlab.com"
            onChange={(e) => patch({ gitlabUrl: e.target.value })}
          />
          <small>
            For self-hosted, enter your own address, e.g.:
            https://gitlab.company.com
          </small>
        </label>
        <TokenSection
          provider="gitlab"
          state={tokens?.gitlab}
          busy={busy}
          hint={
            <>
              GitLab &rarr; Preferences &rarr; Access tokens. Needs{" "}
              <code>read_api</code> scope for watching, <code>api</code> for
              retry/cancel.
            </>
          }
          onSave={onSaveToken}
          onClear={onClearToken}
        />
      </section>

      <section>
        <h2>GitHub</h2>
        <label>
          <span>API address</span>
          <input
            type="url"
            value={config.githubUrl}
            placeholder="https://api.github.com"
            onChange={(e) => patch({ githubUrl: e.target.value })}
          />
          <small>
            Leave as-is for github.com; for GitHub Enterprise use
            https://host/api/v3
          </small>
        </label>
        <TokenSection
          provider="github"
          state={tokens?.github}
          busy={busy}
          hint={
            <>
              GitHub &rarr; Settings &rarr; Developer settings &rarr; Personal
              access tokens. Fine-grained: <code>Actions (read/write)</code> +{" "}
              <code>Pull requests (read)</code>; classic: <code>repo</code>{" "}
              scope.
            </>
          }
          onSave={onSaveToken}
          onClear={onClearToken}
        />
      </section>

      <section>
        <h2>Azure DevOps</h2>
        <label>
          <span>Organization address</span>
          <input
            type="url"
            value={config.azureOrgUrl}
            placeholder="https://dev.azure.com/organization"
            onChange={(e) => patch({ azureOrgUrl: e.target.value })}
          />
          <small>Required if you'll watch an Azure project.</small>
        </label>
        <TokenSection
          provider="azure"
          state={tokens?.azure}
          busy={busy}
          hint={
            <>
              Azure DevOps &rarr; User settings &rarr; Personal access tokens.
              Scope: <code>Build (Read &amp; execute)</code> +{" "}
              <code>Code (Read)</code>.
            </>
          }
          onSave={onSaveToken}
          onClear={onClearToken}
        />
      </section>

      <section>
        <h2>Watched projects</h2>
        <p className="hint">
          The id format depends on the provider — GitLab: <code>12345</code>{" "}
          or <code>group/project</code>; GitHub: <code>owner/repo</code>;
          Azure: <code>Project</code> or <code>Project/DefinitionId</code>.
          The order below is also the order the notch shows the cards in —
          use &uarr;/&darr; to rearrange.
        </p>
        <table className="projects">
          <thead>
            <tr>
              <th>Provider</th>
              <th>Project</th>
              <th>Branch (opt.)</th>
              <th>Label (opt.)</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {config.watched.map((p, i) => (
              <tr key={i}>
                <td>
                  <select
                    value={p.provider}
                    onChange={(e) =>
                      patchProject(i, {
                        provider: e.target.value as ProviderKind,
                      })
                    }
                  >
                    <option value="gitlab">GitLab</option>
                    <option value="github">GitHub</option>
                    <option value="azure">Azure</option>
                  </select>
                </td>
                <td>
                  <input
                    value={p.id}
                    placeholder={ID_PLACEHOLDERS[p.provider]}
                    onChange={(e) => patchProject(i, { id: e.target.value })}
                  />
                </td>
                <td>
                  <input
                    value={p.gitRef ?? ""}
                    placeholder="main"
                    onChange={(e) =>
                      patchProject(i, { gitRef: e.target.value })
                    }
                  />
                </td>
                <td>
                  <input
                    value={p.label ?? ""}
                    placeholder="API"
                    onChange={(e) => patchProject(i, { label: e.target.value })}
                  />
                </td>
                <td className="projects__actions">
                  <button
                    type="button"
                    className="icon-btn"
                    title="Move up"
                    disabled={i === 0}
                    onClick={() => moveProject(i, -1)}
                  >
                    &uarr;
                  </button>
                  <button
                    type="button"
                    className="icon-btn"
                    title="Move down"
                    disabled={i === config.watched.length - 1}
                    onClick={() => moveProject(i, 1)}
                  >
                    &darr;
                  </button>
                  <button
                    type="button"
                    onClick={() =>
                      patch({
                        watched: config.watched.filter((_, j) => j !== i),
                      })
                    }
                  >
                    Remove
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <button
          type="button"
          onClick={() =>
            patch({ watched: [...config.watched, { ...EMPTY_PROJECT }] })
          }
        >
          Add project
        </button>
      </section>

      <section>
        <h2>Display</h2>
        <p className="hint">
          Only one of these is on screen at a time. The tray menu's{" "}
          <strong>Widget mode</strong> item switches between them too, as do the{" "}
          <strong>Widget</strong> button in the notch panel and{" "}
          <strong>Notch mode</strong> in the widget's footer.
        </p>
        <label className="check">
          <input
            type="radio"
            name="display-mode"
            checked={config.displayMode === "notch"}
            onChange={() => patch({ displayMode: "notch" })}
          />
          <span>
            <strong>Notch</strong> — a pill at the top center of the screen
            that opens on hover
          </span>
        </label>
        <label className="check">
          <input
            type="radio"
            name="display-mode"
            checked={config.displayMode === "widget"}
            onChange={() => patch({ displayMode: "widget" })}
          />
          <span>
            <strong>Widget</strong> — a panel you place anywhere and leave
            open; drag its header to move it, its bottom-right corner to
            resize it
          </span>
        </label>

        <div className="sub" data-disabled={config.displayMode !== "widget"}>
          <label className="check">
            <input
              type="radio"
              name="widget-layer"
              disabled={config.displayMode !== "widget"}
              checked={config.widget.layer === "front"}
              onChange={() =>
                patch({ widget: { ...config.widget, layer: "front" } })
              }
            />
            <span>Keep the widget above other windows</span>
          </label>
          <label className="check">
            <input
              type="radio"
              name="widget-layer"
              disabled={config.displayMode !== "widget"}
              checked={config.widget.layer === "desktop"}
              onChange={() =>
                patch({ widget: { ...config.widget, layer: "desktop" } })
              }
            />
            <span>
              Keep it on the desktop, behind other windows (it never covers
              what you're working on — and is only in view when the desktop is)
            </span>
          </label>
          <label className="inline">
            <span>Widget background opacity</span>
            <input
              type="range"
              min={35}
              max={100}
              step={1}
              disabled={config.displayMode !== "widget"}
              value={Math.round(config.widget.opacity * 100)}
              onChange={(e) =>
                patch({
                  widget: {
                    ...config.widget,
                    opacity: Number(e.target.value) / 100,
                  },
                })
              }
            />
            <span>{Math.round(config.widget.opacity * 100)}%</span>
            <small>
              Only the background fades; the text and status colors stay fully
              opaque. Applies as soon as you save.
            </small>
          </label>
        </div>
      </section>

      <section>
        <h2>Behavior</h2>
        <label className="inline">
          <span>Refresh interval (seconds)</span>
          <input
            type="number"
            min={5}
            max={3600}
            value={config.pollSeconds}
            onChange={(e) => patch({ pollSeconds: Number(e.target.value) })}
          />
        </label>
        <label className="inline">
          <span>Offset from the top edge (px, notch only)</span>
          <input
            type="number"
            min={0}
            max={200}
            value={config.topOffset}
            onChange={(e) => patch({ topOffset: Number(e.target.value) })}
          />
          <small>
            0 to sit inside the notch on macOS; a bit of offset looks better
            on Windows.
          </small>
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={config.watchMergeRequests}
            onChange={(e) => patch({ watchMergeRequests: e.target.checked })}
          />
          <span>
            Also watch open merge requests / PRs (one extra request per project)
          </span>
        </label>
        <label className="check">
          <input
            type="checkbox"
            disabled={!config.watchMergeRequests}
            checked={config.notifyOnNewMergeRequest}
            onChange={(e) =>
              patch({ notifyOnNewMergeRequest: e.target.checked })
            }
          />
          <span>
            Notify when a new MR/PR opens (also scrolls as ticker text in the notch)
          </span>
        </label>
        <label className="check">
          <input
            type="checkbox"
            disabled={
              !config.watchMergeRequests || !config.notifyOnNewMergeRequest
            }
            checked={config.notifyOnlyWatchedBranchMr}
            onChange={(e) =>
              patch({ notifyOnlyWatchedBranchMr: e.target.checked })
            }
          />
          <span>
            Only notify about MRs opened against the project's watched branch
            (no effect if the project's "branch" field is left empty; others
            still show up in the panel)
          </span>
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={config.notifyOnFailure}
            onChange={(e) => patch({ notifyOnFailure: e.target.checked })}
          />
          <span>Notify when a pipeline fails</span>
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={config.notifyOnRecovery}
            onChange={(e) => patch({ notifyOnRecovery: e.target.checked })}
          />
          <span>Notify when it recovers after a failure</span>
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={config.startCollapsed}
            onChange={(e) => patch({ startCollapsed: e.target.checked })}
          />
          <span>Show only as a small pill on startup</span>
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={config.showOnAllSpaces}
            onChange={(e) => patch({ showOnAllSpaces: e.target.checked })}
          />
          <span>
            Stay on every desktop / above full-screen apps (macOS)
          </span>
        </label>
      </section>

      {logFile && (
        <p className="hint">
          If something goes wrong, the log file is at: <code>{logFile}</code>
        </p>
      )}

      <div className="row row--footer">
        <button
          type="button"
          className="primary"
          disabled={busy}
          onClick={() => void onSave()}
        >
          Save
        </button>
        {status && <span className="ok">{status}</span>}
        {error && <span className="err">{error}</span>}
      </div>
    </div>
  );
}
