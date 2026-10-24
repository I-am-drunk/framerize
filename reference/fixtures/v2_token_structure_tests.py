"""Synthetic-only fixtures. No SDK, token, browser or network input."""

from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts" / "experiments"))
from v2_token_structure import parse_fingerprint_section as parse


def field(identifier, kind, payload=b""):
    return bytes([(identifier << 3) | kind]) + payload


def group(identifier, *fields):
    return ((identifier << 5) | len(fields)).to_bytes(2, "big") + b"".join(fields)


def section(identifier=None, *fields):
    return b"".join(group(g, *(fields if g == identifier else ()))
                    for g in (0, 4, 7, 8, 9))


def entry(g, f, kind, length):
    return {"group_id": g, "field_id": f, "type": kind, "byte_length": length}


class TokenStructureTests(unittest.TestCase):
    def refused(self, data):
        with self.assertRaises(ValueError) as caught:
            parse(data)
        self.assertEqual(caught.exception.args, ("token-structure",))

    def test_literal_five_group_vector_and_all_generic_types(self):
        # Hand-framed, independent of the fixture field/group helpers.
        data = bytes.fromhex("0007 01 0a 13ff 1c03616263 257f 2d8080 362a "
                             "0081 fc00 00e0 0100 0120")
        rows, consumed = parse(data)
        self.assertEqual(rows, [
            entry(0, 0, 1, 1), entry(0, 1, 2, 1), entry(0, 2, 3, 2),
            entry(0, 3, 4, 5), entry(0, 4, 5, 2), entry(0, 5, 5, 3),
            entry(0, 6, 6, 2), entry(4, 31, 4, 2),
        ])
        self.assertEqual(consumed, 28)

    def test_empty_groups_leave_trailer_to_caller(self):
        self.assertEqual(parse(section() + b"unparsed\xff"), ([], 10))

    def test_all_fixed_type_seven_widths(self):
        cases = ((0, 8, 2), (0, 9, 5), (0, 10, 5), (0, 28, 1),
                 (0, 30, 5), (0, 31, 2), (4, 19, 5), (4, 21, 4),
                 (4, 24, 4), (7, 19, 4), (7, 22, 4), (7, 23, 6),
                 (8, 6, 6), (8, 30, 10), (8, 31, 10))
        for g, f, length in cases:
            with self.subTest(group=g, field=f):
                data = section(g, field(f, 7, bytes(length)))
                self.assertEqual(parse(data), ([entry(g, f, 7, length + 1)],
                                               len(data)))
                self.refused(data[:2 + length])

    def test_bitset_fields_cross_byte_and_chunk_boundaries(self):
        keys = ((0, 11), (0, 14), (0, 21), (4, 10), (4, 13),
                (4, 16), (4, 17), (4, 20), (4, 23), (4, 27),
                (7, 7), (7, 14), (7, 17), (8, 7), (8, 16))
        for g, f in keys:
            for bits in (0, 1, 7, 8, 9, 23, 24, 25, 255):
                with self.subTest(group=g, field=f, bits=bits):
                    payload = bytes([bits]) + bytes((bits + 7) // 8)
                    data = section(g, field(f, 7, payload))
                    self.assertEqual(parse(data),
                                     ([entry(g, f, 7, len(payload) + 1)], len(data)))

    def test_dimension_pairs_use_each_own_compression_flag(self):
        compact = b"\x80\x01"
        separate = b"\x00\x02\x00\x03"
        for g, f in ((0, 4), (7, 18)):
            for first in (compact, separate):
                for second in (compact, separate):
                    with self.subTest(group=g, first=first, second=second):
                        payload = first + second
                        data = section(g, field(f, 7, payload))
                        self.assertEqual(parse(data),
                                         ([entry(g, f, 7, len(payload) + 1)], len(data)))

    def test_six_byte_arrays_enforce_current_element_counts(self):
        for f, count in ((12, 9), (16, 2)):
            with self.subTest(field=f):
                data = section(7, field(f, 7, bytes([count]) + bytes(count * 6)))
                self.assertEqual(parse(data),
                                 ([entry(7, f, 7, 2 + 6 * count)], len(data)))
                for wrong in (0, count - 1, count + 1, 255):
                    self.refused(section(7, field(f, 7,
                                 bytes([wrong]) + bytes(wrong * 6))))

    def test_long_byte_fields_accept_empty_and_bounded_lengths(self):
        for g, f, cap in ((0, 12, 512), (7, 6, 2048)):
            for length in (0, 1, 255, 256, cap):
                with self.subTest(group=g, length=length):
                    width = (length.bit_length() + 7) // 8
                    payload = bytes([width]) + length.to_bytes(width, "big")
                    payload += bytes(length)
                    data = section(g, field(f, 7, payload))
                    self.assertEqual(parse(data),
                                     ([entry(g, f, 7, len(payload) + 1)], len(data)))

    def test_long_byte_fields_refuse_bad_width_and_length(self):
        for g, f, cap in ((0, 12, 512), (7, 6, 2048)):
            for prefix in (b"\x07", b"\xff", b"\x01\x00", b"\x02\x00\x01",
                           b"\x06\xff\xff\xff\xff\xff\xff"):
                with self.subTest(group=g, prefix=prefix):
                    self.refused(section(g, field(f, 7, prefix)))
            oversized = b"\x02" + (cap + 1).to_bytes(2, "big") + bytes(cap + 1)
            self.refused(section(g, field(f, 7, oversized)))

    def test_short_byte_length_and_packed_integer_boundaries(self):
        for length in (0, 1, 255):
            data = section(0, field(0, 4, bytes([length]) + bytes(length)))
            self.assertEqual(parse(data), ([entry(0, 0, 4, length + 2)], len(data)))
        for payload in (b"\x00", b"\x7f", b"\x80\x80", b"\xff\xff"):
            data = section(0, field(0, 5, payload))
            self.assertEqual(parse(data),
                             ([entry(0, 0, 5, len(payload) + 1)], len(data)))
        for payload in (b"\x80\x00", b"\x80\x7f"):
            self.refused(section(0, field(0, 5, payload)))

    def test_unknown_type_and_unmapped_raw_field_refuse(self):
        for g in (0, 4, 7, 8, 9):
            with self.subTest(group=g):
                self.refused(section(g, field(0, 0)))
                self.refused(section(g, field(0, 7, bytes(12))))

    def test_exact_group_identity_and_order_are_required(self):
        for ids in ((1, 4, 7, 8, 9), (2047, 4, 7, 8, 9),
                    (0, 7, 4, 8, 9), (0, 4, 4, 8, 9), (0, 4, 7, 9, 8)):
            with self.subTest(groups=ids):
                self.refused(b"".join(group(g) for g in ids))

    def test_group_counts_and_unique_ordered_fields(self):
        data = section(0, *(field(i, 1) for i in range(31)))
        rows, consumed = parse(data)
        self.assertEqual(rows, [entry(0, i, 1, 1) for i in range(31)])
        self.assertEqual(consumed, 41)
        for ids in ((1, 1), (2, 1)):
            self.refused(section(0, *(field(i, 1) for i in ids)))
        one = section(0, field(0, 1))
        for claimed in (0, 2, 31):
            with self.subTest(claimed=claimed):
                self.refused(bytes([0, claimed]) + one[2:])

    def test_every_truncated_prefix_refuses_without_partial_result(self):
        packets = [section(), section(0, field(1, 4, b"\x03abc")),
                   section(0, field(1, 5, b"\xff\xff")),
                   section(0, field(4, 7, b"\x80\x01\x00\x02\x00\x03")),
                   section(4, field(16, 7, b"\x09\x00\x00")),
                   section(7, field(12, 7, b"\x09" + bytes(54))),
                   section(7, field(6, 7, b"\x01\x03abc")),
                   section(8, field(30, 7, bytes(10)), field(31, 7, bytes(10)))]
        for index, data in enumerate(packets):
            self.assertEqual(parse(data)[1], len(data))
            for end in range(len(data)):
                with self.subTest(packet=index, end=end):
                    self.refused(data[:end])

    def test_exact_bytes_input_and_total_bound(self):
        class ByteSubclass(bytes):
            def __len__(self):
                raise AssertionError("subclass methods must not run")

        for value in (None, "", 10, True, [], bytearray(10), memoryview(b""),
                      ByteSubclass(section()), b"", bytes(9), bytes(16385)):
            with self.subTest(kind=type(value).__name__):
                self.refused(value)
        self.assertEqual(parse(section() + bytes(16384 - 10)), ([], 10))

    def test_projection_is_only_four_ints_and_never_payload_content(self):
        secret = b"synthetic-private-marker\xff\x00"
        rows, consumed = parse(section(0, field(0, 4, bytes([len(secret)]) + secret)))
        alternate = b"x" * len(secret)
        self.assertEqual((rows, consumed),
                         parse(section(0, field(0, 4, bytes([len(secret)]) + alternate))))
        self.assertEqual(set(rows[0]), {"group_id", "field_id", "type", "byte_length"})
        self.assertTrue(all(type(value) is int for value in rows[0].values()))
        self.assertEqual(sum(row["byte_length"] for row in rows) + 10, consumed)
        self.assertNotIn("synthetic-private-marker", repr(rows))


if __name__ == "__main__":
    unittest.main()
