"use client";

import { type FormEvent, useState } from "react";

interface LoginPageProps {
  busy: boolean;
  error: string | null;
  onLogin: (email: string, password: string) => void;
  onRegister: (displayName: string, email: string, password: string) => void;
}

type AuthMode = "login" | "register";

export function LoginPage({
  busy,
  error,
  onLogin,
  onRegister,
}: LoginPageProps) {
  const [mode, setMode] = useState<AuthMode>("login");
  const [displayName, setDisplayName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [localError, setLocalError] = useState<string | null>(null);

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setLocalError(null);

    if (mode === "register") {
      if (password.length < 8) {
        setLocalError("Password must be at least 8 characters.");
        return;
      }
      if (password !== confirmPassword) {
        setLocalError("Passwords do not match.");
        return;
      }
      onRegister(displayName, email, password);
      return;
    }

    onLogin(email, password);
  }

  function switchMode(nextMode: AuthMode) {
    setMode(nextMode);
    setLocalError(null);
  }

  const activeError = localError ?? error;
  const isRegistering = mode === "register";

  return (
    <main className="login-shell">
      <section className="login-brief" aria-labelledby="login-title">
        <div className="brand-lockup">
          <div className="brand-mark" aria-hidden="true">
            M
          </div>
          <div>
            <p className="section-kicker">Midgard</p>
            <h1 id="login-title">Middleware operations console</h1>
          </div>
        </div>

        <div className="login-status-grid" aria-label="Security posture">
          <article>
            <span>Access</span>
            <strong>Session guarded</strong>
            <p>Workspace APIs require an authenticated operator session.</p>
          </article>
          <article>
            <span>Roles</span>
            <strong>Admin / Operator / Viewer</strong>
            <p>Mutating actions stay separated from read-only monitoring.</p>
          </article>
          <article>
            <span>Runtime</span>
            <strong>Agent events live</strong>
            <p>SSE updates resume after sign-in with the current workspace state.</p>
          </article>
        </div>
      </section>

      <section className="login-card" aria-labelledby="login-form-title">
        <div className="login-card-heading">
          <div>
            <p className="section-kicker">
              {isRegistering ? "Register" : "Sign in"}
            </p>
            <h2 id="login-form-title">
              {isRegistering
                ? "Create your Midgard account"
                : "Enter your Midgard credentials"}
            </h2>
          </div>
          <div className="auth-mode-toggle" role="group" aria-label="Authentication mode">
            <button
              className={!isRegistering ? "active" : ""}
              disabled={busy}
              type="button"
              aria-pressed={!isRegistering}
              onClick={() => switchMode("login")}
            >
              Sign in
            </button>
            <button
              className={isRegistering ? "active" : ""}
              disabled={busy}
              type="button"
              aria-pressed={isRegistering}
              onClick={() => switchMode("register")}
            >
              Create account
            </button>
          </div>
        </div>

        {activeError && (
          <div className="inline-alert login-alert" role="alert">
            {activeError}
          </div>
        )}

        <form className="login-form" onSubmit={handleSubmit}>
          {isRegistering && (
            <>
              <label htmlFor="display-name">Display name</label>
              <input
                id="display-name"
                autoComplete="name"
                name="display-name"
                placeholder="Platform Operator"
                value={displayName}
                disabled={busy}
                onChange={(event) => setDisplayName(event.target.value)}
              />
            </>
          )}

          <label htmlFor="email">Email</label>
          <input
            id="email"
            autoComplete="email"
            inputMode="email"
            name="email"
            placeholder="operator@example.com"
            type="email"
            value={email}
            disabled={busy}
            onChange={(event) => setEmail(event.target.value)}
          />

          <label htmlFor="password">Password</label>
          <input
            id="password"
            autoComplete={isRegistering ? "new-password" : "current-password"}
            name="password"
            placeholder="Password"
            type="password"
            value={password}
            disabled={busy}
            onChange={(event) => setPassword(event.target.value)}
          />

          {isRegistering && (
            <>
              <label htmlFor="confirm-password">Confirm password</label>
              <input
                id="confirm-password"
                autoComplete="new-password"
                name="confirm-password"
                placeholder="Confirm password"
                type="password"
                value={confirmPassword}
                disabled={busy}
                onChange={(event) => setConfirmPassword(event.target.value)}
              />
            </>
          )}

          <button className="button button-primary" disabled={busy} type="submit">
            {busy
              ? isRegistering
                ? "Creating account"
                : "Signing in"
              : isRegistering
                ? "Create account"
                : "Sign in"}
          </button>
        </form>
      </section>
    </main>
  );
}
