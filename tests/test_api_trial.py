"""Guard against losing native output items in compatible streaming responses."""
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from api_trial import completed_response


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


if __name__ == "__main__":
    unittest.main()
