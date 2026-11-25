import assert from "node:assert/strict";
import test from "node:test";
import type { EntityRecord } from "./api.ts";
import { extractWeatherData } from "./weather.ts";

function makeSensor(
  id: string,
  value: number | string,
  unit?: string,
  extra?: Record<string, unknown>,
  ts = 1000,
): EntityRecord {
  return {
    entity_id: id,
    area_id: "outdoor",
    name: id,
    source: { type: "skill", skill_id: "open-meteo-weather" },
    availability: "online",
    state: { domain: "sensor", value, unit },
    extra,
    last_changed_at: ts,
    last_updated_at: ts,
  };
}

test("extractWeatherData returns null when no weather sensors exist", () => {
  assert.equal(extractWeatherData({}), null);
});

test("extractWeatherData aggregates weather sensor entities and metadata", () => {
  const data = extractWeatherData({
    "sensor.weather_temperature": makeSensor(
      "sensor.weather_temperature",
      21.5,
      "°C",
      { location: "Shanghai", icon: "rain", provider: "open-meteo" },
      2000,
    ),
    "sensor.weather_apparent_temp": makeSensor(
      "sensor.weather_apparent_temp",
      20.8,
      "°C",
      undefined,
      1900,
    ),
    "sensor.weather_humidity": makeSensor(
      "sensor.weather_humidity",
      68,
      "%",
      undefined,
      2100,
    ),
    "sensor.weather_wind_speed": makeSensor(
      "sensor.weather_wind_speed",
      12.4,
      "km/h",
      undefined,
      2050,
    ),
    "sensor.weather_condition": makeSensor(
      "sensor.weather_condition",
      "Light rain",
      undefined,
      { location: "Shanghai", icon: "rain", provider: "open-meteo" },
      2200,
    ),
  });

  assert.ok(data);
  assert.equal(data.temperature, 21.5);
  assert.equal(data.apparentTemp, 20.8);
  assert.equal(data.humidity, 68);
  assert.equal(data.windSpeed, 12.4);
  assert.equal(data.condition, "Light rain");
  assert.equal(data.icon, "rain");
  assert.equal(data.location, "Shanghai");
  assert.equal(data.lastUpdatedAt, 2200);
});
