"use client";

import { useEffect, useMemo, useState, type FormEvent, type ReactNode } from "react";
import {
  addOrganizationMember,
  createOrganizationRole,
  createSystemRole,
  createUser,
  fetchOrganizationContext,
  fetchOrganizationMembers,
  fetchOrganizationRoles,
  fetchPermissionCatalog,
  fetchSystemRoles,
  fetchUsers,
  replaceOrganizationRolePermissions,
  replaceSystemRolePermissions,
  updateOrganizationMember,
  updateUser,
} from "@/lib/api";
import type {
  AuthUser,
  OrganizationContext,
  OrganizationMemberView,
  PermissionCatalogItem,
  PermissionKey,
  RbacRole,
} from "@/lib/types";

interface AdminShellProps {
  busyAuth: boolean;
  title: string;
  eyebrow: string;
  user: AuthUser;
  onLogout: () => void;
  children: ReactNode;
}

interface OrgAdminProps {
  busyAuth: boolean;
  orgSlug: string;
  user: AuthUser;
  onLogout: () => void;
}

interface SystemAdminProps {
  busyAuth: boolean;
  user: AuthUser;
  onLogout: () => void;
}

type LoadState = "loading" | "ready" | "error";

function AdminShell({
  busyAuth,
  title,
  eyebrow,
  user,
  onLogout,
  children,
}: AdminShellProps) {
  return (
    <main className="app-shell">
      <header className="app-header">
        <div className="brand-lockup">
          <div className="brand-mark" aria-hidden="true">
            M
          </div>
          <div>
            <p className="section-kicker">{eyebrow}</p>
            <h1>{title}</h1>
          </div>
        </div>
        <div className="header-actions">
          <div className="user-chip" aria-label="Signed in user">
            <strong>{user.display_name || user.email}</strong>
            <span>{user.email}</span>
          </div>
          <a className="button button-outline" href="/">
            Workspace
          </a>
          <button
            className="button button-outline logout-button"
            disabled={busyAuth}
            type="button"
            onClick={onLogout}
          >
            Logout
          </button>
        </div>
      </header>
      {children}
    </main>
  );
}

