"""Explicit development entry for the native-minter/browser transport."""
import asyncio
from pathlib import Path
from framerize_runtime.release import verify_runtime

if __name__ == '__main__':
    verify_runtime(Path(__file__).resolve().parent)
    from framerize_host.supplied import SuppliedHost
    asyncio.run(SuppliedHost().loop())
