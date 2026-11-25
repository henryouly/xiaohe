"""Xiaohe Device Skill: Open-Meteo Weather API -> typed sensor entities."""

from __future__ import annotations

import asyncio
import json
import sys
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any

for _parent in Path(__file__).resolve().parents:
    _candidate = _parent / "sdk" / "python" / "src"
    if _candidate.is_dir():
        if str(_candidate) not in sys.path:
            sys.path.insert(0, str(_candidate))
        break

from xiaohe_sdk import SkillApp  # noqa: E402

DEFAULT_API_URL = "https://api.open-meteo.com/v1/forecast"

# WMO Weather interpretation codes (Open-Meteo standard) -> (label, icon)
WMO_CODES: dict[int, tuple[str, str]] = {
    0: ("Clear sky", "sun"),
    1: ("Mainly clear", "sun"),
    2: ("Partly cloudy", "cloud-sun"),
    3: ("Overcast", "cloud"),
    45: ("Fog", "fog"),
    48: ("Depositing rime fog", "fog"),
    51: ("Light drizzle", "rain"),
    53: ("Moderate drizzle", "rain"),
    55: ("Dense drizzle", "rain"),
    56: ("Light freezing drizzle", "rain"),
    57: ("Dense freezing drizzle", "rain"),
    61: ("Slight rain", "rain"),
    63: ("Moderate rain", "rain"),
    65: ("Heavy rain", "rain"),
    66: ("Light freezing rain", "rain"),
    67: ("Heavy freezing rain", "rain"),
    71: ("Slight snow", "snow"),
    73: ("Moderate snow", "snow"),
    75: ("Heavy snow", "snow"),
    77: ("Snow grains", "snow"),
    80: ("Slight rain showers", "rain"),
    81: ("Moderate rain showers", "rain"),
    82: ("Violent rain showers", "rain"),
    85: ("Slight snow showers", "snow"),
    86: ("Heavy snow showers", "snow"),
    95: ("Thunderstorm", "storm"),
    96: ("Thunderstorm with hail", "storm"),
    99: ("Severe thunderstorm", "storm"),
}


def describe_wmo_code(code: int) -> tuple[str, str]:
    return WMO_CODES.get(int(code), (f"Weather code {code}", "cloud"))


@dataclass
class WeatherSnapshot:
    temperature_c: float
    apparent_temp_c: float
    humidity_pct: float
    wind_speed_kmh: float
    wmo_code: int
    condition: str
    icon: str
    is_day: bool
    location_name: str
    source: str


def fetch_open_meteo(
    latitude: float,
    longitude: float,
    location_name: str,
    api_base_url: str = DEFAULT_API_URL,
    timeout_secs: float = 10.0,
) -> WeatherSnapshot:
    params = urllib.parse.urlencode(
        {
            "latitude": f"{latitude:.4f}",
            "longitude": f"{longitude:.4f}",
            "current": (
                "temperature_2m,relative_humidity_2m,"
                "apparent_temperature,is_day,weather_code,wind_speed_10m"
            ),
            "timezone": "auto",
        }
    )
    sep = "&" if "?" in api_base_url else "?"
    url = f"{api_base_url}{sep}{params}"
    req = urllib.request.Request(
        url,
        headers={"User-Agent": "xiaohe-open-meteo-weather/0.1.0"},
        method="GET",
    )
    with urllib.request.urlopen(req, timeout=timeout_secs) as resp:
        payload = json.loads(resp.read().decode("utf-8"))

    current = payload.get("current") or {}
    wmo_code = int(current.get("weather_code", 0))
    condition, icon = describe_wmo_code(wmo_code)
    return WeatherSnapshot(
        temperature_c=round(float(current["temperature_2m"]), 1),
        apparent_temp_c=round(
            float(current.get("apparent_temperature", current["temperature_2m"])), 1
        ),
        humidity_pct=round(float(current.get("relative_humidity_2m", 0.0)), 1),
        wind_speed_kmh=round(float(current.get("wind_speed_10m", 0.0)), 1),
        wmo_code=wmo_code,
        condition=condition,
        icon=icon,
        is_day=bool(current.get("is_day", 1)),
        location_name=location_name,
        source="open-meteo",
    )


class DemoWeatherBackend:
    """Deterministic local weather generator for offline testing and demos."""

    def __init__(self, location_name: str) -> None:
        self._location_name = location_name
        self._tick = 0

    def fetch(self) -> WeatherSnapshot:
        samples = [
            (22.4, 21.8, 48.0, 9.5, 2, True),
            (22.7, 22.1, 46.5, 10.2, 1, True),
            (22.1, 21.5, 50.0, 8.8, 2, True),
        ]
        temp, apparent, hum, wind, code, is_day = samples[self._tick % len(samples)]
        self._tick += 1
        condition, icon = describe_wmo_code(code)
        return WeatherSnapshot(
            temperature_c=temp,
            apparent_temp_c=apparent,
            humidity_pct=hum,
            wind_speed_kmh=wind,
            wmo_code=code,
            condition=condition,
            icon=icon,
            is_day=is_day,
            location_name=self._location_name,
            source="demo",
        )


app = SkillApp()
_config: dict[str, Any] = {}
_demo_backend: DemoWeatherBackend | None = None
_poll_task: asyncio.Task[None] | None = None