export function SystemUsersAdmin({
  busyAuth,
  user,
  onLogout,
}: SystemAdminProps) {
  const [state, setState] = useState<LoadState>("loading");
  const [users, setUsers] = useState<AuthUser[]>([]);
  const [roles, setRoles] = useState<RbacRole[]>([]);
  const [email, setEmail] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [password, setPassword] = useState("");
  const [roleId, setRoleId] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    Promise.all([fetchUsers(), fetchSystemRoles()])
      .then(([loadedUsers, loadedRoles]) => {
        if (cancelled) return;
        setUsers(loadedUsers);
        setRoles(loadedRoles);
        setRoleId(loadedRoles.find((role) => role.slug === "viewer")?.id ?? "");
        setState("ready");
      })
      .catch((caught) => {
        if (!cancelled) {
          setError(caught instanceof Error ? caught.message : "Failed to load users.");
          setState("error");
        }
      });

    return () => {
      cancelled = true;
    };
  }, []);

  async function handleCreate(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (!email || !password || !roleId) return;
    setError(null);
    try {
      const created = await createUser({
        email,
        password,
        display_name: displayName || null,
        role: null,
        system_role_id: roleId,
        active: true,
      });
      setUsers((current) => [...current, created].sort((a, b) => a.email.localeCompare(b.email)));
      setEmail("");
      setDisplayName("");
      setPassword("");
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Failed to create user.");
    }
  }

  async function handleUpdate(target: AuthUser, nextRoleId: string, active: boolean) {
    setError(null);
    try {
      const updated = await updateUser(target.id, {
        password: null,
        display_name: null,
        role: null,
        system_role_id: nextRoleId,
        active,
      });
      setUsers((current) =>
        current.map((candidate) => (candidate.id === updated.id ? updated : candidate)),
      );
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Failed to update user.");
    }
  }

  return (
    <AdminShell
      busyAuth={busyAuth}
      eyebrow="System RBAC"
      title="Users"
      user={user}
      onLogout={onLogout}
    >
      <section className="settings-grid">
        <form className="workspace-panel settings-panel" onSubmit={handleCreate}>
          <div className="panel-header">
            <div>
              <p className="section-kicker">Create user</p>
              <h2>Account access</h2>
            </div>
          </div>
          <div className="settings-form">
            <input placeholder="email@example.com" value={email} onChange={(e) => setEmail(e.target.value)} />
            <input placeholder="Display name" value={displayName} onChange={(e) => setDisplayName(e.target.value)} />
            <input placeholder="Temporary password" type="password" value={password} onChange={(e) => setPassword(e.target.value)} />
            <select value={roleId} onChange={(e) => setRoleId(e.target.value)}>
              {roles.map((role) => (
                <option key={role.id} value={role.id}>
                  {role.name}
                </option>
              ))}
            </select>
            <button className="button button-primary" type="submit">
              Create user
            </button>
          </div>
        </form>

        <section className="workspace-panel settings-panel">
          <div className="panel-header">
            <div>
              <p className="section-kicker">Directory</p>
              <h2>System users</h2>
            </div>
            <span className="badge badge-outline">{state}</span>
          </div>
          {error && <div className="inline-alert">{error}</div>}
          <div className="settings-table">
            {users.map((account) => (
              <div className="settings-row" key={account.id}>
                <div>
                  <strong>{account.display_name || account.email}</strong>
                  <span>{account.email}</span>
                </div>
                <select
                  value={account.system_role_id}
                  onChange={(e) => handleUpdate(account, e.target.value, account.active)}
                >
                  {roles.map((role) => (
                    <option key={role.id} value={role.id}>
                      {role.name}
                    </option>
                  ))}
                </select>
                <label className="toggle-line">
                  <input
                    checked={account.active}
                    type="checkbox"
                    onChange={(e) =>
                      handleUpdate(account, account.system_role_id, e.target.checked)
                    }
                  />
                  Active
                </label>
              </div>
            ))}
          </div>
        </section>
      </section>
    </AdminShell>
  );
}

export function SystemRolesAdmin({
  busyAuth,
  user,
  onLogout,
}: SystemAdminProps) {
  return (
    <RolesAdmin
      busyAuth={busyAuth}
      mode="system"
      title="System roles"
      user={user}
      onLogout={onLogout}
    />
  );
}

export function OrganizationRolesAdmin({
  busyAuth,
  orgSlug,
  user,
  onLogout,
}: OrgAdminProps) {
  return (
    <RolesAdmin
      busyAuth={busyAuth}
      mode="organization"
      orgSlug={orgSlug}
      title="Organization roles"
      user={user}
      onLogout={onLogout}
    />
  );
}

