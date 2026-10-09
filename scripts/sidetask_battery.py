#!/usr/bin/env python3
"""T3 side-task metering-exemption isolation battery.

Fires a controlled matrix of /ai/v3/* envelopes from a live account session
and reads the X-Framer-AI-Usage header (real-time remaining) on EVERY
response — the ledger lags, the header does not. The cell where `remaining`
stops decrementing while the call still serves tokens is the exemption
boundary.

Dimensions (one varied at a time off a metered baseline):
  route    : /ai/v3/chat/ | /ai/v3/messages/
  stream   : false | true
  category : agents | translation | canvas
  feature  : agents | translation | tracking-id | custom-code-name | (none)
  step     : message | translate | generate-session-title | compact-turn |
             suggest | font-picker | extract-design-patterns

Cost discipline: baseline cells use max_tokens=1 on gemini-2.5-flash-lite
(sub-cent). Metered reference cell runs first and last to prove the header
is live for the whole run.
"""
import asyncio
import json
import os
import sys
import time
import uuid

sys.path.insert(0, os.path.expanduser("/home/tjninja/framerize/scripts"))
from camoufox.async_api import AsyncCamoufox

SESS_DIR = os.path.expanduser("/home/tjninja/.framerize/profiles/.sessions")
API = "https://api.framer.com"
CASTLE_CHUNK = "https://app.framerstatic.com/chunk-5LKSDTQ5.mjs"
CASTLE_PK = "pk_xYqGbB61EcFiSh4LUshpT4DGoEQMKQMB"

# reis — framer_user_id ad922786…, healthy, 500 credits
ACCOUNT = {
    "alias": "reis",
    "session_file": "ad922786-114e-4219-b5b3-05345f8954c9.session",
    "team": "7dd4520d-8ffc-45fd-9dd1-917549753374",
    "project": "6OT4S1xsTCm40UDEzkeg",
}

MODEL = "google/gemini-2.5-flash-lite"
PROMPT = "Say ok."
