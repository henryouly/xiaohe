"""End-to-end tests for skills/open-meteo-weather (mock HTTP Open-Meteo API + demo_mode)."""

from __future__ import annotations

import json
import subprocess
import sys
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
SKILL_SCRIPT = REPO_ROOT / "skills" / "open-meteo-weather" / "src" / "main.py"


class _MockOpenMeteoHandler(BaseHTTPRequestHandler):
    payload: dict = {
        "current": {
            "temperature_2m": 18.6,
            "relative_humidity_2m": 62,
            "apparent_temperature": 17.9,
            "is_day": 1,
            "weather_code": 61,
            "wind_speed_10m": 14.3,
        }
    }

    def do_GET(self) -> None:  # noqa: N802
        body = json.dumps(self.payload).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, _format: str, *_args: object) -> None:
        return


class WeatherSkillTests(unittest.TestCase):
    @staticmethod
    def _read_until_id(
        proc: subprocess.Popen[str], target_id: int, frames: list[dict]
    ) -> dict:
        assert proc.stdout is not None
        while True:
            line = proc.stdout.readline()
            if not line:
                stderr = proc.stderr.read() if proc.stderr else ""
                raise AssertionError(
                    f"skill exited before responding to id={target_id}; frames={frames}; stderr={stderr}"
                )
            line = line.strip()
            if not line:
                continue
            frame = json.loads(line)
            frames.append(frame)
            if frame.get("id") == target_id:
                return frame

    def test_fetches_from_open_meteo_api_and_handles_refresh_command(self) -> None:
        server = HTTPServer(("127.0.0.1", 0), _MockOpenMeteoHandler)
        port = server.server_address[1]
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            proc = subprocess.Popen(
                [sys.executable, str(SKILL_SCRIPT)],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            assert proc.stdin is not None
            frames: list[dict] = []

            init_req = {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "skill.init",
                "params": {
                    "skill_id": "open-meteo-weather",
                    "config": {
                        "latitude": 37.7749,
                        "longitude": -122.4194,
                        "location_name": "San Francisco",
                        "area_id": "outdoor",
                        "poll_interval_secs": 300,
                        "demo_mode": False,
                        "api_base_url": f"http://127.0.0.1:{port}/v1/forecast",
                    },
                    "secrets": {},
                },
            }
            proc.stdin.write(json.dumps(init_req) + "\n")
            proc.stdin.flush()
            init_resp = self._read_until_id(proc, 1, frames)
            self.assertEqual(init_resp["result"], {"ok": True})

            refresh_req = {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "entity.command",
                "params": {
                    "entity_id": "sensor.weather_temperature",
                    "action": "refresh",
                    "params": {},
                },
            }
            proc.stdin.write(json.dumps(refresh_req) + "\n")
            proc.stdin.flush()
            refresh_resp = self._read_until_id(proc, 2, frames)
            self.assertEqual(refresh_resp["result"]["temperature_c"], 18.6)
            self.assertEqual(refresh_resp["result"]["condition"], "Slight rain")
            self.assertEqual(refresh_resp["result"]["provider"], "open-meteo")

            proc.stdin.write(
                json.dumps({"jsonrpc": "2.0", "method": "skill.shutdown", "params": {}})
                + "\n"
            )
            proc.stdin.flush()
            proc.stdin.close()
            proc.wait(timeout=5)
            err = proc.stderr.read() if proc.stderr else ""
            if proc.stdout:
                proc.stdout.close()
            if proc.stderr:
                proc.stderr.close()
            self.assertEqual(proc.returncode, 0, msg=f"stderr={err}")

            # Verify entity.register includes all 5 weather sensor entities
            reg = next(f for f in frames if f.get("method") == "entity.register")
            reg_ids = {e["entity_id"] for e in reg["params"]["entities"]}
            self.assertEqual(
                reg_ids,
                {
                    "sensor.weather_temperature",
                    "sensor.weather_apparent_temp",
                    "sensor.weather_humidity",
                    "sensor.weather_wind_speed",
                    "sensor.weather_condition",
                },
            )

            # Verify emitted sensor values came from mock Open-Meteo API
            state_events = [
                f["params"] for f in frames if f.get("method") == "entity.state_changed"
            ]
            temp_ev = next(
                e for e in state_events if e["entity_id"] == "sensor.weather_temperature"
            )
            self.assertEqual(temp_ev["state"]["value"], 18.6)
            self.assertEqual(temp_ev["state"]["unit"], "°C")
            self.assertEqual(temp_ev["extra"]["location"], "San Francisco")
            self.assertEqual(temp_ev["extra"]["icon"], "rain")

            cond_ev = next(
                e for e in state_events if e["entity_id"] == "sensor.weather_condition"
            )
            self.assertEqual(cond_ev["state"]["value"], "Slight rain")
        finally:
            server.shutdown()
            server.server_close()

    def test_demo_mode_emits_deterministic_weather_sensors(self) -> None:
        proc = subprocess.Popen(
            [sys.executable, str(SKILL_SCRIPT)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        assert proc.stdin is not None
        frames: list[dict] = []
        proc.stdin.write(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "skill.init",
                    "params": {
                        "skill_id": "open-meteo-weather",
                        "config": {
                            "location_name": "Beijing",
                            "area_id": "outdoor",
                            "demo_mode": True,
                        },
                        "secrets": {},
                    },
                }
            )
            + "\n"
        )
        proc.stdin.flush()
        init_resp = self._read_until_id(proc, 1, frames)
        self.assertEqual(init_resp["result"], {"ok": True})

        proc.stdin.write(
            json.dumps({"jsonrpc": "2.0", "method": "skill.shutdown", "params": {}})
            + "\n"
        )
        proc.stdin.flush()
        proc.stdin.close()
        proc.wait(timeout=5)
        if proc.stdout:
            proc.stdout.close()
        if proc.stderr:
            proc.stderr.close()
        self.assertEqual(proc.returncode, 0)

        state_events = [
            f["params"] for f in frames if f.get("method") == "entity.state_changed"
        ]
        temp_ev = next(
            e for e in state_events if e["entity_id"] == "sensor.weather_temperature"
        )
        self.assertEqual(temp_ev["state"]["value"], 22.4)
        self.assertEqual(temp_ev["extra"]["provider"], "demo")
        self.assertEqual(temp_ev["extra"]["location"], "Beijing")


if __name__ == "__main__":
    unittest.main()
