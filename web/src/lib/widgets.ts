import type {
  DashboardWidget,
  EntityRecord,
  NumericPoint,
  WidgetSize,
  WidgetType,
} from "./api.ts";

export interface WidgetCatalogEntry {
  type: WidgetType;
  label: string;
  summary: string;
  defaultSize: WidgetSize;
  category: "Telemetry" | "Control" | "Overview";
}

export const WIDGET_CATALOG: WidgetCatalogEntry[] = [
  {
    type: "metric",
    label: "Metric KPI",
    summary:
      "Large numeric readout for any sensor entity with optional warn/critical thresholds.",
    defaultSize: "sm",
    category: "Telemetry",
  },
  {
    type: "sparkline",
    label: "Sensor Trend",
    summary:
      "Inline SVG sparkline plotting recent numeric history (1h / 6h / 24h / 7d) with live updates.",
    defaultSize: "md",
    category: "Telemetry",
  },
  {
    type: "entity_card",
    label: "Entity Control",
    summary:
      "Pin any single light, switch, climate, lock, cover, media player, or sensor card.",
    defaultSize: "sm",
    category: "Control",
  },
  {
    type: "weather",
    label: "Weather Banner",
    summary:
      "Live temperature, feels-like, humidity, wind, and WMO icon from weather sensors.",
    defaultSize: "lg",
    category: "Telemetry",
  },
  {
    type: "entity_list",
    label: "State List",
    summary:
      "Compact multi-row table of internal entity states filtered by area, domain, or selection.",
    defaultSize: "md",
    category: "Overview",
  },
  {
    type: "area_group",
    label: "Area Devices",
    summary:
      "Full grid of entity cards for a specific Area, or all areas with interactive filter pills.",
    defaultSize: "lg",
    category: "Control",
  },
  {
    type: "system_status",
    label: "System Overview",
    summary:
      "Live internal status: SSE stream, online/offline entities, areas, skills, and automations.",
    defaultSize: "md",
    category: "Overview",
  },
];

const SIZE_ORDER: WidgetSize[] = ["sm", "md", "lg"];

export function nextWidgetSize(current: WidgetSize): WidgetSize {
  const idx = SIZE_ORDER.indexOf(current);
  return SIZE_ORDER[(idx + 1) % SIZE_ORDER.length];
}

export function generateWidgetId(type: WidgetType): string {
  const rand = Math.random().toString(36).slice(2, 7);
  return `${type.replace(/_/g, "-")}-${Date.now().toString(36)}-${rand}`;
}

export function addWidget(
  widgets: DashboardWidget[],
  widget: DashboardWidget,
): DashboardWidget[] {
  return [...widgets, widget];
}

export function updateWidget(
  widgets: DashboardWidget[],
  updated: DashboardWidget,
): DashboardWidget[] {
  return widgets.map((w) => (w.id === updated.id ? updated : w));
}

export function removeWidget(
  widgets: DashboardWidget[],
  id: string,
): DashboardWidget[] {
  return widgets.filter((w) => w.id !== id);
}

export function moveWidget(
  widgets: DashboardWidget[],
  id: string,
  delta: -1 | 1,
): DashboardWidget[] {
  const idx = widgets.findIndex((w) => w.id === id);
  if (idx === -1) return widgets;
  const nextIdx = idx + delta;
  if (nextIdx < 0 || nextIdx >= widgets.length) return widgets;
  const copy = [...widgets];
  const [item] = copy.splice(idx, 1);
  copy.splice(nextIdx, 0, item);
  return copy;
}

export function cycleWidgetSize(
  widgets: DashboardWidget[],
  id: string,
): DashboardWidget[] {
  return widgets.map((w) =>
    w.id === id ? { ...w, size: nextWidgetSize(w.size) } : w,
  );
}

export function filterEntitiesForList(
  entities: EntityRecord[],
  filter: { area_id?: string; domain?: string; entity_ids?: string[] },
): EntityRecord[] {
  const explicitSet =
    filter.entity_ids && filter.entity_ids.length > 0
      ? new Set(filter.entity_ids)
      : null;

  return entities
    .filter((e) => {
      if (explicitSet && !explicitSet.has(e.entity_id)) return false;
      if (filter.area_id && e.area_id !== filter.area_id) return false;
      if (filter.domain && e.state.domain !== filter.domain) return false;
      return true;
    })
    .sort((a, b) => a.name.localeCompare(b.name));
}

export function formatEntityStateSummary(entity: EntityRecord): string {
  const st = entity.state;
  switch (st.domain) {
    case "sensor": {
      const val =
        typeof st.value === "number"
          ? Number.isInteger(st.value)
            ? String(st.value)
            : st.value.toFixed(1)
          : String(st.value);
      return st.unit ? `${val} ${st.unit}` : val;
    }
    case "binary_sensor":
      return st.active ? "Active" : "Clear";
    case "light":
      return st.on
        ? st.brightness !== undefined
          ? `On · ${st.brightness}%`
          : "On"
        : "Off";
    case "switch":
      return st.on ? "On" : "Off";
    case "climate":
      return st.target_temp_c !== undefined
        ? `${st.mode} · ${st.target_temp_c}°C`
        : st.mode;
    case "cover":
      return st.position !== undefined ? `${st.state} · ${st.position}%` : st.state;
    case "lock":
      return st.state;
    case "media_player":
      return st.title ? `${st.state} · ${st.title}` : st.state;
    case "button":
      return "Ready";
    case "custom":
      return st.state;
  }
}

export interface SparklineGeometry {
  linePath: string;
  areaPath: string;
  min: number;
  max: number;
  latest: number;
}

export function buildSparklineGeometry(
  points: NumericPoint[],
  width: number,
  height: number,
  padY = 6,
): SparklineGeometry | null {
  if (points.length === 0) return null;
  const values = points.map((p) => p.avg);
  const min = Math.min(...values);
  const max = Math.max(...values);
  const span = max - min || 1;
  const usableH = Math.max(1, height - padY * 2);

  const coords = points.map((p, i) => {
    const x =
      points.length === 1 ? width / 2 : (i / (points.length - 1)) * width;
    const norm = (p.avg - min) / span;
    const y = padY + (1 - norm) * usableH;
    return { x: Number(x.toFixed(2)), y: Number(y.toFixed(2)) };
  });

  const linePath =
    coords.length === 1
      ? `M 0 ${coords[0].y} L ${width} ${coords[0].y}`
      : coords
          .map((c, idx) => `${idx === 0 ? "M" : "L"} ${c.x} ${c.y}`)
          .join(" ");

  const firstX = coords.length === 1 ? 0 : coords[0].x;
  const lastX = coords.length === 1 ? width : coords[coords.length - 1].x;
  const areaPath = `${linePath} L ${lastX} ${height} L ${firstX} ${height} Z`;

  return {
    linePath,
    areaPath,
    min,
    max,
    latest: values[values.length - 1],
  };
}
