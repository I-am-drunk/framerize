"""Inert generated-source fixtures. Never import or execute support source."""

from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts' / 'experiments'))
from resident_attempt_paths import check_request_path

EXPECTED = 'resident-request-r14.json'
ASSIGN = "request_path = R / 'private/resident-request-r14.json'"
CALL = 'report, code = resident_control_client.run(str(CONTROL_SOCKET), str(request_path), 120)'


def support(*lines):
    return ('def normal_then_control(capture, protected):\n'
            + ''.join('    ' + line + '\n' for line in lines))


class ResidentAttemptPathsTests(unittest.TestCase):
    def rejected(self, source, expected=EXPECTED):
        with self.assertRaisesRegex(ValueError, '^resident-attempt-path$') as error:
            check_request_path(source, expected)
        self.assertEqual(error.exception.args, ('resident-attempt-path',))
        self.assertIsNone(error.exception.__context__)

    def test_correct_literal_accepts_spacing_quotes_and_same_line_write(self):
        variants = (ASSIGN, 'request_path = (R / "private/resident-request-r14.json")',
                    ASSIGN + '; private_state.write_new(request_path, request)')
        for assignment in variants:
            self.assertIsNone(check_request_path(support(assignment, CALL), EXPECTED))

    def test_expected_basename_is_explicit_not_fixed_to_r14(self):
        source = support(ASSIGN.replace('r14.json', 'r15.json'), CALL)
        self.assertIsNone(check_request_path(source, 'resident-request-r15.json'))
        self.rejected(source)

    def test_stale_r13_filename_survives_preparer_replacements_and_refuses(self):
        stale = support(ASSIGN.replace('r14.json', 'r13.json'), CALL)
        moved = stale.replace('resident-live-r13', 'resident-live-r14').replace(
            'resident-live-r12', 'resident-live-r13').replace(
            'resident-r13', 'resident-r14').replace('resident-r12', 'resident-r13')
        self.assertEqual(moved, stale)
        self.rejected(moved)

    def test_missing_and_duplicate_assignments_refuse(self):
        for lines in ((CALL,), (ASSIGN, ASSIGN, CALL), (ASSIGN, CALL, ASSIGN),
                      (ASSIGN, "request_path = R / 'private/stale.json'", CALL)):
            self.rejected(support(*lines))

    def test_dynamic_paths_refuse(self):
        expressions = ("R / ('private/' + name)", "R / f'private/{name}'",
                       "R / 'private/{}.json'.format(name)",
                       "R.joinpath('private/resident-request-r14.json')",
                       "R / 'private' / 'resident-request-r14.json'", 'path')
        for expression in expressions:
            self.rejected(support('request_path = ' + expression, CALL))

    def test_wrong_root_directory_or_literal_type_refuses(self):
        for expression in ("B / 'private/resident-request-r14.json'", 'R / 14',
                           "R / b'private/resident-request-r14.json'",
                           "R / 'other/resident-request-r14.json'"):
            self.rejected(support('request_path = ' + expression, CALL))

    def test_alternate_assignments_and_rebinding_refuse(self):
        path = "R / 'private/resident-request-r14.json'"
        for assignment in ('request_path: Path = ' + path,
                           'request_path = other = ' + path,
                           'request_path, other = ' + path + ', None',
                           '(request_path := ' + path + ')'):
            self.rejected(support(assignment, CALL))
        for rebind in ('request_path += suffix', 'del request_path',
                       '(request_path := other)', 'for request_path in paths: pass'):
            self.rejected(support(ASSIGN, rebind, CALL))

    def test_conditional_and_nested_assignments_are_not_direct(self):
        for prefix in ('if ready:', 'def nested():', 'class Nested:'):
            self.rejected(support(prefix, '    ' + ASSIGN, CALL))
        self.rejected(support(ASSIGN, 'def nested():', '    ' + ASSIGN, CALL))

    def test_call_is_unique_direct_and_after_assignment(self):
        for lines in ((CALL, ASSIGN), (ASSIGN,), (ASSIGN, CALL, CALL),
                      (ASSIGN, 'if ready:', '    ' + CALL),
                      (ASSIGN, 'def nested():', '    ' + CALL),
                      (ASSIGN, CALL.replace('resident_control_client', 'alias'))):
            self.rejected(support(*lines))

    def test_function_scope_is_unique_module_level_and_undecorated(self):
        valid = support(ASSIGN, CALL)
        for source in ('pass', valid + valid, '@decorate\n' + valid,
                       valid.replace('def normal_', 'async def normal_'),
                       'def outer():\n' + ''.join('    ' + line + '\n'
                                                  for line in valid.splitlines())):
            self.rejected(source)

    def test_comments_strings_and_other_functions_do_not_supply_assignment(self):
        for line in ('# ' + ASSIGN, repr(ASSIGN)):
            self.rejected(support(line, CALL))
        self.rejected('def other():\n    ' + ASSIGN + '\n' + support(CALL))

    def test_source_is_parsed_without_importing_or_executing_it(self):
        source = ('raise RuntimeError("this source must never execute")\n'
                  'import a_module_that_does_not_exist\n' + support(ASSIGN, CALL))
        self.assertIsNone(check_request_path(source, EXPECTED))

    def test_bad_source_has_only_the_fixed_error(self):
        for source in (None, b'', '', True, 'def private_value(', '\x00',
                       'x' * (256 * 1024 + 1)):
            self.rejected(source)

    def test_expected_value_must_be_a_plain_bounded_basename(self):
        class Text(str):
            pass
        for expected in (None, True, b'request.json', Text(EXPECTED), '', '.', '..',
                         '../request.json', 'private/request.json', 'a\\b.json',
                         'request.json\n', 'a b.json', 'x' * 256):
            self.rejected(support(ASSIGN, CALL), expected)
        self.rejected(Text(support(ASSIGN, CALL)))


if __name__ == '__main__':
    unittest.main()
