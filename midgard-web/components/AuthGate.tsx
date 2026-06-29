"use client";

import { useEffect, useState } from "react";
import { LoginPage } from "@/components/LoginPage";
import { WorkspaceShell } from "@/components/WorkspaceShell";
import { fetchCurrentUser, login, logout } from "@/lib/api";
import type { AuthUser } from "@/lib/types";

type AuthState =
  | { status: "loading"; user: null; error: null }
  | { status: "anonymous"; user: null; error: string | null }
  | { status: "authenticated"; user: AuthUser; error: null };

export function AuthGate() {
  const [auth, setAuth] = useState<AuthState>({
    status: "loading",
    user: null,
    error: null,
  });
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const controller = new AbortController();
    const timeout = new Promise<null>((resolve) => {
      window.setTimeout(() => {
        controller.abort();
        resolve(null);
      }, 1200);
    });

    Promise.race([fetchCurrentUser({ signal: controller.signal }), timeout])
      .then((user) => {
        if (!user) {
          if (!cancelled) {
            setAuth({ status: "anonymous", user: null, error: null });
          }
          return;
        }
        if (!cancelled) {
          setAuth({ status: "authenticated", user, error: null });
        }
      })
      .catch(() => {
        if (!cancelled) {
          setAuth({ status: "anonymous", user: null, error: null });
        }
      });

    return () => {
      cancelled = true;
      controller.abort();
    };
  }, []);

  async function handleLogin(email: string, password: string) {
    if (busy) return;
    setBusy(true);
    setAuth({ status: "anonymous", user: null, error: null });

    try {
      const user = await login(email, password);
      setAuth({ status: "authenticated", user, error: null });
    } catch {
      setAuth({
        status: "anonymous",
        user: null,
        error: "Invalid email or password.",
      });
    } finally {
      setBusy(false);
    }
  }

  async function handleLogout() {
    setBusy(true);
    try {
      await logout();
    } finally {
      setBusy(false);
      setAuth({ status: "anonymous", user: null, error: null });
    }
  }

  if (auth.status === "loading") {
    return (
      <main className="login-shell login-loading" aria-busy="true">
        <div className="brand-lockup">
          <div className="brand-mark" aria-hidden="true">
            M
          </div>
          <div>
            <p className="section-kicker">Midgard</p>
            <h1>Checking session</h1>
          </div>
        </div>
      </main>
    );
  }

  if (auth.status === "authenticated") {
    return (
      <WorkspaceShell
        busyAuth={busy}
        user={auth.user}
        onLogout={handleLogout}
      />
    );
  }

  return <LoginPage busy={busy} error={auth.error} onSubmit={handleLogin} />;
}
