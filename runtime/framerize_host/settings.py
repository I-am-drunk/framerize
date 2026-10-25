"""Per-process browser and rate policy."""
import os

ROOT = os.environ.get("FRAMERIZE_STATE", os.path.expanduser("~/.framerize/camoufox"))


LOGIN_URL = "https://framer.com/login"


API = "https://api.framer.com"


CASTLE_CHUNK = "https://app.framerstatic.com/chunk-5LKSDTQ5.mjs"


POOL_CAP = int(os.environ.get("FRAMERIZE_PAGE_POOL", "6"))


FORGE_MAX_AGE_S = float(os.environ.get("FRAMERIZE_FORGE_MAX_AGE", "1500"))


DEVICE_POOL = int(os.environ.get("FRAMERIZE_DEVICE_POOL", "4"))


HEARTBEAT_S = float(os.environ.get("FRAMERIZE_HEARTBEAT_S", "60"))


RESERVE_PER_DAY = int(os.environ.get("FRAMERIZE_RESERVE_PER_DAY", "8"))


DEV_COOLDOWN_S = 600.0


DEV_WINDOW_S = 600.0


DEV_WINDOW_CAP = 180


MINT_GAP_S = float(os.environ.get("FRAMERIZE_MINT_GAP_S", "0.25"))
