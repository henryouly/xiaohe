"""End-to-end JSON-RPC 2.0 subprocess tests for starter Skill templates."""

from __future__ import annotations

import json
import subprocess
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]


class StarterTemplateTests(unittest.TestCase):
    def test_device_starter_init_command_and_shutdown(self) -> None:
        script = REPO_ROOT / "templates" / "skills" / "device-starter" / "src" / "main.py"
        proc = subprocess.Popen(
            [sys.executable, str(script)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        msgs = [
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "skill.init",
                "params": {
                    "skill_id": "device-starter",
                    "config": {"area_id": "living_room", "poll_interval_secs": 60, "demo_mode": True},
                    "secrets": {},
                },
            },
            {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "entity.command",
                "params": {
                    "entity_id": "switch.starter_power",
                    "action": "turn_on",
                    "params": {},
                },
            },
            {"jsonrpc": "2.0", "method": "skill.shutdown", "params": {}},
        ]
        out, err = proc.communicate("\n".join(json.dumps(m) for m in msgs) + "\n", timeout=5)
        self.assertEqual(proc.returncode, 0, msg=f"stderr={err}")
        frames = [json.loads(line) for line in out.strip().splitlines() if line.strip()]
        methods = [f.get("method") for f in frames if "method" in f]
        self.assertIn("entity.register", methods)
        self.assertIn("entity.state_changed", methods)
        by_id = {f["id"]: f for f in frames if "id" in f}
        self.assertEqual(by_id[1].get("result"), {"ok": True})
        self.assertEqual(by_id[2].get("result"), {"ok": True, "on": True})

    def test_action_starter_invoke_callback_and_shutdown(self) -> None:
        script = REPO_ROOT / "templates" / "skills" / "action-starter" / "src" / "main.py"
        proc = subprocess.Popen(
            [sys.executable, str(script)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        assert proc.stdin is not None and proc.stdout is not None
        proc.stdin.write(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "skill.init",
                    "params": {"skill_id": "action-starter", "config": {}, "secrets": {}},
                }
            )
            + "\n"
        )
        proc.stdin.flush()
        init_resp = json.loads(proc.stdout.readline())
        self.assertEqual(init_resp, {"jsonrpc": "2.0", "id": 1, "result": {"ok": True}})

        proc.stdin.write(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "skill.invoke",
                    "params": {
                        "action": "ping_entity",
                        "params": {"entity_id": "switch.starter_power"},
                        "context": {},
                    },
                }
            )
            + "\n"
        )
        proc.stdin.flush()

        # Expect core.get_entity callback from action-starter
        cb = json.loads(proc.stdout.readline())
        self.assertEqual(cb["method"], "core.get_entity")
        self.assertEqual(cb["params"]["entity_id"], "switch.starter_power")

        proc.stdin.write(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": cb["id"],
                    "result": {
                        "entity_id": "switch.starter_power",
                        "availability": "online",
                        "state": {"domain": "switch", "on": True},
                    },
                }
            )
            + "\n"
        )
        proc.stdin.flush()

        invoke_resp = json.loads(proc.stdout.readline())
        self.assertEqual(invoke_resp["id"], 2)
        self.assertEqual(invoke_resp["result"]["availability"], "online")

        proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "skill.shutdown", "params": {}}) + "\n")
        proc.stdin.flush()
        proc.stdin.close()
        proc.wait(timeout=5)
        self.assertEqual(proc.returncode, 0)


if __name__ == "__main__":
    unittest.main()
