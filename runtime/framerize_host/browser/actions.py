"""Owned browser interaction."""
from .scripts import SAFE_CLICK_JS

async def human_burst(page, rng, allow_click=True, rpc=None):
    """One small human-like activity burst: a CURVED mouse path with
    ease-in-out timing, sometimes a wheel scroll, rarely a verified-safe
    click. Castle's docs name "straight vector mouse movements" and "time
    patterns between interactions" as bot tells — a single
    mouse.move(steps=N) lays N collinear points, exactly that tell. So the
    path is a quadratic bezier with smoothstep easing (humans accelerate and
    decelerate), jitter, and occasional mid-path hesitation. Each burst is
    ~200-500ms — cheap enough to keep every page's behavioral buffer warm."""
    if rpc is None:
        async def rpc(call, *args, **kwargs):
            return await call(*args, **kwargs)
    x0, y0 = rng.uniform(60, 1100), rng.uniform(60, 560)
    x1, y1 = rng.uniform(60, 1100), rng.uniform(60, 560)
    cx, cy = (x0 + x1) / 2 + rng.uniform(-220, 220), (y0 + y1) / 2 + rng.uniform(-160, 160)
    n = rng.randint(10, 22)
    hesitate_at = rng.randint(2, n - 2) if rng.random() < 0.25 else -1
    for i in range(n + 1):
        t = i / n
        te = t * t * (3 - 2 * t)  # smoothstep ease-in-out
        px = (1 - te) ** 2 * x0 + 2 * (1 - te) * te * cx + te ** 2 * x1
        py = (1 - te) ** 2 * y0 + 2 * (1 - te) * te * cy + te ** 2 * y1
        await rpc(page.mouse.move, px + rng.uniform(-1.5, 1.5), py + rng.uniform(-1.5, 1.5), steps=1)
        if i == hesitate_at:
            await rpc(page.wait_for_timeout, rng.uniform(90, 260))
        else:
            await rpc(page.wait_for_timeout, rng.uniform(4, 18))
    if rng.random() < 0.4:
        await rpc(page.mouse.wheel, 0, rng.uniform(-320, 320))
        x2, y2 = x1 + rng.uniform(-90, 90), y1 + rng.uniform(-60, 60)
        await rpc(page.mouse.move, x2, y2, steps=rng.randint(3, 8))
    if allow_click and rng.random() < 0.3:
        try:
            if await rpc(page.evaluate, SAFE_CLICK_JS, [int(x1), int(y1)]):
                await rpc(page.mouse.click, int(x1), int(y1), delay=rng.randint(25, 95))
        except Exception:
            pass
