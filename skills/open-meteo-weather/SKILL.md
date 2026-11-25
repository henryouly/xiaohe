# Open-Meteo Weather Skill

Fetches live weather data from the free [Open-Meteo API](https://open-meteo.com/) (no API key required) and publishes typed `sensor` entities into Xiaohe Core so they are stored in `HistoryStore` (SQLite / InfluxDB), streamed over SSE, and rendered in the Web UI Weather Widget.

## Registered Entities
- `sensor.weather_temperature` (`°C`, supports custom action `refresh`)
- `sensor.weather_apparent_temp` (`°C`)
- `sensor.weather_humidity` (`%`)
- `sensor.weather_wind_speed` (`km/h`)
- `sensor.weather_condition` (WMO condition label + `extra.icon` / `extra.location`)

## Configuration (`config/devices.toml`)
```toml
[skills.open-meteo-weather]
enabled = true

[skills.open-meteo-weather.config]
latitude = 39.9042
longitude = 116.4074
location_name = "Beijing"
area_id = "outdoor"
poll_interval_secs = 600
demo_mode = false
```
