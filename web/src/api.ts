// Thin typed client. Same-origin only — the daemon serves this bundle and the API.

export type AccountState =
  | "disconnected"
  | "authenticating"
  | "ready"
  | "cooling_down"
  | "exhausted"
  | "quarantined";

export interface CreditSnapshot {
  available: number;
  funds: number;
}

export interface Account {
  id: string;
  alias: string;
  framer_user_id: string;
  current_team: string | null; // workspace teamId — credits live here
  current_project: string | null; // project inference is scoped to
  state: AccountState;
  credit: CreditSnapshot | null;
  consecutive_failures: number;
}

export interface RunOutcome {
  text: string;
  usage: { input_tokens: number; output_tokens: number };
  chat_session: string;
  credits_remaining: number | null;
}

export async function listAccounts(): Promise<Account[]> {
  const r = await fetch("/api/accounts");
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}

/** Step 1 of adding an account: Framer emails a magic link to `email`. */
export async function beginLogin(accountId: string, email: string): Promise<void> {
  const r = await fetch("/api/accounts/begin", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ account_id: accountId, email }),
  });
  if (!r.ok) throw new Error(await r.text());
}

/** Step 2: exchange the #token uuid from the emailed link for the session cookie. */
export async function completeLogin(accountId: string, emailToken: string): Promise<void> {
  const r = await fetch("/api/accounts/complete", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ account_id: accountId, email_token: emailToken }),
  });
  if (!r.ok) throw new Error(await r.text());
}

export async function run(
  accountId: string,
  prompt: string,
  chatSession?: string,
): Promise<RunOutcome> {
  const r = await fetch("/api/run", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ account_id: accountId, prompt, chat_session: chatSession }),
  });
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}
