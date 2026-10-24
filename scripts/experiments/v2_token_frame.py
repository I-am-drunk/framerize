"""Bounded V13 decoding for isolated diagnostics, never mint/dispatch authority.

Returns private inner bytes to the structural parser. No CLI or logging.
Constants and framing are bound in v2-serializer-compatibility-20261024.md.
"""
import base64
import re
import struct

_KEY = (0x45678907, 0xE7F2A9B0, 0x0D0D0D0D, 0xA3B5C5D6)
_MASK = 0xFFFFFFFF
_DELTA = 0x9E3779B9
_MAX_TOKEN = 16384


def _mix(z, y, total, key):
    return ((((z >> 5) ^ (y << 2)) + ((y >> 3) ^ (z << 4)))
            ^ ((total ^ y) + (key ^ z))) & _MASK


def _decrypt(data):
    words = list(struct.unpack('<' + 'I' * (len(data) // 4), data))
    count = len(words)
    rounds = 6 + 52 // count
    total = rounds * _DELTA & _MASK
    for _ in range(rounds):
        selector = total >> 2 & 3
        y = words[0]
        for index in range(count - 1, -1, -1):
            z = words[index - 1] if index else words[-1]
            words[index] = (words[index] - _mix(
                z, y, total, _KEY[(index & 3) ^ selector])) & _MASK
            y = words[index]
        total = total - _DELTA & _MASK
    return struct.pack('<' + 'I' * count, *words)


def _xor_hex(data, key, length, rotation_index):
    digits = key.hex()
    rotation = int(digits[rotation_index], 16) % length
    part = digits[:length]
    part = part[rotation:] + part[:rotation]
    return bytes.fromhex(''.join(format(int(char, 16)
        ^ int(part[index % length], 16), 'x')
        for index, char in enumerate(data.hex())))


def fingerprint_bytes(token):
    """Decode a canonical V13 frame without exporting identity or timing fields.

    This checks framing only. It does not authenticate, accept or modify a token.
    The returned bytes contain private data and belong only in the isolated
    structural parser. Exceptions have a fixed message and no input values.
    """
    try:
        return _fingerprint_bytes(token)
    except (ValueError, TypeError, IndexError, struct.error):
        raise ValueError('token-frame') from None


def _fingerprint_bytes(token):
    if type(token) is not str or not 1 <= len(token) <= _MAX_TOKEN:
        raise ValueError()
    if not re.fullmatch(r'[A-Za-z0-9_-]+', token):
        raise ValueError()
    packet = base64.urlsafe_b64decode(token + '=' * (-len(token) % 4))
    if base64.urlsafe_b64encode(packet).rstrip(b'=').decode('ascii') != token:
        raise ValueError()
    if len(packet) < 68 or (len(packet) - 4) % 4:
        raise ValueError()
    frame = bytes(value ^ packet[0] for value in packet[1:])
    if frame[0] != 13 or frame[1] > 3 or frame[-1] != ((len(frame) - 1) * 2 & 255):
        raise ValueError()
    plain = _decrypt(frame[2:-1])
    padding = frame[1]
    if padding:
        if plain[-padding:] != bytes(padding):
            raise ValueError()
        plain = plain[:-padding]
    if len(plain) < 62 or int.from_bytes(plain[6:8], 'big') >> 11 != 5:
        raise ValueError()
    # Six-byte init time, two-byte group/integration word, 32-byte key,
    # 16-byte device ID, then two reversible nibble streams.
    once = _xor_hex(plain[56:], plain[40:56], 8, 9)
    inner = _xor_hex(once[6:], once[:6], 4, 3)
    if len(inner) < 11 or inner[-1] != 255:
        raise ValueError()
    return inner
