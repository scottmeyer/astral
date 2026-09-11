import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "python"))
from astral import Host, Session


class WorkingHostTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.folder = Path(self.temp.name)
        self.workspace = self.folder / "work"
        shutil.copytree(ROOT / "examples/working-task", self.workspace, ignore=shutil.ignore_patterns("__pycache__"))
        profile = json.loads((self.workspace / "profile.json").read_text())
        profile["checks"]["tests"]["argv"][0] = sys.executable
        self.profile = self.folder / "profile.json"
        self.profile.write_text(json.dumps(profile))
        self.host = Host(self.workspace, self.folder / "state", self.profile)

    def tearDown(self):
        self.host.close()
        self.temp.cleanup()

    def test_real_checks_bind_to_files_and_old_output_remains_retrievable(self):
        failed = self.host.call("check", name="tests", action_id="check-1")["result"]
        self.assertFalse(failed["passed"])
        self.assertGreater(failed["summary"]["failed"], 0)
        before = self.host.snapshot()
        code = "def total(items, discount_bps=0):\n    if not 0 <= discount_bps <= 10000: raise ValueError('discount')\n    return (sum(q*p for q,p in items)*(10000-discount_bps)+5000)//10000\n"
        changed = self.host.call("write_file", path="total.py", expected_sha256=before["files"]["total.py"]["sha256"], content=code, action_id="write-1")
        self.assertTrue(changed["ok"])
        passed = self.host.call("check", name="tests", action_id="check-2")["result"]
        self.assertTrue(passed["passed"])
        self.assertNotIn("stdout_preview", passed)
        self.assertIn("audit_token", passed["summary"]["available_fields"])
        self.assertTrue(self.host.snapshot()["checks"]["tests"]["current"])
        with (self.workspace / "total.py").open("a") as f:
            f.write("\n# external edit\n")
        self.assertFalse(self.host.snapshot()["checks"]["tests"]["current"])
        first = self.host.call("check_history", name="tests", before_sequence=0)["result"]
        self.assertEqual(len(first["receipts"]), 2)
        found = self.host.call("search", handle=passed["stdout_handle"], query="audit_token")["result"]
        offset = found["matches"][0]["offset"]
        page = self.host.call("recall", handle=passed["stdout_handle"], offset=offset, limit=512)["result"]
        self.assertIn("run-", page["data"])
        self.host.close()
        self.host = Host(self.workspace, self.folder / "state", self.profile)
        replay = self.host.call("check", name="tests", action_id="check-2")["result"]
        self.assertEqual(replay, passed)

    def test_unknown_command_and_readonly_edit_are_rejected(self):
        self.assertFalse(self.host.call("check", name="shell")["ok"])
        self.assertFalse(self.host.call("write_file", path="check_total.py", expected_sha256=None, content="pass")["ok"])

    def test_state_compare_and_swap_with_action_id_and_provider_binding(self):
        sequence = self.host.snapshot()["sequence"]
        result = self.host.call("constraint", id="one", text="keep", source="user", expected_sequence=sequence, action_id="entry-1")
        self.assertTrue(result["ok"])
        self.assertEqual(result["result"]["sequence"], self.host.snapshot()["sequence"])
        self.assertFalse(self.host.call("constraint", id="two", text="stale", source="user", expected_sequence=sequence, action_id="entry-2")["ok"])
        session = Session(self.host, self.folder / "session.json", "http://127.0.0.1:1", "test", provider_scope="provider-a")
        session.save()
        Session(self.host, self.folder / "session.json", "http://127.0.0.1:2", "test", provider_scope="provider-a")
        with self.assertRaisesRegex(RuntimeError, "configuration changed"):
            Session(self.host, self.folder / "session.json", "http://127.0.0.1:2", "test", provider_scope="provider-b")

    def test_external_edits_append_updates_without_changing_frozen_snapshot(self):
        session = Session(self.host, self.folder / "session.json", "http://127.0.0.1:1", "test")
        session._snapshot()
        frozen = json.dumps(session.data["history"][0], sort_keys=True)
        (self.workspace / "requirements.json").write_text('{"phase":2,"seed":731}')
        # An independent host observer must not consume the session's changes.
        self.host.snapshot()
        session._begin_turn("Continue with the changed requirements.")
        self.assertEqual(json.dumps(session.data["history"][0], sort_keys=True), frozen)
        update = session.data["history"][-1]["content"][0]["text"]
        self.assertTrue(update.startswith("HOST_WORKING_UPDATES"))
        self.assertIn('"invalidates_prior_verification":true', update)
        self.assertIn('"generation":2', update)
        self.assertEqual(session._collect_updates(), [])
        for number in range(34):
            self.host.call("obligation", id=f"external-{number}", text="pending", source="external host")
        events = session._collect_updates()
        self.assertEqual(len(events), 34)
        self.assertEqual(len({event["sequence"] for event in events}), 34)
        self.assertEqual(session._collect_updates(), [])

    def test_snapshot_wire_is_frozen_until_explicit_epoch_change_and_restart(self):
        session = Session(self.host, self.folder / "session.json", "http://127.0.0.1:1", "test")
        session._snapshot()
        original = json.dumps(session.data["history"], sort_keys=True)
        self.host.call("constraint", id="budget", text="23", source="user correction")
        self.assertEqual(json.dumps(session.data["history"], sort_keys=True), original)
        restored = Session(self.host, self.folder / "session.json", "http://127.0.0.1:1", "test")
        self.assertEqual(restored.data["history"], session.data["history"])
        restored.data["epoch"] = 1
        restored._snapshot()
        self.assertEqual(len(restored.data["snapshots"]), 2)
        self.assertNotEqual(restored.data["snapshots"][0]["sha256"], restored.data["snapshots"][1]["sha256"])
        self.assertIn('"23"', restored.data["history"][-1]["content"][0]["text"])


if __name__ == "__main__":
    unittest.main()
