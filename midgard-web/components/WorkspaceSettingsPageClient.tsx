"use client";

import { type FormEvent, useEffect, useState } from "react";
import {
  AlertTriangle,
  Archive,
  ArrowLeft,
  LogOut,
  Play,
  Plus,
  Save,
} from "lucide-react";
import { AuthGate } from "@/components/AuthGate";
import {
  createMiddlewareInstance,
  fetchMiddlewareInstances,
  fetchOrganizationContext,
  fetchWorkspace,
  updateMiddlewareInstance,
  updateWorkspace,
} from "@/lib/api";
import type {
  AuthUser,
  MiddlewareInstance,
  OrganizationContext,
  Workspace,
  WorkspaceRuntimeMode,
} from "@/lib/types";

interface WorkspaceSettingsPageClientProps {
  orgSlug: string;
  workspaceSlug: string;
}

type SettingsState =
  | { status: "loading"; context: null; workspace: null; instances: [] }
  | {
      status: "ready";
      context: OrganizationContext;
      workspace: Workspace;
      instances: MiddlewareInstance[];
    }
  | { status: "error"; context: null; workspace: null; instances: []; error: string };

export function WorkspaceSettingsPageClient({
  orgSlug,
  workspaceSlug,
}: WorkspaceSettingsPageClientProps) {
  return (
    <AuthGate>
      {({ busyAuth, user, onLogout }) => (
        <WorkspaceSettingsRoute
          busyAuth={busyAuth}
          orgSlug={orgSlug}
          user={user}
          workspaceSlug={workspaceSlug}
          onLogout={onLogout}
        />
      )}
    </AuthGate>
  );
}

