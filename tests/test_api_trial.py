"""Guard against losing native output items in compatible streaming responses."""
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from api_trial import completed_response
from inline_trial import prune_inline_history


def wire(*events):
    return b"".join(b"data: " + json.dumps(event).encode() + b"\r\n\r\n" for event in events)


class ResponseItems(unittest.TestCase):
    def test_empty_terminal_output_replays_complete_native_items_in_index_order(self):
        reasoning = {"type": "reasoning", "encrypted_content": "opaque", "summary": []}
        call = {"type": "function_call", "call_id": "a", "name": "read_fixture", "arguments": '{"name":"config"}'}
        raw = wire(
            {"type": "response.output_item.done", "output_index": 1, "item": call},
            {"type": "response.output_item.done", "output_index": 0, "item": reasoning},
            {"type": "response.completed", "response": {"status": "completed", "output": []}},
        )
        self.assertEqual(completed_response(raw)["output"], [reasoning, call])

    def test_terminal_output_keeps_full_canonical_fields(self):
        item = {"type": "message", "role": "assistant", "phase": "final_answer", "content": [{"type": "output_text", "text": "ok"}]}
        raw = wire({"type": "response.completed", "response": {"status": "completed", "output": [item]}})
        self.assertEqual(completed_response(raw)["output"], [item])

    def test_missing_completed_response_is_not_accepted(self):
        raw = wire({"type": "response.output_item.done", "output_index": 0, "item": {"type": "message"}})
        with self.assertRaisesRegex(RuntimeError, "missing_completed_response"):
            completed_response(raw)

    def test_missing_output_index_is_not_silently_dropped(self):
        raw = wire(
            {"type": "response.output_item.done", "output_index": 1, "item": {"type": "message"}},
            {"type": "response.completed", "response": {"status": "completed", "output": []}},
        )
        with self.assertRaisesRegex(RuntimeError, "incomplete_output_item_sequence"):
            completed_response(raw)


class InlineHistory(unittest.TestCase):
    def test_latest_inline_compaction_keeps_complete_ordered_suffix(self):
        old = {"type": "compaction", "encrypted_content": "old-opaque"}
        current = {"type": "compaction", "encrypted_content": "new-opaque", "id": "c2"}
        tail = [
            {"type": "reasoning", "encrypted_content": "reasoning+/=", "summary": []},
            {"type": "function_call", "call_id": "a", "name": "read", "arguments": "{}"},
            {"type": "function_call_output", "call_id": "a", "output": "fixture"},
            {"type": "message", "role": "assistant", "phase": "final_answer", "content": []},
        ]
        history = [{"role": "user", "content": "old facts"}, old, current, *tail]
        self.assertEqual(prune_inline_history(history), [current, *tail])
        self.assertEqual(len(history), 7, "the original history must not be modified")

    def test_reasoning_alone_does_not_authorize_pruning(self):
        history = [{"role": "user", "content": "retain me"},
                   {"type": "reasoning", "encrypted_content": "opaque"}]
        self.assertEqual(prune_inline_history(history), history)

    def test_malformed_inline_compaction_cannot_discard_history(self):
        for payload in (None, "", 12, ["opaque"]):
            with self.subTest(payload=payload), self.assertRaisesRegex(RuntimeError, "missing_encrypted_content"):
                prune_inline_history([{"role": "user", "content": "retain me"},
                                      {"type": "compaction", "encrypted_content": payload}])


if __name__ == "__main__":
    unittest.main()
