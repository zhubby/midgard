"use client";

import { type ReactNode, useEffect, useState } from "react";
import { LoginPage } from "@/components/LoginPage";
import { fetchCurrentUser, login, logout, register } from "@/lib/api";
import type { AuthContext, AuthUser } from "@/lib/types";

type AuthState =
  | { status: "loading"; auth: null; error: null }
  | { status: "anonymous"; auth: null; error: string | null }
  | { status: "authenticated"; auth: AuthContext; error: null };

const SESSION_CHECK_TIMEOUT_MS = 1200;
const AUTH_CACHE_KEY = "midgard.auth.context";

let cachedAuth: AuthContext | null = null;

interface AuthGateProps {
  children: (props: {
    auth: AuthContext;
    busyAuth: boolean;
    user: AuthUser;
    onLogout: () => void;
  }) => ReactNode;
}

export function AuthGate({ children }: AuthGateProps) {
  const [auth, setAuth] = useState<AuthState>(() => {
    if (cachedAuth) {
      return { status: "authenticated", auth: cachedAuth, error: null };
    }

    return { status: "loading", auth: null, error: null };
  });
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    let requestId = 0;
    let controller: AbortController | null = null;
    let timeoutId: number | null = null;

    function clearPendingCheck() {
      if (timeoutId !== null) {
        window.clearTimeout(timeoutId);
        timeoutId = null;
      }
      controller?.abort();
      controller = null;
    }

    function checkSession() {
      if (cancelled) return;

      clearPendingCheck();
      const cached = readCachedAuth();
      if (cached) {
        setAuth({ status: "authenticated", auth: cached, error: null });
      }

      const currentRequest = requestId + 1;
      requestId = currentRequest;
      controller = new AbortController();
      let timedOut = false;

      timeoutId = window.setTimeout(() => {
        if (cancelled || currentRequest !== requestId) return;

        timedOut = true;
        controller?.abort();
        if (!readCachedAuth()) {
          setAuth({ status: "anonymous", auth: null, error: null });
        }
      }, SESSION_CHECK_TIMEOUT_MS);

      fetchCurrentUser({ signal: controller.signal })
        .then((user) => {
          if (cancelled || currentRequest !== requestId) {
            return;
          }
          if (timeoutId !== null) {
            window.clearTimeout(timeoutId);
            timeoutId = null;
          }
          cacheAuth(user);
          setAuth({ status: "authenticated", auth: user, error: null });
        })
        .catch(() => {
          if (cancelled || currentRequest !== requestId) {
            return;
          }
          if (timeoutId !== null) {
            window.clearTimeout(timeoutId);
            timeoutId = null;
          }
          if (timedOut && readCachedAuth()) {
            return;
          }
          clearCachedAuth();
          setAuth({ status: "anonymous", auth: null, error: null });
        });
    }

    function handlePageShow(event: PageTransitionEvent) {
      if (event.persisted) {
        checkSession();
      }
    }

    checkSession();
    window.addEventListener("pageshow", handlePageShow);

    return () => {
      cancelled = true;
      window.removeEventListener("pageshow", handlePageShow);
      clearPendingCheck();
    };
  }, []);

  async function handleLogin(email: string, password: string) {
    if (busy) return;
    setBusy(true);
    setAuth({ status: "anonymous", auth: null, error: null });

    try {
      const user = await login(email, password);
      cacheAuth(user);
      setAuth({ status: "authenticated", auth: user, error: null });
    } catch {
      clearCachedAuth();
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
      cacheAuth(user);
      setAuth({ status: "authenticated", auth: user, error: null });
    } catch (caught) {
      clearCachedAuth();
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
      clearCachedAuth();
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

function cacheAuth(auth: AuthContext) {
  cachedAuth = auth;
  if (typeof window === "undefined") return;

  try {
    window.sessionStorage.setItem(AUTH_CACHE_KEY, JSON.stringify(auth));
  } catch {
    // Session storage is an optimization for browser history restores.
  }
}

function clearCachedAuth() {
  cachedAuth = null;
  if (typeof window === "undefined") return;

  try {
    window.sessionStorage.removeItem(AUTH_CACHE_KEY);
  } catch {
    // Ignore storage failures; auth state is still cleared in memory.
  }
}

function readCachedAuth() {
  if (cachedAuth) return cachedAuth;
  if (typeof window === "undefined") return null;

  try {
    const value = window.sessionStorage.getItem(AUTH_CACHE_KEY);
    if (!value) return null;

    const parsed: unknown = JSON.parse(value);
    if (isAuthContext(parsed)) {
      cachedAuth = parsed;
      return parsed;
    }
  } catch {
    clearCachedAuth();
  }

  return null;
}

function isAuthContext(value: unknown): value is AuthContext {
  if (!value || typeof value !== "object") return false;

  const candidate = value as {
    user?: { email?: unknown };
    system_permissions?: unknown;
  };

  return (
    typeof candidate.user?.email === "string" &&
    Array.isArray(candidate.system_permissions)
  );
}