function WorkspaceSettingsRoute({
  busyAuth,
  orgSlug,
  user,
  workspaceSlug,
  onLogout,
}: {
  busyAuth: boolean;
  orgSlug: string;
  user: AuthUser;
  workspaceSlug: string;
  onLogout: () => void;
}) {
  const [state, setState] = useState<SettingsState>({
    status: "loading",
    context: null,
    workspace: null,
    instances: [],
  });
  const [runtimeMode, setRuntimeMode] =
    useState<WorkspaceRuntimeMode>("kubernetes");
  const [dockerApiUrl, setDockerApiUrl] = useState("");
  const [allowInsecureDocker, setAllowInsecureDocker] = useState(false);
  const [kubeconfig, setKubeconfig] = useState("");
  const [instanceKind, setInstanceKind] = useState("redis");
  const [instanceName, setInstanceName] = useState("");
  const [instanceNamespace, setInstanceNamespace] = useState("default");
  const [instanceConfig, setInstanceConfig] = useState("{}");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setState({ status: "loading", context: null, workspace: null, instances: [] });
    Promise.all([
      fetchOrganizationContext(orgSlug),
      fetchWorkspace(orgSlug, workspaceSlug),
      fetchMiddlewareInstances(orgSlug, workspaceSlug),
    ])
      .then(([context, workspace, instances]) => {
        if (cancelled) return;
        setRuntimeMode(workspace.runtime_config.mode ?? "kubernetes");
        setState({ status: "ready", context, workspace, instances });
      })
      .catch((caught) => {
        if (!cancelled) {
          setState({
            status: "error",
            context: null,
            workspace: null,
            instances: [],
            error:
              caught instanceof Error
                ? caught.message
                : "Failed to load workspace settings.",
          });
        }
      });

    return () => {
      cancelled = true;
    };
  }, [orgSlug, workspaceSlug]);

  async function handleRuntimeSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (state.status !== "ready" || busy) return;
    setBusy(true);
    setError(null);
    try {
      const workspace = await updateWorkspace(orgSlug, workspaceSlug, {
        name: null,
        archived: null,
        runtime_config:
          runtimeMode === "docker"
            ? {
                mode: "docker",
                docker_api_url: dockerApiUrl,
                allow_insecure_local_endpoint: allowInsecureDocker,
              }
            : { mode: "kubernetes", kubeconfig },
      });
      setState({ ...state, workspace });
      setDockerApiUrl("");
      setKubeconfig("");
    } catch (caught) {
      setError(
        caught instanceof Error ? caught.message : "Failed to save runtime.",
      );
    } finally {
      setBusy(false);
    }
  }

  async function handleInstanceSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (state.status !== "ready" || busy) return;
    setBusy(true);
    setError(null);
    try {
      const parsedConfig = instanceConfig.trim()
        ? JSON.parse(instanceConfig)
        : {};
      const instance = await createMiddlewareInstance(orgSlug, workspaceSlug, {
        kind: instanceKind,
        name: instanceName,
        namespace: instanceNamespace,
        desired_state: "enabled",
        config: parsedConfig,
      });
      setState({
        ...state,
        instances: [
          instance,
          ...state.instances.filter((current) => current.id !== instance.id),
        ],
      });
      setInstanceName("");
      setInstanceConfig("{}");
    } catch (caught) {
      setError(
        caught instanceof Error ? caught.message : "Failed to save instance.",
      );
    } finally {
      setBusy(false);
    }
  }

  async function setInstanceStatus(
    instance: MiddlewareInstance,
    status: MiddlewareInstance["status"] | "archive",
  ) {
    if (state.status !== "ready" || busy) return;
    setBusy(true);
    setError(null);
    try {
      const updated = await updateMiddlewareInstance(
        orgSlug,
        workspaceSlug,
        instance.id,
        status === "archive"
          ? {
              desired_state: null,
              status: null,
              config: null,
              archived: true,
            }
          : {
              desired_state: null,
              status,
              config: null,
              archived: null,
            },
      );
      setState({
        ...state,
        instances:
          status === "archive"
            ? state.instances.filter((current) => current.id !== updated.id)
            : [
                updated,
                ...state.instances.filter((current) => current.id !== updated.id),
              ],
      });
    } catch (caught) {
      setError(
        caught instanceof Error ? caught.message : "Failed to update instance.",
      );
    } finally {
      setBusy(false);
    }
  }

  if (state.status !== "ready") {
    return (
      <main className="login-shell login-loading" aria-busy="true">
        <div className="brand-lockup">
          <div className="brand-mark" aria-hidden="true">
            M
          </div>
          <div>
            <p className="section-kicker">{user.display_name || user.email}</p>
            <h1>{state.status === "error" ? state.error : "Loading settings"}</h1>
          </div>
        </div>
      </main>
    );
  }

  const canManage = state.context.permissions.includes("workspaces.manage");
  const runtime = state.workspace.runtime_config;

  return (
    <main className="settings-shell">
      <header className="app-header">
        <div className="brand-lockup">
          <div className="brand-mark" aria-hidden="true">
            M
          </div>
          <div>
            <p className="section-kicker">Workspace settings</p>
            <h1>{state.workspace.name}</h1>
            <p className="workspace-breadcrumb">
              {state.context.organization.name} / {state.workspace.slug}
            </p>
          </div>
        </div>
        <div className="header-actions">
          <a
            className="button button-outline"
            href={`/orgs/${orgSlug}/workspaces/${workspaceSlug}`}
          >
            <ArrowLeft aria-hidden="true" />
            Workspace
          </a>
          <button
            className="button button-danger"
            disabled={busyAuth}
            type="button"
            onClick={onLogout}
          >
            <LogOut aria-hidden="true" />
            Logout
          </button>
        </div>
      </header>

      {error && (
        <div className="inline-alert settings-alert" role="alert">
          {error}
        </div>
      )}

      <section className="settings-grid">
        <form className="settings-panel" onSubmit={handleRuntimeSubmit}>
          <div className="section-row">
            <div>
              <p className="section-kicker">Runtime</p>
              <h2>Connection profile</h2>
            </div>
            <span className="badge badge-outline">{runtime.status}</span>
          </div>

          <div className="runtime-summary compact">
            <div>
              <span>Mode</span>
              <strong>{runtime.mode ?? "unconfigured"}</strong>
              <p>{runtime.updated_at ?? "credentials not saved"}</p>
            </div>
            <div>
              <span>Endpoint</span>
              <strong>
                {runtime.docker?.endpoint_host ??
                  runtime.kubernetes?.cluster_server_host ??
                  runtime.kubernetes?.context_name ??
                  "--"}
              </strong>
              <p>secret values are not returned</p>
            </div>
          </div>

          <fieldset className="runtime-fieldset" disabled={!canManage || busy}>
            <legend>Runtime mode</legend>
            <div className="segmented-control" role="group">
              <button
                className={runtimeMode === "kubernetes" ? "active" : ""}
                type="button"
                onClick={() => setRuntimeMode("kubernetes")}
              >
                Kubernetes
              </button>
              <button
                className={runtimeMode === "docker" ? "active" : ""}
                type="button"
                onClick={() => setRuntimeMode("docker")}
              >
                Docker
              </button>
            </div>
          </fieldset>

          {runtimeMode === "docker" ? (
            <>
              <label htmlFor="settings-docker-url">Docker API URL</label>
              <input
                id="settings-docker-url"
                placeholder="https://docker.example.com:2376"
                value={dockerApiUrl}
                disabled={!canManage || busy}
                onChange={(event) => setDockerApiUrl(event.target.value)}
              />
              <label className="checkbox-row" htmlFor="settings-insecure-docker">
                <input
                  id="settings-insecure-docker"
                  checked={allowInsecureDocker}
                  disabled={!canManage || busy}
                  type="checkbox"
                  onChange={(event) =>
                    setAllowInsecureDocker(event.target.checked)
                  }
                />
                <span>Allow local HTTP endpoint</span>
              </label>
            </>
          ) : (
            <>
              <label htmlFor="settings-kubeconfig">Kubeconfig</label>
              <textarea
                id="settings-kubeconfig"
                placeholder="apiVersion: v1&#10;kind: Config"
                rows={10}
                value={kubeconfig}
                disabled={!canManage || busy}
                onChange={(event) => setKubeconfig(event.target.value)}
              />
            </>
          )}

          <button
            className="button button-primary"
            disabled={!canManage || busy}
            type="submit"
          >
            <Save aria-hidden="true" />
            {busy ? "Saving" : "Save runtime"}
          </button>
        </form>

        <section className="settings-panel">
          <div className="section-row">
            <div>
              <p className="section-kicker">Middleware</p>
              <h2>Instances</h2>
            </div>
            <span className="badge badge-outline">{state.instances.length}</span>
          </div>

          <form className="instance-form" onSubmit={handleInstanceSubmit}>
            <label htmlFor="instance-kind">Kind</label>
            <input
              id="instance-kind"
              value={instanceKind}
              disabled={!canManage || busy}
              onChange={(event) => setInstanceKind(event.target.value)}
            />
            <label htmlFor="instance-name">Name</label>
            <input
              id="instance-name"
              placeholder="cache"
              value={instanceName}
              disabled={!canManage || busy}
              onChange={(event) => setInstanceName(event.target.value)}
            />
            <label htmlFor="instance-namespace">Namespace</label>
            <input
              id="instance-namespace"
              value={instanceNamespace}
              disabled={!canManage || busy}
              onChange={(event) => setInstanceNamespace(event.target.value)}
            />
            <label htmlFor="instance-config">Config JSON</label>
            <textarea
              id="instance-config"
              rows={5}
              value={instanceConfig}
              disabled={!canManage || busy}
              onChange={(event) => setInstanceConfig(event.target.value)}
            />
            <button
              className="button button-primary"
              disabled={!canManage || busy}
              type="submit"
            >
              <Plus aria-hidden="true" />
              Add instance
            </button>
          </form>

          <div className="settings-instance-list">
            {state.instances.map((instance) => (
              <article className="instance-row" key={instance.id}>
                <div>
                  <strong>{instance.name}</strong>
                  <p>
                    {instance.kind} / {instance.namespace}
                  </p>
                </div>
                <div className="instance-actions">
                  <span className="badge badge-outline">{instance.status}</span>
                  <button
                    aria-pressed={instance.status === "running"}
                    className={`button ${
                      instance.status === "running"
                        ? "button-active"
                        : "button-outline"
                    }`}
                    disabled={!canManage || busy}
                    type="button"
                    onClick={() => setInstanceStatus(instance, "running")}
                  >
                    <Play aria-hidden="true" />
                    Running
                  </button>
                  <button
                    aria-pressed={instance.status === "degraded"}
                    className={`button ${
                      instance.status === "degraded"
                        ? "button-warning"
                        : "button-outline"
                    }`}
                    disabled={!canManage || busy}
                    type="button"
                    onClick={() => setInstanceStatus(instance, "degraded")}
                  >
                    <AlertTriangle aria-hidden="true" />
                    Degraded
                  </button>
                  <button
                    className="button button-danger"
                    disabled={!canManage || busy}
                    type="button"
                    onClick={() => setInstanceStatus(instance, "archive")}
                  >
                    <Archive aria-hidden="true" />
                    Archive
                  </button>
                </div>
              </article>
            ))}
            {state.instances.length === 0 && (
              <article className="empty-row">No middleware instances.</article>
            )}
          </div>
        </section>
      </section>
    </main>
  );
}
