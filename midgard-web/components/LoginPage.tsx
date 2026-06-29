"use client";

import { type FormEvent, useState } from "react";

interface LoginPageProps {
  busy: boolean;
  error: string | null;
  onSubmit: (email: string, password: string) => void;
}

export function LoginPage({ busy, error, onSubmit }: LoginPageProps) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    onSubmit(email, password);
  }

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
        <div>
          <p className="section-kicker">Sign in</p>
          <h2 id="login-form-title">Enter your Midgard credentials</h2>
        </div>

        {error && (
          <div className="inline-alert login-alert" role="alert">
            {error}
          </div>
        )}

        <form className="login-form" onSubmit={handleSubmit}>
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
            autoComplete="current-password"
            name="password"
            placeholder="Password"
            type="password"
            value={password}
            disabled={busy}
            onChange={(event) => setPassword(event.target.value)}
          />

          <button className="button button-primary" disabled={busy} type="submit">
            {busy ? "Signing in" : "Sign in"}
          </button>
        </form>
      </section>
    </main>
  );
}
