"""The browser cooldown must match the Rust 403 classification policy."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_host.refusal import is_castle_refusal


class RefusalTests(unittest.TestCase):
    def test_complete_framer_security_envelope_is_account_local(self):
        body = ('{"error":{"status":403,"message":"This request was blocked by our security checks",'
                '"data":{"reason":"request_blocked"},"ref":"ref-1",'
                '"timestamp":"2026-11-06T00:00:00Z"}}')
        self.assertTrue(is_castle_refusal(body))

    def test_openrouter_model_refusal_does_not_cool_device(self):
        body = ('{"type":"error","error":{"type":"not_found_error",'
                '"message":"No endpoints found for model"},"request_id":"gen-fixture"}')
        self.assertFalse(is_castle_refusal(body))

    def test_ambiguous_forbidden_and_incomplete_envelope_follow_rust_policy(self):
        self.assertTrue(is_castle_refusal("Forbidden"))
        self.assertFalse(is_castle_refusal(
            '{"error":{"data":{"reason":"request_blocked"}}}'))


if __name__ == "__main__":
    unittest.main()
