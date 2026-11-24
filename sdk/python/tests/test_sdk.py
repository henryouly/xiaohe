"""Unit tests for xiaohe_sdk.SkillApp over real OS pipes."""

from __future__ import annotations

import asyncio
import io
import json
import os
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from xiaohe_sdk import RpcError, SkillApp  # noqa: E402


class _PipePair:
    """Real OS pipe pair for testing line-oriented stdio JSON-RPC."""

    def __init__(self) -> None:
        r_fd, w_fd = os.pipe()
        self.reader = os.fdopen(r_fd, "r", encoding="utf-8", buffering=1)
        self.writer = os.fdopen(w_fd, "w", encoding="utf-8", buffering=1)

    def send(self, obj: dict) -> None:
        self.writer.write(json.dumps(obj) + "\n")
        self.writer.flush()

    async def recv(self) -> dict:
        line = await asyncio.to_thread(self.reader.readline)
        assert line, "unexpected EOF on pipe"
        return json.loads(line)

    def close(self) -> None:
        for f in (self.writer, self.reader):
            try:
                f.close()
            except OSError:
                pass


class SkillAppTests(unittest.IsolatedAsyncioTestCase):
    async def test_device_skill_lifecycle_poll_command_and_ping(self) -> None:
        to_skill = _PipePair()
        from_skill = _PipePair()
        stderr_buf = io.StringIO()

        app = SkillApp(stdin=to_skill.reader, stdout=from_skill.writer, stderr=stderr_buf)
        shutdown_called = False
        poll_ticks = 0

        @app.on_init
        async def handle_init(config: dict, secrets: dict) -> None:
            app.log(f"init host={config.get('host')} has_token={'tok' in secrets}")
            await app.register_entities(
                device={"device_id": "dev1", "name": "Dev 1"},
                entities=[{"entity_id": "switch.dev1", "name": "Power", "domain": "switch"}],
            )
            app.spawn_poll_loop(poll_once, interval=0.05)

        async def poll_once() -> None:
            nonlocal poll_ticks
            poll_ticks += 1
            await app.emit_state("switch.dev1", {"domain": "switch", "on": False})

        @app.on_command("switch.dev1")
        async def handle_cmd(action: str, params: dict) -> dict:
            await app.emit_state("switch.dev1", {"domain": "switch", "on": action == "turn_on"})
            return {"ok": True, "echo": action, "params": params}

        @app.on_shutdown
        async def handle_shutdown() -> None:
            nonlocal shutdown_called
            shutdown_called = True

        runner = asyncio.create_task(app.run_async())

        # 1. skill.init
        to_skill.send(
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "skill.init",
                "params": {
                    "protocol_version": "1.0",
                    "skill_id": "test-dev",
                    "config": {"host": "10.0.0.5"},
                    "secrets": {"tok": "abc"},
                },
            }
        )

        seen_init_ok = False
        seen_register = False
        seen_state = False
        for _ in range(10):
            msg = await asyncio.wait_for(from_skill.recv(), timeout=2.0)
            if msg.get("id") == 1:
                self.assertEqual(msg["result"], {"ok": True})
                seen_init_ok = True
            elif msg.get("method") == "entity.register":
                seen_register = True
            elif msg.get("method") == "entity.state_changed":
                seen_state = True
            if seen_init_ok and seen_register and seen_state:
                break

        self.assertTrue(seen_init_ok)
        self.assertTrue(seen_register)
        self.assertTrue(seen_state)

        # 2. skill.ping
        to_skill.send({"jsonrpc": "2.0", "id": 2, "method": "skill.ping"})
        while True:
            msg = await asyncio.wait_for(from_skill.recv(), timeout=2.0)
            if msg.get("id") == 2:
                self.assertEqual(msg["result"], {"ok": True})
                break

        # 3. entity.command
        to_skill.send(
            {
                "jsonrpc": "2.0",
                "id": 3,
                "method": "entity.command",
                "params": {"entity_id": "switch.dev1", "action": "turn_on", "params": {"fast": True}},
            }
        )
        while True:
            msg = await asyncio.wait_for(from_skill.recv(), timeout=2.0)
            if msg.get("id") == 3:
                self.assertEqual(
                    msg["result"], {"ok": True, "echo": "turn_on", "params": {"fast": True}}
                )
                break

        # 4. skill.shutdown
        to_skill.send({"jsonrpc": "2.0", "method": "skill.shutdown"})
        await asyncio.wait_for(runner, timeout=2.0)

        self.assertTrue(shutdown_called)
        self.assertGreaterEqual(poll_ticks, 1)
        self.assertIn("init host=10.0.0.5 has_token=True", stderr_buf.getvalue())

        to_skill.close()
        from_skill.close()

    async def test_action_skill_invoke_with_core_callbacks_and_rpc_error(self) -> None:
        to_skill = _PipePair()
        from_skill = _PipePair()
        stderr_buf = io.StringIO()

        app = SkillApp(stdin=to_skill.reader, stdout=from_skill.writer, stderr=stderr_buf)

        @app.on_invoke("compute")
        async def handle_compute(params: dict, context: dict) -> dict:
            ent = await app.get_entity(params["sensor_id"])
            val = ent["state"]["value"]
            call_res = await app.call_entity("switch.fan", "turn_on", {"reason": "hot"})
            # Also verify RpcError propagation from a missing entity callback
            try:
                await app.get_entity("sensor.missing")
                missing_err = None
            except RpcError as exc:
                missing_err = exc.code
            return {"temp": val, "called": call_res["ok"], "missing_err": missing_err}

        runner = asyncio.create_task(app.run_async())

        to_skill.send(
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "skill.init",
                "params": {"protocol_version": "1.0", "skill_id": "act", "config": {}, "secrets": {}},
            }
        )
        init_resp = await asyncio.wait_for(from_skill.recv(), timeout=2.0)
        self.assertEqual(init_resp["result"], {"ok": True})

        to_skill.send(
            {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "skill.invoke",
                "params": {"action": "compute", "params": {"sensor_id": "sensor.temp"}, "context": {}},
            }
        )

        # Serve core.get_entity, core.call_entity, and core.get_entity (error)
        cb1 = await asyncio.wait_for(from_skill.recv(), timeout=2.0)
        self.assertEqual(cb1["method"], "core.get_entity")
        to_skill.send(
            {"jsonrpc": "2.0", "id": cb1["id"], "result": {"state": {"domain": "sensor", "value": 29.5}}}
        )

        cb2 = await asyncio.wait_for(from_skill.recv(), timeout=2.0)
        self.assertEqual(cb2["method"], "core.call_entity")
        to_skill.send({"jsonrpc": "2.0", "id": cb2["id"], "result": {"ok": True}})

        cb3 = await asyncio.wait_for(from_skill.recv(), timeout=2.0)
        self.assertEqual(cb3["method"], "core.get_entity")
        to_skill.send(
            {"jsonrpc": "2.0", "id": cb3["id"], "error": {"code": -32004, "message": "not found"}}
        )

        invoke_resp = await asyncio.wait_for(from_skill.recv(), timeout=2.0)
        self.assertEqual(invoke_resp["id"], 2)
        self.assertEqual(
            invoke_resp["result"], {"temp": 29.5, "called": True, "missing_err": -32004}
        )

        to_skill.send({"jsonrpc": "2.0", "method": "skill.shutdown"})
        await asyncio.wait_for(runner, timeout=2.0)

        to_skill.close()
        from_skill.close()


if __name__ == "__main__":
    unittest.main()
