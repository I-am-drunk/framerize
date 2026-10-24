"""Inert checks for canonical project URLs with a synthetic access suffix."""
import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location("_project_page",
    Path(__file__).resolve().parents[2] / "scripts/contextual_authorization.py")
C = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(C)
PROJECT = "AB12cd34EF56gh78IJ90"
BASE = "https://framer.com/projects/"


class ProjectPageTests(unittest.TestCase):
    def test_access_links_match_only_the_project_id(self):
        for stem in (PROJECT, "My-Site--" + PROJECT):
            page = BASE + stem + "-fixtureAccess1?node=fixture"
            self.assertTrue(C.page_matches(page, PROJECT))
            self.assertFalse(C.page_matches(page, "ZZ12cd34EF56gh78IJ90"))

    def test_title_cannot_supply_the_expected_id(self):
        page = BASE + PROJECT + "--ZZ12cd34EF56gh78IJ90-fixture"
        self.assertFalse(C.page_matches(page, PROJECT))

    def test_malformed_or_nested_access_links_are_refused(self):
        for path in (PROJECT + "-", PROJECT + "-bad_suffix", PROJECT + "-bad-suffix",
                     PROJECT + "-fixture/child", PROJECT + "%2Dfixture",
                     "Bad_Title--" + PROJECT + "-fixture"):
            self.assertFalse(C.page_matches(BASE + path, PROJECT))

    def test_access_links_retain_origin_and_size_checks(self):
        for origin in ("http://framer.com", "https://example.test",
                       "https://framer.com:444", "https://user@framer.com"):
            self.assertFalse(C.page_matches(origin + "/projects/" + PROJECT + "-fixture", PROJECT))
        self.assertFalse(C.page_matches(BASE + PROJECT + "-" + "A" * 4096, PROJECT))


if __name__ == "__main__":
    unittest.main()
