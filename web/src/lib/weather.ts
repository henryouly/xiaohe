import type { Availability, EntityRecord } from "./api.ts";

export type WeatherIconKind =
  | "sun"
  | "cloud-sun"
  | "cloud"
  | "fog"
  | "rain"
  | "snow"
  | "storm";

export interface WeatherWidgetData {
  temperature: number | null;
  temperatureUnit: string;
  apparentTemp: number | null;
  humidity: number | null;
  windSpeed: number | null;
  windUnit: string;
  condition: string;
  icon: WeatherIconKind;
  location: string;
  provider: string | null;
  availability: Availability;
  lastUpdatedAt: number;
  refreshEntityId: string;
}

const VALID_ICONS: ReadonlySet<string> = new Set([
  "sun",
  "cloud-sun",
  "cloud",
  "fog",
  "rain",
  "snow",
  "storm",
]);

function readNumericSensor(ent: EntityRecord | undefined): {
  value: number | null;
  unit?: string;
} {
  if (!ent || ent.state.domain !== "sensor") {
    return { value: null };
  }
  const v = ent.state.value;
  if (typeof v === "number" && Number.isFinite(v)) {
    return { value: v, unit: ent.state.unit };
  }
  if (typeof v === "string") {
    const parsed = Number(v);
    if (Number.isFinite(parsed)) {
      return { value: parsed, unit: ent.state.unit };
    }
  }
  return { value: null, unit: ent.state.unit };
}

/**
 * Extract structured Weather Widget data from Xiaohe's live `StateStore` sensor entities.
 * Returns `null` when no weather sensor entities are registered or visible to the user.
 */
export function extractWeatherData(
  entities: Record<string, EntityRecord>,
): WeatherWidgetData | null {
  const tempEnt = entities["sensor.weather_temperature"];
  const condEnt = entities["sensor.weather_condition"];
  const appEnt = entities["sensor.weather_apparent_temp"];
  const humEnt = entities["sensor.weather_humidity"];
  const windEnt = entities["sensor.weather_wind_speed"];

  const primary = tempEnt ?? condEnt;
  if (!primary) {
    return null;
  }

  const temp = readNumericSensor(tempEnt);
  const app = readNumericSensor(appEnt);
  const hum = readNumericSensor(humEnt);
  const wind = readNumericSensor(windEnt);

  const extra = (condEnt?.extra ?? tempEnt?.extra ?? {}) as Record<
    string,
    unknown
  >;

  const rawCondition =
    condEnt?.state.domain === "sensor"
      ? String(condEnt.state.value)
      : typeof extra.condition === "string"
        ? extra.condition
        : "Weather";

  const rawIcon = typeof extra.icon === "string" ? extra.icon : "cloud";
  const icon: WeatherIconKind = VALID_ICONS.has(rawIcon)
    ? (rawIcon as WeatherIconKind)
    : "cloud";

  const location =
    typeof extra.location === "string" && extra.location.trim().length > 0
      ? extra.location.trim()
      : primary.area_id ?? "Outdoor";

  const provider =
    typeof extra.provider === "string" ? extra.provider : null;

  const timestamps = [tempEnt, condEnt, appEnt, humEnt, windEnt]
    .filter((e): e is EntityRecord => e !== undefined)
    .map((e) => e.last_updated_at);

  return {
    temperature: temp.value,
    temperatureUnit: temp.unit ?? "°C",
    apparentTemp: app.value,
    humidity: hum.value,
    windSpeed: wind.value,
    windUnit: wind.unit ?? "km/h",
    condition: rawCondition,
    icon,
    location,
    provider,
    availability: primary.availability,
    lastUpdatedAt: timestamps.length > 0 ? Math.max(...timestamps) : 0,
    refreshEntityId: tempEnt ? tempEnt.entity_id : primary.entity_id,
  };
}
