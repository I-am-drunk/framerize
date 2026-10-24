"""Inert comparison cookie-order fixtures. UNRUN by the source-only author."""
import copy
import importlib.util
from itertools import permutations
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location(
    "_cookie_order_diagnostics", Path(__file__).with_name("browser_comparison_diagnostics_tests.py"))
D = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(D)
F, C, SECRET = D.FIXTURE, D.C, D.SECRET


def cookies():
    return [{"name": "session", "value": F.OLD.LOGIN},
            {"name": "__cuid", "value": F.DEVICE},
            {"name": "zeta", "value": SECRET + "=tail"},
            {"name": "pref", "value": SECRET + "-first"},
            {"name": "pref", "value": SECRET + "-second"},
            {"name": "alpha", "value": SECRET + "-alpha"}]


CHANGES = (
    ("added", "membership", lambda rows: rows.append({"name": "new", "value": SECRET})),
    ("removed", "membership", lambda rows: rows.pop(2)),
    ("renamed", "membership", lambda rows: rows[2].update(name="changed-name")),
    ("duplicate_added", "membership", lambda rows: rows.append(dict(rows[3]))),
    ("value_changed", "values", lambda rows: rows[2].update(value=SECRET + "-changed")),
    ("duplicate_value_changed", "values", lambda rows: rows[3].update(value=SECRET + "-changed")),
)


class CookieHeaderOrderingTests(unittest.TestCase):
    def test_distinct_name_interleavings_have_one_header(self):
        rows = cookies()
        expected = ("__cuid=" + F.DEVICE + "; alpha=" + SECRET + "-alpha; session="
                    + F.OLD.LOGIN + "; zeta=" + SECRET + "=tail")
        for index, order in enumerate(permutations([rows[i] for i in (0, 1, 2, 5)])):
            with self.subTest(order=index):
                sample = list(order)
                original = copy.deepcopy(sample)
                self.assertEqual(C.comparison_cookie_header(sample), expected)
                self.assertEqual(sample, original)

    def test_duplicate_name_order_and_multiplicity_are_preserved(self):
        rows = cookies()
        expected = ("__cuid=" + F.DEVICE + "; alpha=" + SECRET + "-alpha; pref="
                    + SECRET + "-first; pref=" + SECRET + "-second; session="
                    + F.OLD.LOGIN + "; zeta=" + SECRET + "=tail")
        self.assertEqual(C.comparison_cookie_header(rows), expected)
        interleaved = [rows[i] for i in (3, 2, 1, 4, 5, 0)]
        self.assertEqual(C.comparison_cookie_header(interleaved), expected)
        reversed_duplicates = [rows[i] for i in (4, 2, 1, 3, 5, 0)]
        reverse = expected.replace("pref=" + SECRET + "-first; pref=" + SECRET + "-second",
                                   "pref=" + SECRET + "-second; pref=" + SECRET + "-first")
        self.assertEqual(C.comparison_cookie_header(reversed_duplicates), reverse)
        repeated = rows + [dict(rows[3])]
        repeat = expected.replace("-second; session=", "-second; pref=" + SECRET + "-first; session=")
        self.assertEqual(C.comparison_cookie_header(repeated), repeat)

    def test_invalid_rows_are_refused_by_the_existing_validator_before_sorting(self):
        invalid = (None, SECRET, {}, [None], [{}],
                   [{"name": [SECRET], "value": "value"}],
                   cookies() + [{"name": "bad name", "value": SECRET}],
                   cookies() + [{"name": "extra", "value": SECRET + ";x=y"}])
        for index, rows in enumerate(invalid):
            with self.subTest(case=index):
                with self.assertRaisesRegex(ValueError, "^invalid cookie snapshot$"):
                    C.comparison_cookie_header(rows)

    def test_size_count_and_login_guards_are_preserved(self):
        cases = (([{"name": "extra", "value": "x"}] * 257, "invalid cookie snapshot"),
                 (cookies() + [{"name": "extra", "value": "x" * C.LIMIT}],
                  "cookie snapshot exceeds bound"),
                 (cookies() + [dict(cookies()[0])], "ambiguous private login"),
                 (cookies()[1:], "missing private login"))
        for index, (rows, message) in enumerate(cases):
            with self.subTest(case=index):
                with self.assertRaisesRegex(ValueError, "^" + message + "$"):
                    C.comparison_cookie_header(rows)


