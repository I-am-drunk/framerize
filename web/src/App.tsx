import { useEffect, useState } from "react";
import {
  Account,
  AccountState,
  ChatTurn,
  beginLogin,
  completeLogin,
  listAccounts,
  run,
} from "./api";

// One status -> visual mapping, in one place.
const STATUS: Record<AccountState, { tint: string; label: string }> = {
  ready: { tint: "#34c759", label: "Ready" },
  authenticating: { tint: "#0a84ff", label: "Authenticating" },
  cooling_down: { tint: "#ff9f0a", label: "Cooling down (Castle 403)" },
  exhausted: { tint: "#ff453a", label: "Exhausted" },
  quarantined: { tint: "#bf5af2", label: "Quarantined" },
  disconnected: { tint: "#8e8e93", label: "Disconnected" },
};

type Page = "accounts" | "add" | "logs" | "test";

export function App() {
  const [page, setPage] = useState<Page>("accounts");
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [selected, setSelected] = useState<string | null>(null);

  const refresh = () => listAccounts().then(setAccounts).catch(() => setAccounts([]));
  useEffect(() => {
    refresh();
  }, []);

  const current = accounts.find((a) => a.id === selected) ?? null;

  return (
    <div className="app">
      <nav className="sidebar">
        <div className="pages">
          {(["accounts", "add", "logs", "test"] as Page[]).map((p) => (
            <button key={p} className={page === p ? "active" : ""} onClick={() => setPage(p)}>
              {p === "add" ? "Add account" : p[0].toUpperCase() + p.slice(1)}
            </button>
          ))}
        </div>
        {page === "accounts" && (
          <ul className="pool">
            {accounts.map((a) => (
              <li key={a.id} className={a.id === selected ? "active" : ""} onClick={() => setSelected(a.id)}>
                <span className="dot" style={{ background: STATUS[a.state].tint }} />
                {a.alias}
              </li>
            ))}
            {accounts.length === 0 && <li className="empty">No accounts</li>}
          </ul>
        )}
      </nav>
      <main className="detail">
        {page === "accounts" && <AccountDetail account={current} />}
        {page === "add" && <AddAccount onDone={refresh} />}
        {page === "test" && <TestPage accounts={accounts} />}
        {page === "logs" && <p className="muted">Logs stream — coming with the daemon.</p>}
      </main>
    </div>
  );
}

function AccountDetail({ account }: { account: Account | null }) {
  if (!account) return <p className="muted">Select an account.</p>;
  const s = STATUS[account.state];
  return (
    <section className="form">
      <header>
        <div className="avatar">{account.alias[0]?.toUpperCase()}</div>
        <div>
          <h1>{account.alias}</h1>
          <span className="badge" style={{ color: s.tint, background: s.tint + "24" }}>{s.label}</span>
        </div>
      </header>
      <dl>
        <Row k="Framer user" v={account.framer_user_id} />
        <Row k="Workspace (team)" v={account.current_team ?? "—"} />
        <Row k="Project" v={account.current_project ?? "—"} />
        <Row
          k="Credits"
          v={account.credit ? `${account.credit.available.toFixed(1)} across ${account.credit.funds} fund(s)` : "—"}
        />
        <Row k="Consecutive failures" v={String(account.consecutive_failures)} />
      </dl>
    </section>
  );
}

/** The magic-link flow: email → check inbox → paste the #token from the link. */
function AddAccount({ onDone }: { onDone: () => void }) {
  const [email, setEmail] = useState("");
  const [token, setToken] = useState("");
  const [phase, setPhase] = useState<"email" | "token">("email");
  const [accountId, setAccountId] = useState<string | null>(null);
  const [msg, setMsg] = useState("");
  const [busy, setBusy] = useState(false);

  async function sendLink() {
    setBusy(true);
    setMsg("");
    try {
      // The daemon assigns the account id when it allocates the webview session.
      const id = accountId ?? crypto.randomUUID();
      setAccountId(id);
      await beginLogin(id, email);
      setPhase("token");
      setMsg("Check your inbox — paste the #token from the Framer link (expires in 1 hour).");
    } catch (e) {
      setMsg(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function finish() {
    if (!accountId) return;
    setBusy(true);
    setMsg("");
    try {
      await completeLogin(accountId, token.trim());
      setMsg("Signed in.");
      onDone();
    } catch (e) {
      setMsg(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="form">
      <h1>Add account</h1>
      <p className="muted">
        Framer signs in by email link. Both steps mint a Castle token inside this
        account's own browser session — accounts are devices.
      </p>
      {phase === "email" ? (
        <>
          <input
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="Framer account email"
            type="email"
          />
          <button disabled={busy || !email} onClick={sendLink}>
            {busy ? "Sending…" : "Email me a sign-in link"}
          </button>
        </>
      ) : (
        <>
          <input
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder="token from framer.com/login#token=…"
          />
          <button disabled={busy || !token} onClick={finish}>
            {busy ? "Signing in…" : "Complete sign-in"}
          </button>
        </>
      )}
      {msg && <p className="muted">{msg}</p>}
    </section>
  );
}

function TestPage({ accounts }: { accounts: Account[] }) {
  const [account, setAccount] = useState(accounts[0]?.id ?? "");
  const [model, setModel] = useState("gpt-4.1");
  const [history, setHistory] = useState<ChatTurn[]>([]);
  const [prompt, setPrompt] = useState("");
  const [chatSession, setChatSession] = useState<string | undefined>();
  const [out, setOut] = useState("");
  const [busy, setBusy] = useState(false);

  async function go() {
    setBusy(true);
    try {
      const r = await run(account, prompt, model, chatSession, history);
      setChatSession(r.chat_session);
      setHistory([...history, { role: "user", content: prompt }, { role: "assistant", content: r.text }]);
      setOut(
        `${r.text}\n\n— session ${r.chat_session}` +
          (r.credits_remaining != null ? ` · ${r.credits_remaining.toFixed(1)} credits left` : ""),
      );
    } catch (e) {
      setOut(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="form">
      <h1>Test</h1>
      <select value={account} disabled={busy} onChange={(e) => {
        setAccount(e.target.value); setChatSession(undefined); setHistory([]);
      }}>
        {accounts.map((a) => (
          <option key={a.id} value={a.id}>{a.alias}</option>
        ))}
      </select>
      <label>Model
        <input value={model} disabled={busy} onChange={(e) => setModel(e.target.value)} />
      </label>
      <p className="muted">Custom route. Metered models are unavailable here.</p>
      <textarea value={prompt} onChange={(e) => setPrompt(e.target.value)} placeholder="Prompt" rows={4} />
      <button disabled={busy || !account} onClick={go}>{busy ? "Running…" : "Run"}</button>
      {chatSession && (
        <button className="link" disabled={busy} onClick={() => {
          setChatSession(undefined); setHistory([]);
        }}>New chat session</button>
      )}
      {out && <pre className="out">{out}</pre>}
    </section>
  );
}

function Row({ k, v }: { k: string; v: string }) {
  return (
    <div className="row">
      <dt>{k}</dt>
      <dd>{v}</dd>
    </div>
  );
}
