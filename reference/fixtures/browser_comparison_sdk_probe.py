"""Offline integration probe. Caller must isolate networking and bound resources.

Use only pinned public modules and an empty browser context. The SDK creates
its own temporary device state. No account, inference request or token leaves
the page. This probes local execution, not server acceptance.
"""
import json

PAGE = "https://framer.com/projects/offline-sdk-fixture"
ORIGIN = "https://app.framerstatic.com/"


async def check(browser, emit, mint_expression, modules, public_key):
    assert type(public_key) is str and public_key.startswith("pk_")
    context = await browser.new_context(service_workers="block")
    counts = {"page": 0, "module": 0, "aborted": 0, "crash": 0, "disconnected": 0}

    def disconnected():
        counts["disconnected"] += 1

    def crashed():
        counts["crash"] += 1

    browser.on("disconnected", disconnected)

    async def route(request):
        url = request.request.url
        if url == PAGE:
            counts["page"] += 1
            await request.fulfill(status=200, content_type="text/html",
                                  body="<!doctype html><title>Offline SDK fixture</title>")
        elif url in modules:
            counts["module"] += 1
            await request.fulfill(status=200, content_type="text/javascript",
                                  headers={"Access-Control-Allow-Origin": "*"},
                                  body=modules[url])
        else:
            counts["aborted"] += 1
            await request.abort()

    await context.route("**/*", route)
    try:
        await context.add_init_script("window.castlePublishableAPIKey = " + json.dumps(public_key))
        page = await context.new_page()
        page.on("crash", crashed)
        await page.goto(PAGE, wait_until="domcontentloaded")
        emit("fixture_ready")
        setup = """async url => {
          const module = await import(url);
          await module.a();
          const deadline = performance.now() + 10000;
          while (!localStorage.getItem('__cuid') && performance.now() < deadline)
            await new Promise(resolve => setTimeout(resolve, 25));
          return !!localStorage.getItem('__cuid');
        }"""
        # Catch every phase in-page so SDK errors and token bytes stay private.
        expression = """async () => {
          let stage = 'device_ready';
          try {
            const device = localStorage.getItem('__cuid');
            if (typeof device !== 'string' || !device.length)
              return {stage, accepted: false};
            stage = 'mint';
            const page_url = location.href;
            const result = await (%s)({url: %s, entry: 'b',
              device, page_url});
            if (result && typeof result.failure_stage === 'string')
              return {stage: result.failure_stage, accepted: false};
            const accepted = result && typeof result.token === 'string' &&
              result.token.length > 0 && result.before === page_url && result.after === page_url;
            return {stage: 'finished', accepted: !!accepted};
          } catch (_) { return {stage, accepted: false}; }
        }""" % (mint_expression,
                 json.dumps(ORIGIN + "chunk-VPFQZMNG.mjs"))
        try:
            emit("setup_start")
            ready = await page.evaluate(setup, ORIGIN + "chunk-VPFQZMNG.mjs")
            if not ready:
                raise RuntimeError("fixture device absent")
            emit("setup_ready")
            emit("mint_start")
            result = await page.evaluate(expression)
        except Exception as error:
            name = type(error).__name__
            stage = {"TargetClosedError": "rpc_target_closed", "TimeoutError": "rpc_timeout"}.get(
                name, "rpc_error")
            result = {"stage": stage, "accepted": False}
        assert type(result) is dict and set(result) == {"stage", "accepted"}
        assert result["stage"] in {"import", "initialize", "device_ready", "mint",
                                   "finished", "pre_context", "dynamic_import",
                                   "export_validation", "wrapper_call", "post_context",
                                   "post_page", "post_device",
                                   "invalid_token", "rpc_target_closed", "rpc_timeout", "rpc_error"}
        assert type(result["accepted"]) is bool
        emit("sdk_" + result["stage"])
        emit("sdk_pass" if result["accepted"] else "sdk_refused")
    finally:
        await context.close()
        browser.remove_listener("disconnected", disconnected)
    emit("context_closed")
    return result, counts
