import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test, mock, afterEach } from "node:test";
import ts from "typescript";

const source = await readFile(new URL("../src/api.ts", import.meta.url), "utf8");
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
});
const { run } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString("base64")}`);
afterEach(() => mock.restoreAll());

test("an empty JSON error preserves its HTTP status", async () => {
  mock.method(globalThis, "fetch", async () => Response.json(null, { status: 502 }));
  await assert.rejects(run("fixture-account", "next", "gpt-4.1", "fixture-chat"),
    /Request failed with HTTP 502/);
});

test("an upstream credit refusal remains explicit and makes no fallback request", async () => {
  const fetch = mock.method(globalThis, "fetch", async () => Response.json(
    { error: { message: "The workspace has exhausted its credits." } }, { status: 402 }));
  await assert.rejects(run("fixture-account", "next", "gpt-4.1", "fixture-chat"),
    /The workspace has exhausted its credits/);
  assert.equal(fetch.mock.callCount(), 1);
});

test("a plain-text admission error is not hidden by a JSON parsing failure", async () => {
  const fetch = mock.method(globalThis, "fetch", async () => new Response(
    "Available RAM is below the admission threshold", { status: 503 }));
  await assert.rejects(run("fixture-account", "next", "gpt-4.1", "fixture-chat"),
    /Available RAM is below the admission threshold/);
  assert.equal(fetch.mock.callCount(), 1);
});

test("the app preserves the selected account, model and history on the custom route", async () => {
  const history = [{ role: "user", content: "earlier" }, { role: "assistant", content: "answer" }];
  const fetch = mock.method(globalThis, "fetch", async (url, init) => {
    assert.equal(url, "/openai/v1/chat/completions");
    assert.equal(init.headers["x-framerize-account"], "fixture-account");
    assert.equal(init.headers["x-framerize-session"], "fixture-chat");
    assert.equal(init.headers["x-framerize-mode"], undefined);
    assert.deepEqual(JSON.parse(init.body), { model: "claude-fable-5-1", stream: false,
      messages: [...history, { role: "user", content: "next" }] });
    return Response.json({ choices: [{ message: { content: "done" } }],
      usage: { prompt_tokens: 12, completion_tokens: 4 } });
  });
  const result = await run("fixture-account", "next", "claude-fable-5-1", "fixture-chat", history);
  assert.equal(result.text, "done");
  assert.equal(result.chat_session, "fixture-chat");
  assert.deepEqual(result.usage, { input_tokens: 12, output_tokens: 4 });
  assert.equal(fetch.mock.callCount(), 1);
});
