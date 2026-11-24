import assert from "node:assert/strict";
import test from "node:test";
import type { EntityRecord } from "./api.ts";
import { applyEntityStateEvent, mergeEntitiesSnapshot } from "./merge.ts";

function makeLight(
  id: string,
  on: boolean,
  lastUpdatedAt: number,
  areaId = "living_room",
): EntityRecord {
  return {
    entity_id: id,
    area_id: areaId,
    name: "Living Light",
    source: { type: "esphome", node_id: "node1" },
    availability: "online",
    state: { domain: "light", on },
    last_changed_at: lastUpdatedAt,
    last_updated_at: lastUpdatedAt,
  };
}

test("mergeEntitiesSnapshot keeps newer local SSE state over stale REST snapshot", () => {
  const local = {
    "light.living": makeLight("light.living", true, 2000),
  };
  const staleSnapshot = [
    makeLight("light.living", false, 1500),
    makeLight("light.kitchen", true, 1600, "kitchen"),
  ];

  const merged = mergeEntitiesSnapshot(local, staleSnapshot);
  assert.equal(Object.keys(merged).length, 2);
  assert.equal(merged["light.living"].last_updated_at, 2000);
  assert.equal(
    merged["light.living"].state.domain === "light" &&
      merged["light.living"].state.on,
    true,
  );
  assert.equal(merged["light.kitchen"].last_updated_at, 1600);
});

test("mergeEntitiesSnapshot overwrites local state when REST snapshot is strictly newer and prunes removed entities", () => {
  const local = {
    "light.living": makeLight("light.living", false, 1000),
    "light.removed": makeLight("light.removed", true, 900),
  };
  const newerSnapshot = [makeLight("light.living", true, 2500)];

  const merged = mergeEntitiesSnapshot(local, newerSnapshot);
  assert.deepEqual(Object.keys(merged), ["light.living"]);
  assert.equal(merged["light.living"].last_updated_at, 2500);
  assert.equal(
    merged["light.living"].state.domain === "light" &&
      merged["light.living"].state.on,
    true,
  );
});

test("applyEntityStateEvent updates cached entity and returns null for unknown entity", () => {
  const existing = makeLight("light.living", false, 1000);
  const updated = applyEntityStateEvent(existing, {
    entity_id: "light.living",
    area_id: "living_room",
    availability: "online",
    state: { domain: "light", on: true, brightness: 80 },
    extra: {},
    last_changed_at: 3000,
    last_updated_at: 3000,
  });
  assert.ok(updated);
  assert.equal(updated.last_updated_at, 3000);
  assert.equal(
    updated.state.domain === "light" && updated.state.brightness,
    80,
  );

  const missing = applyEntityStateEvent(undefined, {
    entity_id: "light.new",
    last_changed_at: 3000,
    last_updated_at: 3000,
  });
  assert.equal(missing, null);
});

test("regression: applyEntityStateEvent clears area_id when SSE sends null and preserves when undefined", () => {
  const existing = makeLight("light.living", true, 1000, "living_room");

  const cleared = applyEntityStateEvent(existing, {
    entity_id: "light.living",
    area_id: null,
    availability: "online",
    state: { domain: "light", on: false },
    extra: {},
    last_changed_at: 2000,
    last_updated_at: 2000,
  });
  assert.ok(cleared);
  assert.equal(cleared.area_id, undefined);

  const preserved = applyEntityStateEvent(existing, {
    entity_id: "light.living",
    last_changed_at: 2100,
    last_updated_at: 2100,
  });
  assert.ok(preserved);
  assert.equal(preserved.area_id, "living_room");
});

test("regression: mergeEntitiesSnapshot preserves newer local area_id over stale REST snapshot", () => {
  const local = {
    "light.living": makeLight("light.living", true, 2000, "kitchen"),
  };
  const stale = [makeLight("light.living", false, 1500, "living_room")];

  const merged = mergeEntitiesSnapshot(local, stale);
  assert.equal(merged["light.living"].area_id, "kitchen");
  assert.equal(merged["light.living"].last_updated_at, 2000);
});