async def _fetch_snapshot() -> WeatherSnapshot:
    if _demo_backend is not None:
        return _demo_backend.fetch()
    lat = float(_config.get("latitude", 39.9042))
    lon = float(_config.get("longitude", 116.4074))
    loc = str(_config.get("location_name") or "Local")
    api_url = str(_config.get("api_base_url") or DEFAULT_API_URL)
    return await asyncio.to_thread(fetch_open_meteo, lat, lon, loc, api_url)


async def _emit_snapshot(snap: WeatherSnapshot) -> None:
    meta = {
        "location": snap.location_name,
        "wmo_code": snap.wmo_code,
        "condition": snap.condition,
        "icon": snap.icon,
        "is_day": snap.is_day,
        "provider": snap.source,
    }
    await app.emit_state(
        "sensor.weather_temperature",
        {
            "domain": "sensor",
            "value": snap.temperature_c,
            "unit": "°C",
            "device_class": "temperature",
        },
        extra=meta,
    )
    await app.emit_state(
        "sensor.weather_apparent_temp",
        {
            "domain": "sensor",
            "value": snap.apparent_temp_c,
            "unit": "°C",
            "device_class": "apparent_temperature",
        },
    )
    await app.emit_state(
        "sensor.weather_humidity",
        {
            "domain": "sensor",
            "value": snap.humidity_pct,
            "unit": "%",
            "device_class": "humidity",
        },
    )
    await app.emit_state(
        "sensor.weather_wind_speed",
        {
            "domain": "sensor",
            "value": snap.wind_speed_kmh,
            "unit": "km/h",
            "device_class": "wind_speed",
        },
    )
    await app.emit_state(
        "sensor.weather_condition",
        {
            "domain": "sensor",
            "value": snap.condition,
            "device_class": "weather_condition",
        },
        extra=meta,
    )


async def _poll_loop(interval_secs: float) -> None:
    while True:
        await asyncio.sleep(interval_secs)
        try:
            snap = await _fetch_snapshot()
            await _emit_snapshot(snap)
        except Exception as exc:  # noqa: BLE001
            print(f"weather poll error: {exc}", file=sys.stderr, flush=True)


@app.on_init
async def handle_init(config: dict[str, Any], _secrets: dict[str, str]) -> None:
    global _config, _demo_backend, _poll_task
    _config = dict(config)
    area_id = str(config.get("area_id") or "outdoor")
    loc = str(config.get("location_name") or "Beijing")
    interval = max(5.0, float(config.get("poll_interval_secs") or 600))
    _demo_backend = DemoWeatherBackend(loc) if bool(config.get("demo_mode", False)) else None

    try:
        snap = await _fetch_snapshot()
    except Exception as exc:  # noqa: BLE001
        print(
            f"initial weather fetch failed ({exc}); falling back to demo snapshot until next poll",
            file=sys.stderr,
            flush=True,
        )
        snap = DemoWeatherBackend(loc).fetch()

    meta = {
        "location": snap.location_name,
        "wmo_code": snap.wmo_code,
        "condition": snap.condition,
        "icon": snap.icon,
        "is_day": snap.is_day,
        "provider": snap.source,
    }
    await app.register_entities(
        device={
            "id": "open_meteo_weather",
            "name": f"Weather ({loc})",
            "area_id": area_id,
        },
        entities=[
            {
                "entity_id": "sensor.weather_temperature",
                "name": f"{loc} Temperature",
                "area_id": area_id,
                "state": {
                    "domain": "sensor",
                    "value": snap.temperature_c,
                    "unit": "°C",
                    "device_class": "temperature",
                },
                "extra": meta,
                "custom_actions": [
                    {"name": "refresh", "description": "Fetch latest weather immediately"}
                ],
            },
            {
                "entity_id": "sensor.weather_apparent_temp",
                "name": f"{loc} Feels Like",
                "area_id": area_id,
                "state": {
                    "domain": "sensor",
                    "value": snap.apparent_temp_c,
                    "unit": "°C",
                    "device_class": "apparent_temperature",
                },
            },
            {
                "entity_id": "sensor.weather_humidity",
                "name": f"{loc} Humidity",
                "area_id": area_id,
                "state": {
                    "domain": "sensor",
                    "value": snap.humidity_pct,
                    "unit": "%",
                    "device_class": "humidity",
                },
            },
            {
                "entity_id": "sensor.weather_wind_speed",
                "name": f"{loc} Wind Speed",
                "area_id": area_id,
                "state": {
                    "domain": "sensor",
                    "value": snap.wind_speed_kmh,
                    "unit": "km/h",
                    "device_class": "wind_speed",
                },
            },
            {
                "entity_id": "sensor.weather_condition",
                "name": f"{loc} Condition",
                "area_id": area_id,
                "state": {
                    "domain": "sensor",
                    "value": snap.condition,
                    "device_class": "weather_condition",
                },
                "extra": meta,
            },
        ],
    )
    await _emit_snapshot(snap)
    _poll_task = asyncio.create_task(_poll_loop(interval))


@app.on_command()
async def handle_command(
    entity_id: str, action: str, _params: dict[str, Any]
) -> dict[str, Any]:
    if action != "refresh":
        raise ValueError(f"unsupported action '{action}' on '{entity_id}'")
    snap = await _fetch_snapshot()
    await _emit_snapshot(snap)
    return {
        "ok": True,
        "temperature_c": snap.temperature_c,
        "condition": snap.condition,
        "provider": snap.source,
    }


@app.on_shutdown
async def handle_shutdown() -> None:
    if _poll_task is not None:
        _poll_task.cancel()


if __name__ == "__main__":
    app.run()
