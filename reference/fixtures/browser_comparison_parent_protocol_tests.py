"""Inert wire fixtures. UNRUN by the source-only implementation owner.

Entry: python3 reference/fixtures/browser_comparison_parent_protocol_tests.py
Uses memory fakes only, without sockets, processes, SDKs or account input.
"""
import importlib.util
import json
from pathlib import Path
import struct
import unittest

SPEC = importlib.util.spec_from_file_location("comparison_parent_protocol",
    Path(__file__).resolve().parents[2] / "scripts/browser_comparison_parent_protocol.py")
P = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(P)
NONCE = "a1" * 16


class MemorySocket:
    def __init__(self, incoming=b"", chunk=1 << 20):
        self.incoming = bytearray(incoming)
        self.chunk, self.sent, self.reads = chunk, [], []

    def recv(self, size):
        self.reads.append(size)
        part = bytes(self.incoming[:min(size, self.chunk)])
        del self.incoming[:len(part)]
        return part

    def sendall(self, data):
        self.sent.append(data)


def frame(body):
    return struct.pack("!I", len(body)) + body


def message(command, **fields):
    return {"command": command, "nonce": NONCE, **fields}


def baseline(**changes):
    return {"accepted": True, "status": 200, "content_type": "Json",
            "response_complete": True, "complete_json": True, "nonempty_text": True,
            "body_bytes": 128, "transport_starts": 1, "transport_finishes": 1,
            "fetch_settled": True, "refusal": None, **changes}


def native(**changes):
    return {"complete_json": True, "status": 200, "attempts": 1,
            "authorization_calls": 1, "transport_starts": 1, "local_cleanup": True,
            "failed": False, "refusal": None, **changes}


def result(**changes):
    return message("result", report={"baseline": baseline(), "native": native(),
        "native_nonempty_text": True, "browser_cleanup_confirmed": True,
        "account_closed": True, "failure": None, **changes})


