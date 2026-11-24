"""Xiaohe Reference Device Skill: Xiaomi Air Purifier (python-miio + demo_mode)."""

from __future__ import annotations

import asyncio
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol

# Allow running directly from the repo without pre-installing xiaohe-sdk into .venv.
_SDK_SRC = Path(__file__).resolve().parents[3] / "sdk" / "python" / "src"
if _SDK_SRC.is_dir() and str(_SDK_SRC) not in sys.path:
    sys.path.insert(0, str(_SDK_SRC))

from xiaohe_sdk import SkillApp  # noqa: E402


@dataclass
class PurifierReading:
    is_on: bool
    pm25: float
    temperature_c: float
    humidity_pct: float


class PurifierBackend(Protocol):
    def read_status(self) -> PurifierReading: ...
    def turn_on(self) -> None: ...
    def turn_off(self) -> None: ...


class DemoPurifierBackend:
    """Deterministic virtual air purifier for hardware-free testing and demos."""

    def __init__(self) -> None:
        self._is_on = False
        self._pm25 = 48.0
        self._temp = 24.5
        self._humidity = 52.0

    def read_status(self) -> PurifierReading:
        if self._is_on:
            self._pm25 = max(12.0, self._pm25 - 6.0)
        return PurifierReading(
            is_on=self._is_on,
            pm25=self._pm25,
            temperature_c=self._temp,
            humidity_pct=self._humidity,
        )

    def turn_on(self) -> None:
        self._is_on = True

    def turn_off(self) -> None:
        self._is_on = False


class MiioPurifierBackend:
    """Real LAN Xiaomi Air Purifier backend wrapping python-miio."""

    def __init__(self, host: str, token: str) -> None:
        try:
            from miio import AirPurifierMiot  # type: ignore[import-not-found]
        except ImportError as exc:
            raise RuntimeError(
                "python-miio is not installed. Run `uv sync --extra miio` in "
                "skills/miio-airpurifier or set demo_mode = true."
            ) from exc
        self._dev = AirPurifierMiot(ip=host, token=token)

    def read_status(self) -> PurifierReading:
        st = self._dev.status()
        return PurifierReading(
            is_on=bool(st.is_on),
            pm25=float(getattr(st, "aqi", 0.0)),
            temperature_c=float(getattr(st, "temperature", 0.0) or 0.0),
            humidity_pct=float(getattr(st, "humidity", 0.0) or 0.0),
        )

    def turn_on(self) -> None:
        self._dev.on()

    def turn_off(self) -> None:
        self._dev.off()


app = SkillApp()
backend: PurifierBackend | None = None


@app.on_init
async def init(config: dict[str, Any], secrets: dict[str, str]) -> None:
    global backend
    demo_mode = bool(config.get("demo_mode", False))
    area_id = config.get("area_id") or "living_room"
    poll_interval = float(config.get("poll_interval_secs", 10))

    if demo_mode:
        app.log("starting miio-airpurifier in demo_mode")
        backend = DemoPurifierBackend()
    else:
        host = str(config.get("host") or "").strip()
        token = str(secrets.get("miio_token") or "").strip()
        if not host or not token:
            raise ValueError("host and secrets.miio_token are required when demo_mode is false")
        app.log(f"connecting to Xiaomi Air Purifier at {host}")
        backend = MiioPurifierBackend(host=host, token=token)

    await app.register_entities(
        device={
            "device_id": "skill:miio-airpurifier",
            "name": "Xiaomi Air Purifier",
            "manufacturer": "Xiaomi",
            "model": "AirPurifierMiot" if not demo_mode else "AirPurifierDemo",
            "area_id": area_id,
        },
        entities=[
            {"entity_id": "switch.purifier_power", "name": "Purifier Power", "domain": "switch"},
            {
                "entity_id": "sensor.purifier_pm25",
                "name": "Purifier PM2.5",
                "domain": "sensor",
                "unit": "µg/m³",
                "device_class": "pm25",
            },
            {
                "entity_id": "sensor.purifier_temp",
                "name": "Purifier Temperature",
                "domain": "sensor",
                "unit": "°C",
                "device_class": "temperature",
            },
            {
                "entity_id": "sensor.purifier_humidity",
                "name": "Purifier Humidity",
                "domain": "sensor",
                "unit": "%",
                "device_class": "humidity",
            },
        ],
    )
    app.spawn_poll_loop(poll_status, interval=poll_interval)


async def poll_status() -> None:
    if backend is None:
        return
    reading = await asyncio.to_thread(backend.read_status)
    await app.emit_state("switch.purifier_power", {"domain": "switch", "on": reading.is_on})
    await app.emit_state(
        "sensor.purifier_pm25",
        {"domain": "sensor", "value": reading.pm25, "unit": "µg/m³", "device_class": "pm25"},
    )
    await app.emit_state(
        "sensor.purifier_temp",
        {
            "domain": "sensor",
            "value": reading.temperature_c,
            "unit": "°C",
            "device_class": "temperature",
        },
    )
    await app.emit_state(
        "sensor.purifier_humidity",
        {
            "domain": "sensor",
            "value": reading.humidity_pct,
            "unit": "%",
            "device_class": "humidity",
        },
    )


@app.on_command("switch.purifier_power")
async def handle_power(action: str, _params: dict[str, Any]) -> dict[str, Any]:
    if backend is None:
        raise RuntimeError("purifier backend is not initialized")
    if action == "turn_on":
        await asyncio.to_thread(backend.turn_on)
    elif action == "turn_off":
        await asyncio.to_thread(backend.turn_off)
    else:
        raise ValueError(f"unsupported action '{action}' for switch.purifier_power")
    await poll_status()
    return {"ok": True, "action": action}


if __name__ == "__main__":
    app.run()
