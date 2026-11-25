import assert from "node:assert/strict";
import test from "node:test";
import type { DashboardWidget, EntityRecord } from "./api.ts";
import {
  addWidget,
  buildSparklineGeometry,
  cycleWidgetSize,
  filterEntitiesForList,
  formatEntityStateSummary,
  moveWidget,
  removeWidget,
  updateWidget,
} from "./widgets.ts";

const sampleEntities: EntityRecord[] = [
  {
    entity_id: "sensor.purifier_pm25",
    name: "PM2.5",
    area_id: "living_room",
    source: { type: "skill", skill_id: "miio-airpurifier" },
    availability: "online",
    state: { domain: "sensor", value: 18.4, unit: "µg/m³" },
    last_changed_at: 100,
    last_updated_at: 100,
  },
  {
    entity_id: "switch.purifier_power",
    name: "Air Purifier",
    area_id: "living_room",
    source: { type: "skill", skill_id: "miio-airpurifier" },
    availability: "online",
    state: { domain: "switch", on: true },
    last_changed_at: 100,
    last_updated_at: 100,
  },
  {
    entity_id: "sensor.weather_temperature",
    name: "Beijing Temperature",
    area_id: "outdoor",
    source: { type: "skill", skill_id: "open-meteo-weather" },
    availability: "online",
    state: { domain: "sensor", value: 3.2, unit: "°C" },
    last_changed_at: 100,
    last_updated_at: 100,
  },
];

test("widget layout helpers add, update, move, resize, and remove immutably", () => {
  const w1: DashboardWidget = {
    id: "w1",
    size: "sm",
    type: "metric",
    entity_id: "sensor.purifier_pm25",
  };
  const w2: DashboardWidget = {
    id: "w2",
    size: "md",
    type: "system_status",
  };

  let list = addWidget([], w1);
  list = addWidget(list, w2);
  assert.deepEqual(
    list.map((w) => w.id),
    ["w1", "w2"],
  );

  list = moveWidget(list, "w2", -1);
  assert.deepEqual(
    list.map((w) => w.id),
    ["w2", "w1"],
  );

  list = cycleWidgetSize(list, "w1");
  assert.equal(list.find((w) => w.id === "w1")?.size, "md");
  list = cycleWidgetSize(list, "w1");
  assert.equal(list.find((w) => w.id === "w1")?.size, "lg");
  list = cycleWidgetSize(list, "w1");
  assert.equal(list.find((w) => w.id === "w1")?.size, "sm");

  list = updateWidget(list, { ...w1, title: "Indoor Air" });
  assert.equal(list.find((w) => w.id === "w1")?.title, "Indoor Air");

  list = removeWidget(list, "w2");
  assert.deepEqual(
    list.map((w) => w.id),
    ["w1"],
  );
});

test("filterEntitiesForList filters by area, domain, and explicit entity_ids", () => {
  const sensors = filterEntitiesForList(sampleEntities, { domain: "sensor" });
  assert.deepEqual(
    sensors.map((e) => e.entity_id),
    ["sensor.weather_temperature", "sensor.purifier_pm25"],
  );

  const livingSensors = filterEntitiesForList(sampleEntities, {
    area_id: "living_room",
    domain: "sensor",
  });
  assert.deepEqual(
    livingSensors.map((e) => e.entity_id),
    ["sensor.purifier_pm25"],
  );

  const picked = filterEntitiesForList(sampleEntities, {
    entity_ids: ["switch.purifier_power"],
  });
  assert.equal(picked.length, 1);
  assert.equal(formatEntityStateSummary(picked[0]), "On");
  assert.equal(formatEntityStateSummary(sampleEntities[0]), "18.4 µg/m³");
});

test("buildSparklineGeometry generates valid SVG paths and stats", () => {
  assert.equal(buildSparklineGeometry([], 200, 60), null);

  const geom = buildSparklineGeometry(
    [
      { ts: 1, avg: 10, min: 10, max: 10 },
      { ts: 2, avg: 20, min: 20, max: 20 },
      { ts: 3, avg: 15, min: 15, max: 15 },
    ],
    200,
    60,
  );
  assert.ok(geom);
  assert.equal(geom.min, 10);
  assert.equal(geom.max, 20);
  assert.equal(geom.latest, 15);
  assert.ok(geom.linePath.startsWith("M 0 "));
  assert.ok(geom.areaPath.endsWith("Z"));
});