class ComparisonCookieOrderingTests(unittest.IsolatedAsyncioTestCase):
    asyncSetUp = F.BrowserComparisonTests.asyncSetUp
    asyncTearDown = F.BrowserComparisonTests.asyncTearDown
    fixture = D.ComparisonDiagnosticWorkflowTests.fixture
    assert_public_events = D.ComparisonDiagnosticWorkflowTests.assert_public_events
    assert_pre_send_failure = D.ComparisonDiagnosticWorkflowTests.assert_pre_send_failure
    change_after_mint = D.ComparisonDiagnosticWorkflowTests.change_after_mint

    async def prepared(self):
        value = self.fixture()
        value.context.rows = cookies()
        await value.prepare_context()
        return value

    async def refuse(self, value, stage, mints):
        await self.assert_pre_send_failure(value, stage, mints)
        with self.assertRaises(ValueError):
            await value.native()
        self.assertEqual((value.mints, value.fetches), (mints, 0))

    async def test_interleavings_across_both_mints_keep_the_pair_valid(self):
        value = await self.prepared()
        original = list(value.context.rows)
        bound_header = value.context_reply["cookie_header"]
        value.context.rows = [original[i] for i in (5, 3, 0, 4, 1, 2)]
        self.change_after_mint(value, lambda: setattr(value.context, "rows",
            [original[i] for i in (3, 2, 1, 4, 5, 0)]))
        baseline = await value.baseline()
        self.assertTrue(baseline["accepted"] and baseline["nonempty_text"])
        native = await value.native()
        self.assertEqual(native["cookie_header"], bound_header)
        self.assertEqual((value.mints, value.fetches), (2, 1))
        self.assertEqual(value.scope.comparison_phase, "complete")
        self.assertNotEqual(native["token"], value.fetch_args["token"])
        self.assertEqual([call[0] for call in value.calls].count("goto"), 1)
        self.assertEqual([call[0] for call in value.calls].count("add_cookies"), 1)
        self.assert_public_events(value)

    async def test_before_mint_membership_and_values_changes_refuse_before_send(self):
        for label, difference, change in CHANGES:
            with self.subTest(change=label):
                value = await self.prepared()
                change(value.context.rows)
                await self.refuse(value, "baseline_header_before_mint_" + difference, 0)

    async def test_after_mint_membership_and_values_changes_refuse_before_send(self):
        for label, difference, change in CHANGES:
            with self.subTest(change=label):
                value = await self.prepared()
                self.change_after_mint(value, lambda: change(value.context.rows))
                await self.refuse(value, "baseline_header_after_mint_" + difference, 1)

    async def test_duplicate_value_order_changes_refuse_at_both_header_checks(self):
        for phase, mints in (("before", 0), ("after", 1)):
            with self.subTest(phase=phase):
                value = await self.prepared()

                def swap():
                    rows = value.context.rows
                    rows[3], rows[4] = rows[4], rows[3]

                if phase == "before":
                    swap()
                else:
                    self.change_after_mint(value, swap)
                await self.refuse(value, "baseline_header_" + phase + "_mint_order", mints)

    async def test_reordered_owner_header_still_fails_exact_equality(self):
        value = await self.prepared()
        parts = value.context_reply["cookie_header"].split("; ")
        value.context_reply["cookie_header"] = "; ".join(parts[1:] + parts[:1])
        await self.refuse(value, "baseline_header_before_mint_order", 0)

    async def test_formatting_only_mismatch_is_not_misreported_as_pair_order(self):
        value = await self.prepared()
        value.context_reply["cookie_header"] = value.context_reply["cookie_header"].replace("; ", ";")
        await self.refuse(value, "baseline_header_before_mint", 0)

    async def test_invalid_snapshot_after_context_still_refuses_before_comparison(self):
        value = await self.prepared()
        value.context.rows.append({"name": "bad", "value": SECRET + "\n"})
        await self.refuse(value, "snapshot_header", 0)

    async def test_ordinary_authorization_keeps_original_cookie_order(self):
        value = F.OLD.Harness()
        value.context.rows = cookies()
        try:
            result = await value.prepare()
        finally:
            await value.scope.drain()
        expected = ("session=" + F.OLD.LOGIN + "; __cuid=" + F.DEVICE + "; zeta="
                    + SECRET + "=tail; pref=" + SECRET + "-first; pref="
                    + SECRET + "-second; alpha=" + SECRET + "-alpha")
        self.assertEqual(result["cookie_header"], expected)
        self.assertNotEqual(result["cookie_header"], C.comparison_cookie_header(cookies()))


if __name__ == "__main__":
    unittest.main()
