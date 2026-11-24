import type { EntityRecord, SseEntityStatePayload } from "./api";

/**
 * Snapshot merge rule (docs/api.md §8):
 * REST snapshot fetches (`GET /api/entities`) only overwrite a cached entity
 * if `incoming.last_updated_at` is strictly newer than the locally cached
 * timestamp. Entities missing from the snapshot are pruned.
 */
export function mergeEntitiesSnapshot(
  current: Record<string, EntityRecord>,
  snapshot: EntityRecord[],
): Record<string, EntityRecord> {
  const next: Record<string, EntityRecord> = {};
  for (const incoming of snapshot) {
    const existing = current[incoming.entity_id];
    if (existing && existing.last_updated_at >= incoming.last_updated_at) {
      // Preserve newer local SSE state while refreshing static metadata if needed.
      next[incoming.entity_id] = {
        ...incoming,
        availability: existing.availability,
        state: existing.state,
        extra: existing.extra,
        last_changed_at: existing.last_changed_at,
        last_updated_at: existing.last_updated_at,
      };
    } else {
      next[incoming.entity_id] = incoming;
    }
  }
  return next;
}

/**
 * Apply a live SSE `entity_state` frame strictly in server arrival order.
 * Returns `null` if the entity is not yet in the cache (caller should trigger a resync).
 */
export function applyEntityStateEvent(
  existing: EntityRecord | undefined,
  event: SseEntityStatePayload,
): EntityRecord | null {
  if (!existing) {
    return null;
  }
  return {
    ...existing,
    area_id: event.area_id ?? existing.area_id,
    availability: event.availability ?? existing.availability,
    state: event.state ?? existing.state,
    extra: event.extra ?? existing.extra,
    last_changed_at: event.last_changed_at,
    last_updated_at: event.last_updated_at,
  };
}
