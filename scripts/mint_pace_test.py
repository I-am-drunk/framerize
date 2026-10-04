"""Verify the per-account mint lock + pacing in isolation (no network)."""
import asyncio, time, sys
MINT_GAP_S = 0.25

class Acct:
    def __init__(self):
        self.mint_lock = asyncio.Lock()
        self.last_mint = 0.0

stamps = []

async def mint(acc, i):
    # Mirrors Host._mint exactly.
    async with acc.mint_lock:
        gap = MINT_GAP_S - (time.monotonic() - acc.last_mint)
        if gap > 0:
            await asyncio.sleep(gap)
        t = time.monotonic()
        await asyncio.sleep(0.08)        # a real mint measured ~82ms
        acc.last_mint = time.monotonic()
        stamps.append((i, t))

async def main():
    acc = Acct()
    t0 = time.monotonic()
    await asyncio.gather(*(mint(acc, i) for i in range(12)))
    stamps.sort(key=lambda x: x[1])
    gaps = [stamps[i][1] - stamps[i-1][1] for i in range(1, len(stamps))]
    print(f"12 concurrent mints on one account, wall={time.monotonic()-t0:.2f}s")
    print(f"min gap between mint STARTS = {min(gaps)*1000:.0f}ms (floor should be ~{MINT_GAP_S*1000:.0f}+80ms)")
    print("overlapping mints:", sum(1 for g in gaps if g < 0.05))
    ok = min(gaps) >= MINT_GAP_S
    print("VERDICT:", "PACED" if ok else "NOT PACED")
    sys.exit(0 if ok else 1)

asyncio.run(main())