class ProtocolTests(unittest.TestCase):
    def rejected(self, function, *args):
        with self.assertRaises(P.ProtocolError) as caught:
            function(*args)
        self.assertEqual(str(caught.exception), "invalid browser comparison protocol")
        self.assertIsNone(caught.exception.__cause__)
        self.assertIsNone(caught.exception.__context__)

    def test_send_is_compact_utf8_with_network_length(self):
        sock = MemorySocket()
        P.send_frame(sock, {"label": "caf\u00e9", "ok": True})
        body = '{"label":"caf\u00e9","ok":true}'.encode("utf-8")
        self.assertEqual(sock.sent, [frame(body)])
        self.assertEqual(sock.reads, [])

    def test_send_limit_is_inclusive_and_checked_before_writing(self):
        sock = MemorySocket()
        P.send_frame(sock, {"x": "a" * (P.MAX_SEND_BYTES - 8)})
        self.assertEqual(len(sock.sent[0]), P.MAX_SEND_BYTES + 4)
        self.assertEqual(sock.sent[0][:4], struct.pack("!I", P.MAX_SEND_BYTES))
        other = MemorySocket()
        self.rejected(P.send_frame, other, {"x": "a" * (P.MAX_SEND_BYTES - 7)})
        self.rejected(P.send_frame, other, {"x": "\u00e9" * (P.MAX_SEND_BYTES // 2)})
        self.assertEqual(other.sent, [])

    def test_send_refuses_non_json_types_and_nonfinite_values(self):
        cycle = {}
        cycle["cycle"] = cycle
        values = [None, [], {1: "key"}, {"x": (1, 2)}, {"x": object()},
                  {"x": float("nan")}, {"x": float("inf")}, {"x": "\ud800"}, cycle]
        for index, value in enumerate(values):
            with self.subTest(index=index):
                sock = MemorySocket()
                self.rejected(P.send_frame, sock, value)
                self.assertEqual(sock.sent, [])

    def test_fragmented_reads_leave_the_next_frame_untouched(self):
        first = frame(json.dumps(message("ready")).encode())
        second = frame(b'{"another":true}')
        for chunk in (1, 2, 7, 8192):
            with self.subTest(chunk=chunk):
                sock = MemorySocket(first + second, chunk)
                self.assertEqual(P.receive_frame(sock), message("ready"))
                self.assertEqual(bytes(sock.incoming), second)
                self.assertEqual(P.receive_frame(sock), {"another": True})
                self.assertFalse(sock.incoming)

    def test_eof_at_every_header_or_body_boundary_is_refused(self):
        data = frame(b'{"safe":true}')
        for cut in range(len(data)):
            with self.subTest(cut=cut):
                self.rejected(P.receive_frame, MemorySocket(data[:cut], chunk=2))

    def test_receive_limits_are_checked_before_reading_body(self):
        for size in (0, P.MAX_RECEIVE_BYTES + 1, (1 << 32) - 1):
            sock = MemorySocket(struct.pack("!I", size) + b"unused")
            self.rejected(P.receive_frame, sock)
            self.assertEqual(sock.reads, [4])
            self.assertEqual(bytes(sock.incoming), b"unused")
        body = b'{"x":"' + b"a" * (P.MAX_RECEIVE_BYTES - 8) + b'"}'
        self.assertEqual(len(body), P.MAX_RECEIVE_BYTES)
        self.assertEqual(len(P.receive_frame(MemorySocket(frame(body)))["x"]),
                         P.MAX_RECEIVE_BYTES - 8)

    def test_receive_refuses_invalid_utf8_json_and_non_object_roots(self):
        bodies = [b"\xff", b"\xef\xbb\xbf{}", b"{", b"{}{}", b"{} trailing", b"[]",
                  b"null", b"true", b"1", b'"string"', b'{"x":"\\ud800"}']
        for body in bodies:
            with self.subTest(body=body):
                self.rejected(P.receive_frame, MemorySocket(frame(body)))

    def test_receive_refuses_nested_duplicate_keys_and_nonfinite_numbers(self):
        bodies = [b'{"x":1,"x":2}', b'{"x":{"a":1,"a":2}}',
                  b'{"a":1,"\\u0061":2}', b'{"x":NaN}', b'{"x":Infinity}',
                  b'{"x":-Infinity}', b'{"x":[1e999]}']
        for body in bodies:
            with self.subTest(body=body):
                self.rejected(P.receive_frame, MemorySocket(frame(body)))

    def test_io_and_decoder_errors_have_no_private_exception_chain(self):
        def fail(*_):
            raise OSError("synthetic-private-token-and-error")
        sock = MemorySocket()
        sock.sendall = sock.recv = fail
        self.rejected(P.send_frame, sock, {"session_cookie": "synthetic-private"})
        self.rejected(P.receive_frame, sock)
        self.rejected(P.receive_frame, MemorySocket(frame(b'{"synthetic-private":')))

    def test_protocol_error_does_not_retain_a_callers_active_exception(self):
        try:
            raise RuntimeError("synthetic-private-caller-error")
        except RuntimeError:
            self.rejected(P.validate_ready, message("wrong"), NONCE)

    def test_ready_requires_exact_command_nonce_and_fields(self):
        self.assertIsNone(P.validate_ready(message("ready"), NONCE))
        upper = NONCE.upper()
        self.assertIsNone(P.validate_ready(message("ready", nonce=upper), upper))
        for bad in ("b2" * 16, "a" * 31, "a" * 33, "g" * 32, "", None, True, 1):
            self.rejected(P.validate_ready, message("ready", nonce=bad), NONCE)
            if bad != "b2" * 16:
                self.rejected(P.validate_ready, message("ready", nonce=bad), bad)
        for value in ([], {"command": "ready"}, message("done"),
                      message("ready", raw="synthetic-private")):
            self.rejected(P.validate_ready, value, NONCE)

    def test_result_requires_its_command_nonce_and_report(self):
        for value in (message("result"), message("ready", report=result()["report"]),
                      result() | {"nonce": "b2" * 16}, result() | {"raw": "private"},
                      message("result", report=[])):
            self.rejected(P.validate_result, value, NONCE)
        self.assertEqual(P.validate_result(result(), NONCE), result()["report"])

    def test_unknown_and_missing_report_fields_are_refused_at_every_level(self):
        paths = [("report",), ("report", "baseline"), ("report", "native"),
                 ("report", "baseline", "refusal"), ("report", "native", "refusal")]
        for path in paths:
            for operation in ("extra", "missing"):
                with self.subTest(path=path, operation=operation):
                    value = result()
                    if path[-1] == "refusal":
                        leg = value["report"][path[1]]
                        leg["refusal"] = {"policy": "Castle", "basis": "FramerMarker"}
                        if path[1] == "baseline":
                            leg["accepted"] = False
                    target = value
                    for key in path:
                        target = target[key]
                    if operation == "extra":
                        target["raw_body"] = "synthetic-private-response"
                    else:
                        del target[next(iter(target))]
                    self.rejected(P.validate_result, value, NONCE)

    def test_boolean_fields_do_not_accept_numbers_strings_or_null(self):
        fields = [(None, "native_nonempty_text browser_cleanup_confirmed account_closed"),
                  ("baseline", "accepted response_complete complete_json nonempty_text fetch_settled"),
                  ("native", "complete_json local_cleanup failed")]
        for scope, names in fields:
            for name in names.split():
                for wrong in (0, 1, "true", None):
                    value = result()
                    target = value["report"] if scope is None else value["report"][scope]
                    target[name] = wrong
                    with self.subTest(scope=scope, name=name, wrong=wrong):
                        self.rejected(P.validate_result, value, NONCE)

    def test_baseline_numeric_bounds_and_nullable_counters(self):
        cases = {"status": [99, 600, True, "200", 200.0],
                 "body_bytes": [-1, 65537, True, None, "1", 1.0],
                 "transport_starts": [-1, 2, True, "1", 1.0],
                 "transport_finishes": [-1, 2, False, "1", 1.0]}
        for key, values in cases.items():
            for wrong in values:
                with self.subTest(key=key, wrong=wrong):
                    self.rejected(P.validate_result,
                        result(baseline=baseline(accepted=False, **{key: wrong})), NONCE)
        for status in (None, 100, 599):
            for count in (None, 0, 1):
                b = baseline(accepted=False, status=status, body_bytes=65536,
                             transport_starts=count, transport_finishes=count)
                self.assertEqual(P.validate_result(result(baseline=b), NONCE)["baseline"], b)

    def test_native_uses_u16_and_u64_bounds_without_discarding_excess_counts(self):
        for key in ("attempts", "authorization_calls", "transport_starts"):
            for wrong in (-1, 1 << 64, True, None, "1", 1.0):
                with self.subTest(key=key, wrong=wrong):
                    self.rejected(P.validate_result, result(native=native(**{key: wrong})), NONCE)
        for wrong in (-1, 65536, True, "200", 200.0):
            self.rejected(P.validate_result, result(native=native(status=wrong)), NONCE)
        for status in (None, 0, 600, 65535):
            n = native(failed=True, status=status, attempts=2,
                       authorization_calls=(1 << 64) - 1, transport_starts=3)
            checked = P.validate_result(result(native=n, failure="Dispatch"), NONCE)
            self.assertEqual(checked["native"], n)

    def test_only_rust_debug_failure_and_content_type_labels_are_allowed(self):
        failures = ("MissingAuthority", "Cooling", "WavePaused", "Admission", "Context",
                    "Session", "Bootstrap", "Source", "Dispatch", "Cleanup",
                    "CooldownPublication", "OwnerLost")
        for failure in (None, *failures):
            checked = P.validate_result(result(failure=failure), NONCE)
            self.assertEqual(checked["failure"], failure)
        for wrong in ("", "dispatch", "Other", "raw exception text", 0, True, {}):
            self.rejected(P.validate_result, result(failure=wrong), NONCE)
        for label in ("Json", "EventStream", "Other", "Missing"):
            value = result(baseline=baseline(accepted=False, content_type=label))
            self.assertEqual(P.validate_result(value, NONCE)["baseline"]["content_type"], label)
        for wrong in ("json", "application/json", "", None, 1, {}):
            self.rejected(P.validate_result,
                result(baseline=baseline(accepted=False, content_type=wrong)), NONCE)

    def test_refusal_labels_are_fixed_and_are_not_rederived_from_status(self):
        pairs = [("Upstream", "ProviderMarker"), ("Castle", "FramerSecurityEnvelope"),
                 ("Castle", "FramerMarker"), ("Upstream", "ElapsedFallback"),
                 ("Castle", "ElapsedFallback")]
        for policy, basis in pairs:
            refusal = {"policy": policy, "basis": basis}
            value = result(native=native(failed=True, status=403, refusal=refusal))
            self.assertEqual(P.validate_result(value, NONCE)["native"]["refusal"], refusal)
        for refusal in ({"policy": "castle", "basis": "FramerMarker"},
                        {"policy": "Castle", "basis": "framer_marker"},
                        {"policy": True, "basis": "FramerMarker"}, [], "Castle"):
            self.rejected(P.validate_result, result(native=native(refusal=refusal)), NONCE)

    def test_baseline_refusal_survives_uncertain_cleanup_without_native(self):
        refusal = {"policy": "Castle", "basis": "FramerSecurityEnvelope"}
        b = baseline(accepted=False, status=403, complete_json=False, nonempty_text=False,
                     body_bytes=231, refusal=refusal)
        value = result(baseline=b, native=None, native_nonempty_text=False,
                       browser_cleanup_confirmed=False, account_closed=False, failure="Cleanup")
        checked = P.validate_result(value, NONCE)
        self.assertEqual(checked, value["report"])
        self.assertIsNone(checked["native"])
        self.assertEqual(checked["baseline"]["refusal"], refusal)

    def test_native_refusal_stays_separate_and_unclassified_403_stays_unknown(self):
        refusal = {"policy": "Upstream", "basis": "ProviderMarker"}
        n = native(complete_json=False, failed=True, status=403, refusal=refusal)
        value = result(native=n, native_nonempty_text=False, failure="Dispatch")
        checked = P.validate_result(value, NONCE)
        self.assertEqual(checked["native"]["refusal"], refusal)
        self.assertIsNone(checked["baseline"]["refusal"])
        value["report"]["native"]["refusal"] = None
        self.assertIsNone(P.validate_result(value, NONCE)["native"]["refusal"])

    def test_accepted_baseline_requires_complete_single_fetch_evidence(self):
        changes = {"status": 403, "content_type": "Other", "response_complete": False,
                   "complete_json": False, "nonempty_text": False, "body_bytes": 0,
                   "transport_starts": None, "transport_finishes": 0, "fetch_settled": False,
                   "refusal": {"policy": "Castle", "basis": "FramerMarker"}}
        for key, wrong in changes.items():
            with self.subTest(key=key):
                self.rejected(P.validate_result, result(baseline=baseline(**{key: wrong})), NONCE)

    def test_optional_legs_must_be_explicit_and_uncertainty_is_not_success(self):
        value = result(baseline=None, native=None, native_nonempty_text=False,
                       browser_cleanup_confirmed=False, account_closed=False, failure="Session")
        self.assertEqual(P.validate_result(value, NONCE), value["report"])
        for name in ("baseline", "native"):
            for wrong in (False, [], "unknown"):
                self.rejected(P.validate_result, result(**{name: wrong}), NONCE)
        n = native(failed=True, local_cleanup=False, attempts=2, transport_starts=2)
        checked = P.validate_result(result(native=n, failure="Cleanup"), NONCE)
        self.assertEqual(checked["native"], n)
        self.assertNotIn("local_pair_success", checked)

    def test_returned_report_detaches_nested_mutable_facts(self):
        refusal = {"policy": "Upstream", "basis": "ProviderMarker"}
        value = result(native=native(failed=True, status=403, refusal=refusal), failure="Dispatch")
        checked = P.validate_result(value, NONCE)
        value["report"]["baseline"]["accepted"] = False
        value["report"]["native"]["refusal"]["policy"] = "Castle"
        value["report"]["failure"] = "Cleanup"
        self.assertTrue(checked["baseline"]["accepted"])
        self.assertEqual(checked["native"]["refusal"]["policy"], "Upstream")
        self.assertEqual(checked["failure"], "Dispatch")

    def test_done_preserves_admission_and_publication_failure_facts(self):
        for failure in ("Admission", "CooldownPublication", "OwnerLost", "Cleanup", None):
            value = message("done", local_pair_success=False, failure=failure)
            checked = P.validate_done(value, NONCE)
            self.assertEqual(checked, value)
            self.assertIsNot(checked, value)
        value = message("done", local_pair_success=True, failure=None)
        self.assertEqual(P.validate_done(value, NONCE), value)

    def test_done_refuses_success_with_failure_and_wrong_envelopes(self):
        for failure in ("Admission", "CooldownPublication", "Dispatch"):
            self.rejected(P.validate_done,
                message("done", local_pair_success=True, failure=failure), NONCE)
        for wrong in (0, 1, "true", None):
            self.rejected(P.validate_done,
                message("done", local_pair_success=wrong, failure=None), NONCE)
        valid = message("done", local_pair_success=False, failure="Admission")
        for value in (valid | {"nonce": "b2" * 16}, valid | {"command": "result"},
                      valid | {"failure": "admission"}, valid | {"failure": {}},
                      valid | {"raw": "private"}, message("done", local_pair_success=False)):
            self.rejected(P.validate_done, value, NONCE)


if __name__ == "__main__":
    unittest.main()
