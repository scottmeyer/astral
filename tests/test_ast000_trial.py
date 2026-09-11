import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("ast000_trial", Path(__file__).parents[1] / "scripts/ast000_trial.py")
trial = importlib.util.module_from_spec(spec)
spec.loader.exec_module(trial)


class RestrictionEvidenceTests(unittest.TestCase):
    def test_only_matching_turn_call_and_denied_result_count(self):
        rows = [
            {"type": "event_msg", "payload": {"type": "task_started", "turn_id": "current"}},
            {"type": "response_item", "payload": {"type": "custom_tool_call", "call_id": "call", "input": 'exec("fixture-command")'}},
            {"type": "response_item", "payload": {"type": "custom_tool_call_output", "call_id": "call", "output": [{"text": '{"exit_code":1,"output":"operation not permitted"}'}]}},
        ]
        self.assertEqual(len(trial.denied_tool_results(rows, "current", "fixture-command")), 1)
        self.assertEqual(trial.denied_tool_results(rows, "previous", "fixture-command"), [])
        self.assertEqual(trial.denied_tool_results(rows, "current", "another-command"), [])
        rows[-1]["payload"]["call_id"] = "unrelated"
        self.assertEqual(trial.denied_tool_results(rows, "current", "fixture-command"), [])


class NativeCaptureTests(unittest.TestCase):
    def fixture(self):
        return [
            {"type": "session_meta", "payload": {"id": "synthetic-source"}},
            {"type": "response_item", "payload": {"type": "message", "content": "old"}},
            {"type": "compacted", "payload": {"replacement_history": [
                {"type": "compaction", "encrypted_content": "synthetic", "unknown": {"keep": True}}
            ], "retained_context": {"preserve": True}, "window_id": "synthetic-window"}},
            {"type": "response_item", "payload": {"type": "custom_tool_call", "call_id": "same-id", "input": "fixture"}},
            {"type": "response_item", "payload": {"type": "custom_tool_call_output", "call_id": "same-id", "output": "done"}},
            {"type": "response_item", "payload": {"type": "message", "role": "assistant", "content": "complete tail"}},
        ]

    def test_native_window_complete_tail_and_metadata_are_preserved(self):
        rows = self.fixture()
        result = trial.capture_native_history(rows)
        self.assertEqual(result["source_records"], rows)
        self.assertEqual(result["items"], rows[2]["payload"]["replacement_history"] + [r["payload"] for r in rows[3:]])
        self.assertEqual(result["items_sha256"], trial.stable(result["items"]))
        self.assertEqual(result["compacted_record_ordinal"], 2)

    def test_last_native_replacement_wins_without_editing_source(self):
        rows = self.fixture()
        rows.append({"type": "compacted", "payload": {"replacement_history": [{"type": "compaction", "encrypted_content": "second"}]}})
        result = trial.capture_native_history(rows)
        self.assertEqual(result["items"], rows[-1]["payload"]["replacement_history"])
        self.assertEqual(len(result["source_records"]), 7)

    def test_unresolved_inherited_plaintext_and_pending_shapes_are_rejected(self):
        for rows in [
            [{"type": "session_meta", "payload": {"id": "fork", "history_base": {"thread_id": "unknown"}}}],
            self.fixture() + [{"type": "compacted", "payload": {"message": "plaintext summary"}}],
            self.fixture()[:4],
            self.fixture() + [{"type": "response_item", "payload": {"type": "configuration_update"}}],
        ]:
            with self.assertRaises(ValueError):
                trial.capture_native_history(rows)

    def test_capture_waits_for_matching_client_tool_search_output(self):
        call = {"type": "response_item", "payload": {
            "type": "tool_search_call", "execution": "client", "call_id": "search-1", "arguments": {}
        }}
        output = {"type": "response_item", "payload": {
            "type": "tool_search_output", "execution": "client", "call_id": "search-1",
            "status": "completed", "tools": []
        }}
        with self.assertRaises(ValueError):
            trial.capture_native_history(self.fixture() + [call])
        rows = self.fixture() + [call, output]
        self.assertEqual(trial.capture_native_history(rows)["items"][-2:], [call["payload"], output["payload"]])
        server = {"type": "response_item", "payload": {
            "type": "tool_search_call", "execution": "server", "call_id": None
        }}
        self.assertEqual(trial.capture_native_history(self.fixture() + [server])["items"][-1], server["payload"])
        for bad in [
            {**output["payload"], "call_id": "other"},
            {**output["payload"], "status": "in_progress"},
            {**output["payload"], "tools": None},
            {**output["payload"], "execution": "unknown"},
            {"type": "function_call_output", "call_id": "search-1", "output": "wrong kind"},
        ]:
            with self.assertRaises(ValueError):
                trial.capture_native_history(self.fixture() + [call, {"type": "response_item", "payload": bad}])

    def test_capture_rejects_orphan_and_duplicate_call_ids(self):
        call = {"type": "response_item", "payload": {
            "type": "function_call", "call_id": "duplicate"
        }}
        orphan = {"type": "response_item", "payload": {
            "type": "function_call_output", "call_id": "missing", "output": "orphan"
        }}
        for suffix in [[call, call], [orphan]]:
            with self.assertRaises(ValueError):
                trial.capture_native_history(self.fixture() + suffix)


if __name__ == "__main__":
    unittest.main()
