"""Structural projection of the current decrypted V2 fingerprint groups.

Pinned factory SHA256:
df5a150be914e7d267126f35de92934993e7930f18c3fd00bf79a86bd788ec58
Serializer proof SHA256:
2277a92101204ed781e0337c9e4af48947b85103028a63ace6c522ad5a3328c4
Offsets below are in that factory, not labels from an older decoder.
No decryption, payload decoding, I/O, logging or token acceptance occurs here.
"""

_MAX_INPUT = 16384
_GROUPS = (0, 4, 7, 8, 9)

# r[or], 381236: five field-ID bits, three type bits. Types 1/2
# have no payload; 3/6 have one byte; 4 has a u8 byte count;
# 5 has one byte or a big-endian u16 with its high bit set.

# Type 7 appends encoded bytes directly. r[NF] (459985) emits
# u8 count + u32; r[q9] (459605) emits two bytes per element.
# r[_g] (389884) supplies eight nibbles to r[bF] (461426).
# r[gh] (464531) emits six bytes. r[vh]/r[gA] (360166/360614)
# each supply five elements to r[Bb] (459131), appended at 519700.
_FIXED = {
    (0, 8): 2, (0, 9): 5, (0, 10): 5, (0, 28): 1,
    (0, 30): 5, (0, 31): 2,
    (4, 19): 5, (4, 21): 4, (4, 24): 4,
    (7, 19): 4, (7, 22): 4, (7, 23): 6,
    (8, 6): 6, (8, 30): 10, (8, 31): 10,
}

# r[Ba] (362495): u8 bit count, then ceil(count / 8) bytes.
# Field dispatch is in collectors 472000..504462 and helpers 463080..469000.
_BITS = {
    (0, 11), (0, 14), (0, 21),
    (4, 10), (4, 13), (4, 16), (4, 17), (4, 20),
    (4, 23), (4, 27), (7, 7), (7, 14), (7, 17),
    (8, 7), (8, 16),
}
# r[l3] (462214): two pairs, each compressed u16 or two u16s.
_DIMENSIONS = {(0, 4), (7, 18)}
# r[Kb] (464771): u8 count followed by six bytes per element.
# Callers at 487450..487815 and 489320..489650 supply nine and two elements.
_ARRAY_COUNTS = {(7, 12): 9, (7, 16): 2}

# r[h7] (180002): u8 length-width, big-endian length, then bytes.
# Width zero is its empty/overflow sentinel. Current callers bound the
# resulting payload to 512 (0/12) and 2048 (7/6) bytes.
_LONG_BYTES = {(0, 12): 512, (7, 6): 2048}


def _refuse():
    raise ValueError("token-structure") from None


class _Cursor:
    def __init__(self, data):
        self.data = data
        self.pos = 0

    def skip(self, count):
        if count < 0 or count > len(self.data) - self.pos:
            _refuse()
        start = self.pos
        self.pos += count
        return start

    def byte(self):
        return self.data[self.skip(1)]

    def uint(self, width):
        start = self.skip(width)
        return int.from_bytes(self.data[start:self.pos], "big")


def _raw_field(cursor, key):
    if key in _FIXED:
        cursor.skip(_FIXED[key])
    elif key in _BITS:
        cursor.skip((cursor.byte() + 7) // 8)
    elif key in _DIMENSIONS:
        for _ in range(2):
            if not cursor.uint(2) & 0x8000:
                cursor.skip(2)
    elif key in _ARRAY_COUNTS:
        count = cursor.byte()
        if count != _ARRAY_COUNTS[key]:
            _refuse()
        cursor.skip(count * 6)
    elif key in _LONG_BYTES:
        width = cursor.byte()
        if width > 6:
            _refuse()
        count = cursor.uint(width)
        if width and count < 1 << (8 * (width - 1)):
            _refuse()
        if count > _LONG_BYTES[key]:
            _refuse()
        cursor.skip(count)
    else:
        _refuse()


def _payload(cursor, group, field, kind):
    if kind in (1, 2):
        return
    if kind in (3, 6):
        cursor.skip(1)
    elif kind == 4:
        cursor.skip(cursor.byte())
    elif kind == 5:
        first = cursor.byte()
        if first & 0x80:
            if ((first & 0x7F) << 8 | cursor.byte()) <= 127:
                _refuse()
    elif kind == 7:
        _raw_field(cursor, (group, field))
    else:
        _refuse()


def parse_fingerprint_section(data: bytes) -> tuple[list[dict[str, int]], int]:
    """Return (structural entries, consumed bytes) for exactly five groups.

    Each entry has only group_id, field_id, type and byte_length (all ints).
    byte_length includes the field header and payload framing. Following
    event/trailer bytes are untouched; the caller owns their validation.
    Input must be exact bytes, at most 16384 bytes. Failures have only the
    fixed ValueError message 'token-structure'. No partial result escapes.
    """
    if type(data) is not bytes or not 10 <= len(data) <= _MAX_INPUT:
        _refuse()
    cursor = _Cursor(data)
    entries = []
    # r[oX]/r[nX], 518393..519214: 11-bit group, 5-bit field count.
    for group in _GROUPS:
        header = cursor.uint(2)
        if header >> 5 != group:
            _refuse()
        previous = -1
        for _ in range(header & 31):
            start = cursor.pos
            field_header = cursor.byte()
            field, kind = field_header >> 3, field_header & 7
            # Object.values over numeric field keys, plus group 8's final
            # 30/31 entries, produces strictly increasing field IDs.
            if field <= previous:
                _refuse()
            previous = field
            _payload(cursor, group, field, kind)
            entries.append({"group_id": group, "field_id": field,
                            "type": kind, "byte_length": cursor.pos - start})
    return entries, cursor.pos
