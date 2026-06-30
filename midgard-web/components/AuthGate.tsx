"use client";

import { type ReactNode, useEffect, useState } from "react";
import { LoginPage } from "@/components/LoginPage";
import { fetchCurrentUser, login, logout, register } from "@/lib/api";
import type { AuthContext, AuthUser } from "@/lib/types";

type AuthState =
  | { status: "loading"; auth: null; error: null }
  | { status: "anonymous"; auth: null; error: string | null }
  | { status: "authenticated"; auth: AuthContext; error: null };

interface AuthGateProps {
  children: (props: {
    auth: AuthContext;
    busyAuth: boolean;
    user: AuthUser;
    onLogout: () => void;
  }) => ReactNode;
}

export function AuthGate({ children }: AuthGateProps) {
  const [auth, setAuth] = useState<AuthState>({
    status: "loading",
    auth: null,
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
            setAuth({ status: "anonymous", auth: null, error: null });
          }
          return;
        }
        if (!cancelled) {
          setAuth({ status: "authenticated", auth: user, error: null });
        }
      })
      .catch(() => {
        if (!cancelled) {
          setAuth({ status: "anonymous", auth: null, error: null });
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
    setAuth({ status: "anonymous", auth: null, error: null });

    try {
      const user = await login(email, password);
      setAuth({ status: "authenticated", auth: user, error: null });
    } catch {
      setAuth({
        status: "anonymous",
        auth: null,
        error: "Invalid email or password.",
      });
    } finally {
      setBusy(false);
    }
  }

  async function handleRegister(
    displayName: string,
    email: string,
    password: string,
  ) {
    if (busy) return;
    setBusy(true);
    setAuth({ status: "anonymous", auth: null, error: null });

    try {
      const user = await register({
        email,
        password,
        display_name: displayName.trim() || null,
      });
      setAuth({ status: "authenticated", auth: user, error: null });
    } catch (caught) {
      setAuth({
        status: "anonymous",
        auth: null,
        error:
          caught instanceof Error
            ? registrationErrorMessage(caught.message)
            : "Unable to create account.",
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
      setAuth({ status: "anonymous", auth: null, error: null });
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
      <>
        {children({
          auth: auth.auth,
          busyAuth: busy,
          user: auth.auth.user,
          onLogout: handleLogout,
        })}
      </>
    );
  }

  return (
    <LoginPage
      busy={busy}
      error={auth.error}
      onLogin={handleLogin}
      onRegister={handleRegister}
    />
  );
}

function registrationErrorMessage(message: string) {
  if (message.includes("already exists")) {
    return "An account already exists for that email.";
  }
  if (message.includes("password must")) {
    return "Password must be at least 8 characters.";
  }
  if (message.includes("email is required")) {
    return "Email is required.";
  }

  return "Unable to create account.";
}
