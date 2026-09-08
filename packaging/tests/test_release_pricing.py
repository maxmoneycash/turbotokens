"""Validate release input identity and reject modified pricing before building."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "release_pricing", Path(__file__).resolve().parents[1] / "release-pricing.py")
pricing = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(pricing)
SOURCE = "a" * 40
COMMIT = "b" * 40
PAYLOAD = b'{"claude-test":{"input_cost_per_token":0.001,"output_cost_per_token":0.002}}\n'


class ReleasePricingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="release inputs ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.directory = self.root / "shared snapshot"

    def prepare(self):
        with patch.object(pricing, "download", return_value=PAYLOAD):
            return pricing.prepare(self.directory, SOURCE, COMMIT)

    def edit_manifest(self, change):
        path = self.directory / pricing.MANIFEST
        manifest = json.loads(path.read_text())
        change(manifest)
        path.write_text(json.dumps(manifest))

    def test_resolves_once_then_downloads_an_immutable_url(self):
        with patch.object(pricing, "download",
                          side_effect=[json.dumps({"sha": COMMIT}).encode(), PAYLOAD]) as fetch:
            manifest = pricing.prepare(self.directory, SOURCE)
        self.assertEqual([call.args[0] for call in fetch.call_args_list],
                         [pricing.LATEST_COMMIT, pricing.pricing_url(COMMIT)])
        self.assertEqual(manifest["pricing"]["sha256"], hashlib.sha256(PAYLOAD).hexdigest())
        self.assertEqual(pricing.verify(self.directory, SOURCE), manifest)

    def test_explicit_commit_does_not_query_a_moving_branch(self):
        with patch.object(pricing, "download", return_value=PAYLOAD) as fetch:
            pricing.prepare(self.directory, SOURCE, COMMIT)
        fetch.assert_called_once_with(pricing.pricing_url(COMMIT))

    def test_exports_a_verified_path_with_spaces_without_replacing_other_environment(self):
        self.prepare()
        environment = self.root / "github.env"
        environment.write_text("EXISTING=value\n")
        pricing.verify(self.directory, SOURCE, environment)
        self.assertEqual(environment.read_text(),
                         "EXISTING=value\nTURBOTOKENS_PRICING_JSON_PATH=" +
                         str((self.directory / pricing.FILENAME).resolve()) + "\n")

    def test_rejects_same_size_modified_bytes_before_exporting_a_path(self):
        self.prepare()
        path = self.directory / pricing.FILENAME
        path.write_bytes(PAYLOAD.replace(b"0.001", b"0.009"))
        environment = self.root / "github.env"
        with self.assertRaisesRegex(ValueError, "checksum differs"):
            pricing.verify(self.directory, SOURCE, environment)
        self.assertFalse(environment.exists())

    def test_rejects_truncated_snapshot(self):
        self.prepare()
        (self.directory / pricing.FILENAME).write_bytes(PAYLOAD[:-1])
        with self.assertRaisesRegex(ValueError, "size differs"):
            pricing.verify(self.directory, SOURCE)

    def test_rejects_another_source_revision(self):
        self.prepare()
        with self.assertRaisesRegex(ValueError, "another source revision"):
            pricing.verify(self.directory, "c" * 40)

    def test_rejects_moving_or_unrelated_urls(self):
        self.prepare()
        self.edit_manifest(lambda manifest: manifest["pricing"].update(
            url="https://example.com/pricing.json"))
        with self.assertRaisesRegex(ValueError, "immutable commit"):
            pricing.verify(self.directory, SOURCE)

    def test_rejects_invalid_commit_before_downloading(self):
        with patch.object(pricing, "download") as fetch:
            with self.assertRaises(ValueError):
                pricing.prepare(self.directory, SOURCE, "../../main")
        fetch.assert_not_called()

    def test_rejects_invalid_payload_without_creating_inputs(self):
        for payload in (b"[]", b"{}", b"not JSON"):
            with self.subTest(payload=payload), patch.object(pricing, "download", return_value=payload):
                with self.assertRaises(ValueError):
                    pricing.prepare(self.directory, SOURCE, COMMIT)
                self.assertFalse(self.directory.exists())

    def test_refuses_to_overwrite_existing_release_inputs(self):
        self.prepare()
        original = (self.directory / pricing.FILENAME).read_bytes()
        with patch.object(pricing, "download", return_value=PAYLOAD), self.assertRaisesRegex(ValueError, "must be empty"):
            pricing.prepare(self.directory, SOURCE, COMMIT)
        self.assertEqual((self.directory / pricing.FILENAME).read_bytes(), original)


if __name__ == "__main__":
    unittest.main()