function RolesAdmin({
  busyAuth,
  mode,
  orgSlug,
  title,
  user,
  onLogout,
}: {
  busyAuth: boolean;
  mode: "system" | "organization";
  orgSlug?: string;
  title: string;
  user: AuthUser;
  onLogout: () => void;
}) {
  const [state, setState] = useState<LoadState>("loading");
  const [catalog, setCatalog] = useState<PermissionCatalogItem[]>([]);
  const [roles, setRoles] = useState<RbacRole[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [slug, setSlug] = useState("");
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const scope = mode === "system" ? "system" : "organization";
  const selectedRole = roles.find((role) => role.id === selectedId) ?? roles[0];
  const scopedCatalog = useMemo(
    () => catalog.filter((item) => item.scope_kind === scope),
    [catalog, scope],
  );

  useEffect(() => {
    let cancelled = false;
    const loadRoles =
      mode === "system"
        ? fetchSystemRoles()
        : fetchOrganizationRoles(orgSlug ?? "");
    Promise.all([fetchPermissionCatalog(), loadRoles])
      .then(([loadedCatalog, loadedRoles]) => {
        if (cancelled) return;
        setCatalog(loadedCatalog);
        setRoles(loadedRoles);
        setSelectedId(loadedRoles[0]?.id ?? "");
        setState("ready");
      })
      .catch((caught) => {
        if (!cancelled) {
          setError(caught instanceof Error ? caught.message : "Failed to load roles.");
          setState("error");
        }
      });

    return () => {
      cancelled = true;
    };
  }, [mode, orgSlug]);

  async function handleCreate(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (!slug || !name) return;
    setError(null);
    try {
      const payload = { slug, name, description: null, permissions: [] };
      const created =
        mode === "system"
          ? await createSystemRole(payload)
          : await createOrganizationRole(orgSlug ?? "", payload);
      setRoles((current) => [...current, created].sort((a, b) => a.slug.localeCompare(b.slug)));
      setSelectedId(created.id);
      setSlug("");
      setName("");
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Failed to create role.");
    }
  }

  async function togglePermission(permission: PermissionKey) {
    if (!selectedRole) return;
    const nextPermissions = selectedRole.permissions.includes(permission)
      ? selectedRole.permissions.filter((current) => current !== permission)
      : [...selectedRole.permissions, permission];
    setError(null);
    try {
      const updated =
        mode === "system"
          ? await replaceSystemRolePermissions(selectedRole.id, {
              permissions: nextPermissions,
            })
          : await replaceOrganizationRolePermissions(orgSlug ?? "", selectedRole.id, {
              permissions: nextPermissions,
            });
      setRoles((current) =>
        current.map((role) => (role.id === updated.id ? updated : role)),
      );
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Failed to update permissions.");
    }
  }

  return (
    <AdminShell
      busyAuth={busyAuth}
      eyebrow={mode === "system" ? "System RBAC" : orgSlug ?? "Organization"}
      title={title}
      user={user}
      onLogout={onLogout}
    >
      <section className="settings-grid">
        <form className="workspace-panel settings-panel" onSubmit={handleCreate}>
          <div className="panel-header">
            <div>
              <p className="section-kicker">New role</p>
              <h2>Create custom role</h2>
            </div>
          </div>
          <div className="settings-form">
            <input placeholder="slug" value={slug} onChange={(e) => setSlug(e.target.value)} />
            <input placeholder="Role name" value={name} onChange={(e) => setName(e.target.value)} />
            <button className="button button-primary" type="submit">
              Create role
            </button>
          </div>
        </form>

        <section className="workspace-panel settings-panel">
          <div className="panel-header">
            <div>
              <p className="section-kicker">Permissions</p>
              <h2>{selectedRole?.name ?? "No role selected"}</h2>
            </div>
            <span className="badge badge-outline">{state}</span>
          </div>
          {error && <div className="inline-alert">{error}</div>}
          <div className="role-layout">
            <nav className="role-list" aria-label="Roles">
              {roles.map((role) => (
                <button
                  className={`role-list-item ${role.id === selectedRole?.id ? "active" : ""}`}
                  key={role.id}
                  type="button"
                  onClick={() => setSelectedId(role.id)}
                >
                  <strong>{role.name}</strong>
                  <span>{role.slug}</span>
                </button>
              ))}
            </nav>
            <div className="permission-grid">
              {scopedCatalog.map((item) => (
                <label className="permission-item" key={item.key}>
                  <input
                    checked={selectedRole?.permissions.includes(item.key) ?? false}
                    type="checkbox"
                    onChange={() => togglePermission(item.key)}
                  />
                  <span>
                    <strong>{item.label}</strong>
                    <small>{item.key}</small>
                  </span>
                </label>
              ))}
            </div>
          </div>
        </section>
      </section>
    </AdminShell>
  );
}

export function OrganizationMembersAdmin({
  busyAuth,
  orgSlug,
  user,
  onLogout,
}: OrgAdminProps) {
  const [context, setContext] = useState<OrganizationContext | null>(null);
  const [members, setMembers] = useState<OrganizationMemberView[]>([]);
  const [roles, setRoles] = useState<RbacRole[]>([]);
  const [email, setEmail] = useState("");
  const [roleId, setRoleId] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    Promise.all([
      fetchOrganizationContext(orgSlug),
      fetchOrganizationMembers(orgSlug),
      fetchOrganizationRoles(orgSlug),
    ])
      .then(([loadedContext, loadedMembers, loadedRoles]) => {
        if (cancelled) return;
        setContext(loadedContext);
        setMembers(loadedMembers);
        setRoles(loadedRoles);
        setRoleId(loadedRoles.find((role) => role.slug === "viewer")?.id ?? "");
      })
      .catch((caught) => {
        if (!cancelled) {
          setError(caught instanceof Error ? caught.message : "Failed to load members.");
        }
      });

    return () => {
      cancelled = true;
    };
  }, [orgSlug]);

  async function handleAdd(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (!email || !roleId) return;
    setError(null);
    try {
      await addOrganizationMember(orgSlug, {
        email,
        role: null,
        role_id: roleId,
      });
      setMembers(await fetchOrganizationMembers(orgSlug));
      setEmail("");
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Failed to add member.");
    }
  }

  async function handleUpdate(member: OrganizationMemberView, nextRoleId: string, active: boolean) {
    setError(null);
    try {
      const membership = await updateOrganizationMember(orgSlug, member.user.id, {
        role: null,
        role_id: nextRoleId,
        active,
      });
      setMembers((current) =>
        current.map((candidate) =>
          candidate.user.id === member.user.id ? { ...candidate, membership } : candidate,
        ),
      );
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Failed to update member.");
    }
  }

  return (
    <AdminShell
      busyAuth={busyAuth}
      eyebrow={context?.organization.name ?? orgSlug}
      title="Organization members"
      user={user}
      onLogout={onLogout}
    >
      <section className="settings-grid">
        <form className="workspace-panel settings-panel" onSubmit={handleAdd}>
          <div className="panel-header">
            <div>
              <p className="section-kicker">Add member</p>
              <h2>Existing user</h2>
            </div>
          </div>
          <div className="settings-form">
            <input placeholder="user@example.com" value={email} onChange={(e) => setEmail(e.target.value)} />
            <select value={roleId} onChange={(e) => setRoleId(e.target.value)}>
              {roles.map((role) => (
                <option key={role.id} value={role.id}>
                  {role.name}
                </option>
              ))}
            </select>
            <button className="button button-primary" type="submit">
              Add member
            </button>
          </div>
        </form>

        <section className="workspace-panel settings-panel">
          <div className="panel-header">
            <div>
              <p className="section-kicker">Members</p>
              <h2>{members.length} accounts</h2>
            </div>
          </div>
          {error && <div className="inline-alert">{error}</div>}
          <div className="settings-table">
            {members.map((member) => (
              <div className="settings-row" key={member.membership.id}>
                <div>
                  <strong>{member.user.display_name || member.user.email}</strong>
                  <span>{member.user.email}</span>
                </div>
                <select
                  value={member.membership.role_id}
                  onChange={(e) =>
                    handleUpdate(member, e.target.value, member.membership.active)
                  }
                >
                  {roles.map((role) => (
                    <option key={role.id} value={role.id}>
                      {role.name}
                    </option>
                  ))}
                </select>
                <label className="toggle-line">
                  <input
                    checked={member.membership.active}
                    type="checkbox"
                    onChange={(e) =>
                      handleUpdate(member, member.membership.role_id, e.target.checked)
                    }
                  />
                  Active
                </label>
              </div>
            ))}
          </div>
        </section>
      </section>
    </AdminShell>
  );
}
