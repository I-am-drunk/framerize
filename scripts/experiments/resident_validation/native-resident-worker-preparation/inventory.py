"""Reviewed v2 inventory. Discovery reads source text; it never imports candidates."""
import re

RESIDENT = ['resident_control' + suffix + '.rs' for suffix in
            ('', '_owner', '_wire', '_attempt', '_bootstrap', '_bootstrap_tests', '_tests')]
MAIN_REQUIRED = ('resident_socket.rs', 'startup.rs', 'offline_sessions.rs')
CASE = re.compile(r'#\[(?:tokio::)?test(?:\([^\n]*\))?\]\s*(?:async\s+)?fn\s+(\w+)')
COORDINATOR_CASES = (
    'acquire_checks_actual_page_without_inventing_editor_url',
    'acquire_rejects_identity_route_page_and_type_drift',
    'acquire_rejects_extra_and_missing_fields',
    'finish_requires_exact_fenced_browser_settlement',
    'baseline_acceptance_requires_the_complete_successful_response',
    'v2_acquire_rejects_tokens_and_missing_or_invalid_provenance',
    'baseline_and_mint_reject_context_drift_even_with_success_flags',
    'baseline_failure_or_inconsistent_success_never_permits_mint',
    'baseline_rejects_non_boolean_flags_and_non_http_status',
    'baseline_and_mint_require_exact_schema_and_current_token_context',
    'cancellation_retains_capacity_until_actual_settlement',
    'dropped_waiter_retains_capacity_until_actual_settlement',
    'deadline_retains_capacity_until_actual_settlement',
    'local_baseline_rejection_preserves_only_confirmed_native_cleanup',
    'captured_fallback_proxy_is_distinct_from_logical_configuration',
)
BOOTSTRAP_CASES = (
    'missing_resident_host_does_not_read_sessions_or_open_native_account',
    'expired_capture_does_not_read_sessions_or_open_native_account',
)

def resident_cases(cli):
    cases = []
    for name, prefix, expected in (
        ('resident_control_tests.rs', 'resident_control::tests::', COORDINATOR_CASES),
        ('resident_control_bootstrap_tests.rs', 'resident_control::bootstrap::tests::',
         BOOTSTRAP_CASES),
    ):
        actual = CASE.findall((cli / name).read_text())
        assert len(actual) == len(set(actual)) and set(actual) == set(expected), (
            'review exact resident inventory: ' + name)
        cases += [prefix + case for case in expected]
    assert len(COORDINATOR_CASES) == 15 and len(BOOTSTRAP_CASES) == 2
    return cases

def require_main_modules(cli):
    text = (cli / 'main.rs').read_text()
    for name in ('resident_control', 'resident_socket'):
        assert re.search(r'#\[cfg\(feature = "native-dispatch"\)\]\s*mod '
                         + name + ';', text), 'main registration changed: ' + name
    assert all((cli / name).is_file() for name in (*RESIDENT, *MAIN_REQUIRED))
    assert 'resident_control_socket' in text, 'missing explicit startup option'
