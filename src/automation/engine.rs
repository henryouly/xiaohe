use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration as ChronoDuration, Timelike, Utc};
use serde_json::{Map, Value, json};
use tokio::sync::{Mutex as AsyncMutex, Notify, broadcast, watch};
use tokio::task::{JoinHandle, JoinSet};
use tracing::{debug, info, warn};

use crate::automation::new_run_id;
use crate::automation::schedule::{
    CronSchedule, compute_sun_time_utc, parse_wall_time, time_in_window,
};
use crate::automation::schema::{
    ActionSpec, AutomationConfig, ConditionSpec, ExecutionMode, TriggerSpec, Weekday,
};
use crate::automation::validator::{
    AutomationError, LoadedAutomation, ValidationContext, load_automations_dir,
};
use crate::config::LocationConfig;
use crate::esphome::EspHomeManager;
use crate::protocol::domain::{DomainState, EntitySource, SensorValue};
use crate::protocol::rpc::{EntityCommandParams, SkillInvokeParams};
use crate::skills::SkillBridge;
use crate::state::{CoreEvent, EventBus, StateStore, now_unix_ms};
use crate::storage::{AutomationOutcome, AutomationRunRecord, HistoryStore};

const DEFAULT_SKILL_INVOKE_TIMEOUT_SECS: u64 = 15;

/// Abstraction for executing `entity.call` and `skill.invoke` automation steps.
#[async_trait]
pub trait ActionExecutor: Send + Sync {
    async fn call_entity(
        &self,
        entity_id: &str,
        action: &str,
        params: Map<String, Value>,
    ) -> Result<Value, String>;

    async fn invoke_skill(
        &self,
        skill_id: &str,
        action: &str,
        params: Map<String, Value>,
        context: Map<String, Value>,
        timeout: Duration,
    ) -> Result<Value, String>;
}

/// Production `ActionExecutor` routing commands to `SkillBridge` and `EspHomeManager`.
pub struct CoreActionExecutor {
    state: StateStore,
    skills: Option<SkillBridge>,
    esphome: Option<Arc<EspHomeManager>>,
}

impl CoreActionExecutor {
    pub fn new(
        state: StateStore,
        skills: Option<SkillBridge>,
        esphome: Option<Arc<EspHomeManager>>,
    ) -> Self {
        Self {
            state,
            skills,
            esphome,
        }
    }
}

#[async_trait]
impl ActionExecutor for CoreActionExecutor {
    async fn call_entity(
        &self,
        entity_id: &str,
        action: &str,
        params: Map<String, Value>,
    ) -> Result<Value, String> {
        let entity = self
            .state
            .get(entity_id)
            .ok_or_else(|| format!("entity '{entity_id}' not found"))?;

        match entity.source {
            EntitySource::Skill { skill_id } => {
                let bridge = self
                    .skills
                    .as_ref()
                    .ok_or_else(|| "skill bridge is disabled".to_owned())?;
                bridge
                    .send_entity_command(
                        &skill_id,
                        EntityCommandParams {
                            entity_id: entity_id.to_owned(),
                            action: action.to_owned(),
                            params,
                        },
                    )
                    .await
                    .map_err(|e| e.to_string())
            }
            EntitySource::EspHome { node_id } => {
                let mgr = self
                    .esphome
                    .as_ref()
                    .ok_or_else(|| "ESPHome manager is unavailable".to_owned())?;
                mgr.dispatch_command(&node_id, entity_id, action, params)
                    .await
                    .map(|()| json!({ "ok": true }))
                    .map_err(|e| e.to_string())
            }
        }
    }

    async fn invoke_skill(
        &self,
        skill_id: &str,
        action: &str,
        params: Map<String, Value>,
        context: Map<String, Value>,
        timeout: Duration,
    ) -> Result<Value, String> {
        let bridge = self
            .skills
            .as_ref()
            .ok_or_else(|| "skill bridge is disabled".to_owned())?;

        let invoke = SkillInvokeParams {
            action: action.to_owned(),
            params,
            context,
        };
        match tokio::time::timeout(timeout, bridge.invoke_skill(skill_id, Map::new(), invoke)).await
        {
            Ok(Ok(val)) => Ok(val),
            Ok(Err(err)) => Err(err.to_string()),
            Err(_) => Err(format!(
                "timeout invoking skill '{skill_id}.{action}' after {}s",
                timeout.as_secs()
            )),
        }
    }
}

/// Extract the primary numeric value from a `DomainState` for `numeric_state` triggers/conditions.
pub fn extract_numeric_value(state: &DomainState) -> Option<f64> {
    match state {
        DomainState::Sensor {
            value: SensorValue::Number(n),
            ..
        } => Some(*n),
        DomainState::Climate {
            current_temp_c: Some(t),
            ..
        } => Some(f64::from(*t)),
        DomainState::Light {
            brightness: Some(b),
            ..
        } => Some(f64::from(*b)),
        DomainState::Cover {
            position: Some(p), ..
        } => Some(f64::from(*p)),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DebounceKey {
    automation_id: String,
    trigger_idx: usize,
}

struct ActiveRun {
    run_id: String,
    cancel_tx: watch::Sender<bool>,
}

struct PendingTrigger {
    trigger_summary: String,
}

#[derive(Default)]
struct AutomationSlot {
    active: Vec<ActiveRun>,
    queue: VecDeque<PendingTrigger>,
    queue_worker_running: bool,
}

struct EngineInner {
    automations_dir: PathBuf,
    location: LocationConfig,
    state: StateStore,
    events: EventBus,
    history: Arc<dyn HistoryStore>,
    executor: Arc<dyn ActionExecutor>,
    rules: StdMutex<Vec<AutomationConfig>>,
    slots: StdMutex<HashMap<String, Arc<AsyncMutex<AutomationSlot>>>>,
    next_debounce_gen: AtomicU64,
    debouncers: StdMutex<HashMap<DebounceKey, (u64, watch::Sender<bool>)>>,
    last_schedule_fired: StdMutex<HashMap<DebounceKey, i64>>,
    in_flight_runs: AtomicUsize,
    in_flight_notify: Notify,
    shutdown_rx: watch::Receiver<bool>,
}

struct InFlightGuard {
    inner: Arc<EngineInner>,
}

impl InFlightGuard {
    fn acquire(inner: &Arc<EngineInner>) -> Self {
        inner.in_flight_runs.fetch_add(1, Ordering::AcqRel);
        Self {
            inner: Arc::clone(inner),
        }
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if self.inner.in_flight_runs.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.inner.in_flight_notify.notify_waiters();
        }
    }
}

/// Background declarative TOML automation engine running inside `xiaohe core`.
#[derive(Clone)]
pub struct AutomationEngine {
    inner: Arc<EngineInner>,
    tasks: Arc<StdMutex<Vec<JoinHandle<()>>>>,
}

impl AutomationEngine {
    /// Load `<automations_dir>/*.toml` and start the event listener & 1-second schedule ticker.
    pub fn start(
        automations_dir: impl Into<PathBuf>,
        location: LocationConfig,
        state: StateStore,
        events: EventBus,
        history: Arc<dyn HistoryStore>,
        executor: Arc<dyn ActionExecutor>,
        shutdown_rx: watch::Receiver<bool>,
    ) -> (Self, Vec<AutomationError>) {
        let dir = automations_dir.into();
        let inner = Arc::new(EngineInner {
            automations_dir: dir,
            location,
            state,
            events,
            history,
            executor,
            rules: StdMutex::new(Vec::new()),
            slots: StdMutex::new(HashMap::new()),
            next_debounce_gen: AtomicU64::new(1),
            debouncers: StdMutex::new(HashMap::new()),
            last_schedule_fired: StdMutex::new(HashMap::new()),
            in_flight_runs: AtomicUsize::new(0),
            in_flight_notify: Notify::new(),
            shutdown_rx,
        });

        let engine = Self {
            inner: Arc::clone(&inner),
            tasks: Arc::new(StdMutex::new(Vec::new())),
        };

        let (_, errors) = engine.reload();

        // Subscribe synchronously before returning so events published immediately after start()
        // are never missed due to task scheduling latency.
        let event_rx = engine.inner.events.subscribe();
        let event_engine = engine.clone();
        let event_task = tokio::spawn(async move {
            run_event_loop(event_engine, event_rx).await;
        });

        let sched_engine = engine.clone();
        let sched_task = tokio::spawn(async move {
            run_schedule_loop(sched_engine).await;
        });

        *engine.tasks.lock().expect("tasks mutex poisoned") = vec![event_task, sched_task];
        (engine, errors)
    }

    /// Reload all `*.toml` rules from disk, cancelling obsolete debounce timers.
    pub fn reload(&self) -> (Vec<LoadedAutomation>, Vec<AutomationError>) {
        let mut ctx = ValidationContext::default();
        for ent in self.inner.state.list(&Default::default()) {
            ctx.known_entities.insert(ent.entity_id);
        }

        let (loaded, errors) = load_automations_dir(&self.inner.automations_dir, Some(&ctx));
        for err in &errors {
            warn!(error = %err, "failed to load automation file");
        }
        for auto in &loaded {
            for w in &auto.warnings {
                debug!(automation_id = %w.automation_id, warning = %w.message, "automation validation warning");
            }
        }

        // Cancel all armed debounce timers so stale rules never fire after reload.
        {
            let mut debouncers = self
                .inner
                .debouncers
                .lock()
                .expect("debouncers mutex poisoned");
            for (_, (_, tx)) in debouncers.drain() {
                let _ = tx.send(true);
            }
        }

        let configs: Vec<AutomationConfig> = loaded.iter().map(|l| l.config.clone()).collect();
        info!(
            count = configs.len(),
            errors = errors.len(),
            dir = %self.inner.automations_dir.display(),
            "loaded declarative automations"
        );
        *self.inner.rules.lock().expect("rules mutex poisoned") = configs;
        (loaded, errors)
    }

    /// Return a snapshot of all currently loaded automations.
    pub fn list_rules(&self) -> Vec<AutomationConfig> {
        self.inner
            .rules
            .lock()
            .expect("rules mutex poisoned")
            .clone()
    }

    /// Manually trigger a loaded automation by `id` (bypasses trigger matching; still evaluates
    /// conditions and concurrency mode).
    pub async fn trigger_manual(&self, automation_id: &str) -> Result<(), String> {
        let rule = self
            .inner
            .rules
            .lock()
            .expect("rules mutex poisoned")
            .iter()
            .find(|r| r.id == automation_id)
            .cloned()
            .ok_or_else(|| format!("automation '{automation_id}' not found"))?;
        dispatch_trigger(Arc::clone(&self.inner), rule, "manual".to_owned()).await;
        Ok(())
    }

    /// Gracefully wait for background automation loops and all in-flight runs to exit and persist
    /// their audit records after `shutdown_rx` fires.
    pub async fn shutdown(&self) {
        {
            let mut debouncers = self
                .inner
                .debouncers
                .lock()
                .expect("debouncers mutex poisoned");
            for (_, (_, tx)) in debouncers.drain() {
                let _ = tx.send(true);
            }
        }

        // Cancel any active runs and clear pending queues.
        let slots: Vec<Arc<AsyncMutex<AutomationSlot>>> = self
            .inner
            .slots
            .lock()
            .expect("slots mutex poisoned")
            .values()
            .cloned()
            .collect();
        for slot in slots {
            let mut guard = slot.lock().await;
            guard.queue.clear();
            for active in &guard.active {
                let _ = active.cancel_tx.send(true);
            }
        }

        let handles: Vec<JoinHandle<()>> = {
            let mut guard = self.tasks.lock().expect("tasks mutex poisoned");
            std::mem::take(&mut *guard)
        };
        for handle in handles {
            if let Err(err) = handle.await {
                warn!(error = %err, "automation engine task join error");
            }
        }

        // Wait until every spawned run has completed and persisted its AutomationRunRecord.
        loop {
            let notified = self.inner.in_flight_notify.notified();
            if self.inner.in_flight_runs.load(Ordering::Acquire) == 0 {
                break;
            }
            notified.await;
        }
    }
}

async fn run_event_loop(engine: AutomationEngine, mut rx: broadcast::Receiver<CoreEvent>) {
    let mut shutdown_rx = engine.inner.shutdown_rx.clone();

    loop {
        tokio::select! {
            biased;
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    break;
                }
            }
            ev_res = rx.recv() => {
                match ev_res {
                    Ok(ev) => handle_core_event(&engine.inner, ev).await,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "automation engine lagged on event bus");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

async fn run_schedule_loop(engine: AutomationEngine) {
    let mut shutdown_rx = engine.inner.shutdown_rx.clone();
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased;
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    break;
                }
            }
            _ = ticker.tick() => {
                evaluate_scheduled_triggers(&engine.inner, Utc::now()).await;
            }
        }
    }
}

async fn evaluate_scheduled_triggers(inner: &Arc<EngineInner>, now: DateTime<Utc>) {
    let rules = inner.rules.lock().expect("rules mutex poisoned").clone();
    let now_sec = now.timestamp();
    let now_wd = Weekday::from_chrono(now.weekday());

    for rule in rules {
        if !rule.enabled {
            continue;
        }
        for (idx, trigger) in rule.triggers.iter().enumerate() {
            let matched = match trigger {
                TriggerSpec::Time { at, cron, weekdays } => {
                    if !weekdays.is_empty() && !weekdays.contains(&now_wd) {
                        None
                    } else if let Some(at_str) = at {
                        parse_wall_time(at_str).ok().and_then(|t| {
                            (t.hour() == now.hour()
                                && t.minute() == now.minute()
                                && t.second() == now.second())
                            .then(|| format!("time:{at_str}"))
                        })
                    } else if let Some(cron_str) = cron {
                        CronSchedule::parse(cron_str)
                            .ok()
                            .and_then(|s| s.matches(&now).then(|| format!("cron:{cron_str}")))
                    } else {
                        None
                    }
                }
                TriggerSpec::Sun {
                    event,
                    offset_minutes,
                } => {
                    if let (Some(lat), Some(lon)) =
                        (inner.location.latitude, inner.location.longitude)
                    {
                        compute_sun_time_utc(now.date_naive(), lat, lon, *event).and_then(|t| {
                            let target = t + ChronoDuration::minutes(i64::from(*offset_minutes));
                            (target.hour() == now.hour()
                                && target.minute() == now.minute()
                                && target.second() == now.second())
                            .then(|| format!("sun:{event:?}({offset_minutes:+}m)"))
                        })
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(summary) = matched {
                let key = DebounceKey {
                    automation_id: rule.id.clone(),
                    trigger_idx: idx,
                };
                let already_fired = {
                    let mut guard = inner
                        .last_schedule_fired
                        .lock()
                        .expect("schedule dedup mutex poisoned");
                    if guard.get(&key) == Some(&now_sec) {
                        true
                    } else {
                        guard.insert(key, now_sec);
                        false
                    }
                };
                if !already_fired {
                    dispatch_trigger(Arc::clone(inner), rule.clone(), summary).await;
                }
            }
        }
    }
}

async fn handle_core_event(inner: &Arc<EngineInner>, event: CoreEvent) {
    let rules = inner.rules.lock().expect("rules mutex poisoned").clone();
    for rule in rules {
        if !rule.enabled {
            continue;
        }
        for (idx, trigger) in rule.triggers.iter().enumerate() {
            match (&event, trigger) {
                (
                    CoreEvent::EntityStateChanged {
                        entity_id: ev_entity,
                        old_state,
                        new_state,
                        ..
                    },
                    TriggerSpec::State {
                        entity_id,
                        from,
                        to,
                        for_secs,
                    },
                ) if ev_entity == entity_id => {
                    let old_label = old_state.as_ref().map(|s| s.canonical_state_label());
                    let new_label = new_state.canonical_state_label();
                    let to_matches = to.as_deref().is_none_or(|t| t == new_label);
                    let from_matches = from
                        .as_deref()
                        .is_none_or(|f| old_label.as_deref() == Some(f));

                    let key = DebounceKey {
                        automation_id: rule.id.clone(),
                        trigger_idx: idx,
                    };

                    if !to_matches {
                        cancel_debouncer(inner, &key);
                        continue;
                    }
                    if !from_matches {
                        continue;
                    }

                    let summary = format!("state:{entity_id}={new_label}");
                    if *for_secs == 0 {
                        dispatch_trigger(Arc::clone(inner), rule.clone(), summary).await;
                    } else {
                        arm_debouncer(
                            Arc::clone(inner),
                            key,
                            rule.clone(),
                            trigger.clone(),
                            *for_secs,
                            summary,
                        );
                    }
                }
                (
                    CoreEvent::EntityStateChanged {
                        entity_id: ev_entity,
                        old_state,
                        new_state,
                        ..
                    },
                    TriggerSpec::NumericState {
                        entity_id,
                        above,
                        below,
                        for_secs,
                    },
                ) if ev_entity == entity_id => {
                    let Some(new_val) = extract_numeric_value(new_state) else {
                        continue;
                    };
                    let old_val = old_state.as_ref().and_then(extract_numeric_value);

                    let crossed_above =
                        above.is_some_and(|th| new_val > th && old_val.is_none_or(|o| o <= th));
                    let crossed_below =
                        below.is_some_and(|th| new_val < th && old_val.is_none_or(|o| o >= th));
                    let in_band = above.is_some_and(|th| new_val > th)
                        || below.is_some_and(|th| new_val < th);

                    let key = DebounceKey {
                        automation_id: rule.id.clone(),
                        trigger_idx: idx,
                    };

                    if !in_band {
                        cancel_debouncer(inner, &key);
                        continue;
                    }

                    if crossed_above || crossed_below {
                        let summary = format!("numeric_state:{entity_id}={new_val}");
                        if *for_secs == 0 {
                            dispatch_trigger(Arc::clone(inner), rule.clone(), summary).await;
                        } else {
                            arm_debouncer(
                                Arc::clone(inner),
                                key,
                                rule.clone(),
                                trigger.clone(),
                                *for_secs,
                                summary,
                            );
                        }
                    }
                }
                (
                    CoreEvent::WebhookReceived {
                        webhook_id: ev_id, ..
                    },
                    TriggerSpec::Webhook { webhook_id },
                ) if ev_id == webhook_id => {
                    let summary = format!("webhook:{webhook_id}");
                    dispatch_trigger(Arc::clone(inner), rule.clone(), summary).await;
                }
                (ev, TriggerSpec::Event { event_type }) => {
                    let kind = match ev {
                        CoreEvent::EntityStateChanged { .. } => "entity_state_changed",
                        CoreEvent::EntityAvailabilityChanged { .. } => {
                            "entity_availability_changed"
                        }
                        CoreEvent::WebhookReceived { .. } => "webhook_received",
                        CoreEvent::SkillStatusChanged { .. } => "skill_status_changed",
                        CoreEvent::AutomationRunRecorded { .. } => "automation_run_recorded",
                    };
                    if kind == event_type {
                        let summary = format!("event:{event_type}");
                        dispatch_trigger(Arc::clone(inner), rule.clone(), summary).await;
                    }
                }
                _ => {}
            }
        }
    }
}

fn cancel_debouncer(inner: &EngineInner, key: &DebounceKey) {
    if let Some((_, tx)) = inner
        .debouncers
        .lock()
        .expect("debouncers mutex poisoned")
        .remove(key)
    {
        let _ = tx.send(true);
    }
}

fn arm_debouncer(
    inner: Arc<EngineInner>,
    key: DebounceKey,
    rule: AutomationConfig,
    trigger: TriggerSpec,
    for_secs: u64,
    summary: String,
) {
    let my_gen = inner.next_debounce_gen.fetch_add(1, Ordering::Relaxed);
    let (cancel_tx, mut cancel_rx) = watch::channel(false);
    {
        let mut guard = inner.debouncers.lock().expect("debouncers mutex poisoned");
        if let Some((_, prev)) = guard.insert(key.clone(), (my_gen, cancel_tx)) {
            let _ = prev.send(true);
        }
    }

    let mut shutdown_rx = inner.shutdown_rx.clone();
    tokio::spawn(async move {
        let elapsed = tokio::select! {
            biased;
            changed = cancel_rx.changed() => {
                let _ = changed;
                false
            }
            changed = shutdown_rx.changed() => {
                let _ = changed;
                false
            }
            () = tokio::time::sleep(Duration::from_secs(for_secs)) => true,
        };

        // Remove our entry ONLY if a newer timer hasn't already replaced us.
        {
            let mut guard = inner.debouncers.lock().expect("debouncers mutex poisoned");
            if guard.get(&key).is_some_and(|(g, _)| *g == my_gen) {
                guard.remove(&key);
            }
        }

        if !elapsed {
            return;
        }

        // Re-verify the entity still satisfies the held condition before firing.
        let still_holds = match &trigger {
            TriggerSpec::State { entity_id, to, .. } => {
                inner.state.get(entity_id).is_some_and(|e| {
                    to.as_deref()
                        .is_none_or(|t| e.state.canonical_state_label() == t)
                })
            }
            TriggerSpec::NumericState {
                entity_id,
                above,
                below,
                ..
            } => inner
                .state
                .get(entity_id)
                .and_then(|e| extract_numeric_value(&e.state))
                .is_some_and(|v| above.is_some_and(|th| v > th) || below.is_some_and(|th| v < th)),
            _ => true,
        };

        if still_holds {
            dispatch_trigger(inner, rule, format!("{summary}(for:{for_secs}s)")).await;
        }
    });
}

fn get_or_create_slot(inner: &EngineInner, automation_id: &str) -> Arc<AsyncMutex<AutomationSlot>> {
    let mut slots = inner.slots.lock().expect("slots mutex poisoned");
    Arc::clone(
        slots
            .entry(automation_id.to_owned())
            .or_insert_with(|| Arc::new(AsyncMutex::new(AutomationSlot::default()))),
    )
}

async fn dispatch_trigger(
    inner: Arc<EngineInner>,
    rule: AutomationConfig,
    trigger_summary: String,
) {
    let slot = get_or_create_slot(&inner, &rule.id);

    match rule.mode {
        ExecutionMode::Single => {
            let mut guard = slot.lock().await;
            if !guard.active.is_empty() {
                drop(guard);
                let now = now_unix_ms();
                record_run(
                    &inner,
                    AutomationRunRecord {
                        run_id: new_run_id(),
                        automation_id: rule.id,
                        trigger_source: trigger_summary,
                        conditions_passed: false,
                        outcome: AutomationOutcome::SkippedSingle,
                        started_at: now,
                        finished_at: now,
                        steps: json!([]),
                        error: None,
                    },
                )
                .await;
                return;
            }
            let run_id = new_run_id();
            let (cancel_tx, cancel_rx) = watch::channel(false);
            guard.active.push(ActiveRun {
                run_id: run_id.clone(),
                cancel_tx,
            });
            let flight = InFlightGuard::acquire(&inner);
            drop(guard);

            tokio::spawn(async move {
                let _flight = flight;
                execute_run(inner, slot, rule, run_id, trigger_summary, cancel_rx).await;
            });
        }
        ExecutionMode::Restart => {
            let mut guard = slot.lock().await;
            for active in &guard.active {
                let _ = active.cancel_tx.send(true);
            }
            let run_id = new_run_id();
            let (cancel_tx, cancel_rx) = watch::channel(false);
            guard.active.push(ActiveRun {
                run_id: run_id.clone(),
                cancel_tx,
            });
            let flight = InFlightGuard::acquire(&inner);
            drop(guard);

            tokio::spawn(async move {
                let _flight = flight;
                execute_run(inner, slot, rule, run_id, trigger_summary, cancel_rx).await;
            });
        }
        ExecutionMode::Queued => {
            let mut guard = slot.lock().await;
            if guard.queue_worker_running {
                if guard.queue.len() < rule.max_queued {
                    guard.queue.push_back(PendingTrigger { trigger_summary });
                } else {
                    warn!(
                        automation_id = %rule.id,
                        max_queued = rule.max_queued,
                        "automation queue full; dropping trigger"
                    );
                }
                return;
            }
            guard.queue_worker_running = true;
            guard.queue.push_back(PendingTrigger { trigger_summary });
            let flight = InFlightGuard::acquire(&inner);
            let automation_id = rule.id.clone();
            drop(guard);

            tokio::spawn(async move {
                let _flight = flight;
                loop {
                    let next = {
                        let mut guard = slot.lock().await;
                        match guard.queue.pop_front() {
                            Some(item) => {
                                // Re-fetch the latest active rule so deleted/disabled rules after
                                // reload() are never executed from a stale queue entry.
                                let live_rule = inner
                                    .rules
                                    .lock()
                                    .expect("rules mutex poisoned")
                                    .iter()
                                    .find(|r| r.id == automation_id && r.enabled)
                                    .cloned();
                                match live_rule {
                                    Some(live) => {
                                        let run_id = new_run_id();
                                        let (cancel_tx, cancel_rx) = watch::channel(false);
                                        guard.active.push(ActiveRun {
                                            run_id: run_id.clone(),
                                            cancel_tx,
                                        });
                                        Some((live, item, run_id, cancel_rx))
                                    }
                                    None => {
                                        guard.queue.clear();
                                        guard.queue_worker_running = false;
                                        None
                                    }
                                }
                            }
                            None => {
                                guard.queue_worker_running = false;
                                None
                            }
                        }
                    };
                    let Some((live_rule, item, run_id, cancel_rx)) = next else {
                        break;
                    };
                    execute_run(
                        Arc::clone(&inner),
                        Arc::clone(&slot),
                        live_rule,
                        run_id,
                        item.trigger_summary,
                        cancel_rx,
                    )
                    .await;
                }
            });
        }
        ExecutionMode::Parallel => {
            let mut guard = slot.lock().await;
            if guard.active.len() >= rule.max_parallel {
                drop(guard);
                let now = now_unix_ms();
                record_run(
                    &inner,
                    AutomationRunRecord {
                        run_id: new_run_id(),
                        automation_id: rule.id,
                        trigger_source: trigger_summary,
                        conditions_passed: false,
                        outcome: AutomationOutcome::Skipped,
                        started_at: now,
                        finished_at: now,
                        steps: json!([]),
                        error: Some(format!(
                            "max_parallel ({}) limit reached",
                            rule.max_parallel
                        )),
                    },
                )
                .await;
                return;
            }
            let run_id = new_run_id();
            let (cancel_tx, cancel_rx) = watch::channel(false);
            guard.active.push(ActiveRun {
                run_id: run_id.clone(),
                cancel_tx,
            });
            let flight = InFlightGuard::acquire(&inner);
            drop(guard);

            tokio::spawn(async move {
                let _flight = flight;
                execute_run(inner, slot, rule, run_id, trigger_summary, cancel_rx).await;
            });
        }
    }
}

async fn execute_run(
    inner: Arc<EngineInner>,
    slot: Arc<AsyncMutex<AutomationSlot>>,
    rule: AutomationConfig,
    run_id: String,
    trigger_summary: String,
    mut cancel_rx: watch::Receiver<bool>,
) {
    let started_at = now_unix_ms();
    let now_utc = Utc::now();

    let conditions_passed =
        evaluate_conditions(&rule.conditions, &inner.state, &inner.location, now_utc);
    if !conditions_passed {
        let finished_at = now_unix_ms();
        remove_active_run(&slot, &run_id).await;
        record_run(
            &inner,
            AutomationRunRecord {
                run_id,
                automation_id: rule.id,
                trigger_source: trigger_summary,
                conditions_passed: false,
                outcome: AutomationOutcome::ConditionsNotMet,
                started_at,
                finished_at,
                steps: json!([]),
                error: None,
            },
        )
        .await;
        return;
    }

    let mut step_logs = Vec::with_capacity(rule.actions.len());
    let mut final_outcome = AutomationOutcome::Succeeded;
    let mut final_error: Option<String> = None;

    for (idx, action) in rule.actions.iter().enumerate() {
        if is_cancelled(&cancel_rx, &inner.shutdown_rx) {
            final_outcome = AutomationOutcome::Cancelled;
            break;
        }
        let step_res = execute_action(&inner, action, idx, &mut cancel_rx).await;

        let status = step_res.status;
        step_logs.push(step_res.audit_json);
        match status {
            StepStatus::Succeeded => {}
            StepStatus::Cancelled => {
                final_outcome = AutomationOutcome::Cancelled;
                break;
            }
            StepStatus::Failed(err) => {
                final_outcome = AutomationOutcome::Failed;
                final_error = Some(err);
                break;
            }
        }
    }

    let finished_at = now_unix_ms();
    remove_active_run(&slot, &run_id).await;
    record_run(
        &inner,
        AutomationRunRecord {
            run_id,
            automation_id: rule.id,
            trigger_source: trigger_summary,
            conditions_passed: true,
            outcome: final_outcome,
            started_at,
            finished_at,
            steps: Value::Array(step_logs),
            error: final_error,
        },
    )
    .await;
}

enum StepStatus {
    Succeeded,
    Cancelled,
    Failed(String),
}

struct StepResult {
    status: StepStatus,
    audit_json: Value,
}

fn execute_action<'a>(
    inner: &'a Arc<EngineInner>,
    action: &'a ActionSpec,
    idx: usize,
    cancel_rx: &'a mut watch::Receiver<bool>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = StepResult> + Send + 'a>> {
    Box::pin(async move {
        let t0 = Instant::now();
        match action {
            ActionSpec::EntityCall {
                entity_id,
                action: cmd,
                params,
            } => {
                let res = inner
                    .executor
                    .call_entity(entity_id, cmd, params.clone())
                    .await;
                let dur = t0.elapsed().as_millis() as u64;
                match res {
                    Ok(_) => StepResult {
                        status: StepStatus::Succeeded,
                        audit_json: json!({
                            "index": idx,
                            "type": "entity.call",
                            "entity_id": entity_id,
                            "action": cmd,
                            "status": "succeeded",
                            "duration_ms": dur,
                        }),
                    },
                    Err(err) => StepResult {
                        status: StepStatus::Failed(err.clone()),
                        audit_json: json!({
                            "index": idx,
                            "type": "entity.call",
                            "entity_id": entity_id,
                            "action": cmd,
                            "status": "failed",
                            "duration_ms": dur,
                            "error": err,
                        }),
                    },
                }
            }
            ActionSpec::SkillInvoke {
                skill_id,
                action: skill_action,
                timeout_secs,
                context_entities,
                params,
            } => {
                let mut ctx_entities = Map::new();
                for ent_id in context_entities {
                    if let Some(rec) = inner.state.get(ent_id) {
                        let mut snap = serde_json::to_value(&rec).unwrap_or(Value::Null);
                        if let Some(val) = extract_numeric_value(&rec.state)
                            && let Some(obj) = snap.as_object_mut()
                        {
                            obj.insert("value".into(), json!(val));
                        }
                        ctx_entities.insert(ent_id.clone(), snap);
                    }
                }
                let mut context = Map::new();
                if !ctx_entities.is_empty() {
                    context.insert("entities".into(), Value::Object(ctx_entities));
                }

                let timeout =
                    Duration::from_secs(timeout_secs.unwrap_or(DEFAULT_SKILL_INVOKE_TIMEOUT_SECS));
                let res = inner
                    .executor
                    .invoke_skill(skill_id, skill_action, params.clone(), context, timeout)
                    .await;
                let dur = t0.elapsed().as_millis() as u64;
                match res {
                    Ok(output) => StepResult {
                        status: StepStatus::Succeeded,
                        audit_json: json!({
                            "index": idx,
                            "type": "skill.invoke",
                            "skill_id": skill_id,
                            "action": skill_action,
                            "status": "succeeded",
                            "duration_ms": dur,
                            "result": output,
                        }),
                    },
                    Err(err) => StepResult {
                        status: StepStatus::Failed(err.clone()),
                        audit_json: json!({
                            "index": idx,
                            "type": "skill.invoke",
                            "skill_id": skill_id,
                            "action": skill_action,
                            "status": "failed",
                            "duration_ms": dur,
                            "error": err,
                        }),
                    },
                }
            }
            ActionSpec::Delay { secs } => {
                let mut shutdown_rx = inner.shutdown_rx.clone();
                let completed = tokio::select! {
                    biased;
                    changed = cancel_rx.changed() => {
                        let _ = changed;
                        false
                    }
                    changed = shutdown_rx.changed() => {
                        let _ = changed;
                        false
                    }
                    () = tokio::time::sleep(Duration::from_secs(*secs)) => true,
                };
                let dur = t0.elapsed().as_millis() as u64;
                if completed {
                    StepResult {
                        status: StepStatus::Succeeded,
                        audit_json: json!({
                            "index": idx,
                            "type": "delay",
                            "secs": secs,
                            "status": "succeeded",
                            "duration_ms": dur,
                        }),
                    }
                } else {
                    StepResult {
                        status: StepStatus::Cancelled,
                        audit_json: json!({
                            "index": idx,
                            "type": "delay",
                            "secs": secs,
                            "status": "cancelled",
                            "duration_ms": dur,
                        }),
                    }
                }
            }
            ActionSpec::Parallel { steps } => {
                let mut set = JoinSet::new();
                for (sub_idx, sub_step) in steps.iter().cloned().enumerate() {
                    let inner_clone = Arc::clone(inner);
                    let mut sub_cancel = cancel_rx.clone();
                    set.spawn(async move {
                        execute_action(&inner_clone, &sub_step, sub_idx, &mut sub_cancel).await
                    });
                }

                let mut sub_logs = Vec::with_capacity(steps.len());
                let mut any_cancelled = false;
                let mut first_err: Option<String> = None;

                while let Some(joined) = set.join_next().await {
                    match joined {
                        Ok(res) => {
                            match res.status {
                                StepStatus::Succeeded => {}
                                StepStatus::Cancelled => any_cancelled = true,
                                StepStatus::Failed(e) => {
                                    if first_err.is_none() {
                                        first_err = Some(e);
                                    }
                                }
                            }
                            sub_logs.push(res.audit_json);
                        }
                        Err(err) => {
                            if first_err.is_none() {
                                first_err = Some(err.to_string());
                            }
                        }
                    }
                }

                let dur = t0.elapsed().as_millis() as u64;
                let (status, status_str) = if any_cancelled {
                    (StepStatus::Cancelled, "cancelled")
                } else if let Some(err) = first_err {
                    (StepStatus::Failed(err), "failed")
                } else {
                    (StepStatus::Succeeded, "succeeded")
                };

                StepResult {
                    status,
                    audit_json: json!({
                        "index": idx,
                        "type": "parallel",
                        "status": status_str,
                        "duration_ms": dur,
                        "steps": sub_logs,
                    }),
                }
            }
        }
    })
}

/// Evaluate a list of top-level conditions with AND semantics.
pub fn evaluate_conditions(
    conditions: &[ConditionSpec],
    state: &StateStore,
    location: &LocationConfig,
    now: DateTime<Utc>,
) -> bool {
    conditions
        .iter()
        .all(|c| evaluate_condition(c, state, location, now))
}

fn evaluate_condition(
    cond: &ConditionSpec,
    state: &StateStore,
    location: &LocationConfig,
    now: DateTime<Utc>,
) -> bool {
    match cond {
        ConditionSpec::State { entity_id, equals } => state
            .get(entity_id)
            .is_some_and(|e| e.state.canonical_state_label() == *equals),
        ConditionSpec::NumericState {
            entity_id,
            above,
            below,
        } => state
            .get(entity_id)
            .and_then(|e| extract_numeric_value(&e.state))
            .is_some_and(|v| above.is_none_or(|a| v > a) && below.is_none_or(|b| v < b)),
        ConditionSpec::Time {
            after,
            before,
            weekdays,
        } => {
            let wd = Weekday::from_chrono(now.weekday());
            if !weekdays.is_empty() && !weekdays.contains(&wd) {
                return false;
            }
            let after_t = after.as_deref().and_then(|s| parse_wall_time(s).ok());
            let before_t = before.as_deref().and_then(|s| parse_wall_time(s).ok());
            time_in_window(now.time(), after_t, before_t)
        }
        ConditionSpec::Sun {
            after,
            after_offset_minutes,
            before,
            before_offset_minutes,
        } => {
            let (Some(lat), Some(lon)) = (location.latitude, location.longitude) else {
                return false;
            };
            // Shift into local solar time (lon / 15° hours) so sunrise (~06:00) and sunset (~18:00)
            // never wrap across 00:00 UTC for non-UTC longitudes.
            let solar_offset = ChronoDuration::seconds((lon / 15.0 * 3600.0).round() as i64);
            let local_now = now + solar_offset;
            let local_date = local_now.date_naive();

            let after_t = match after {
                Some(ev) => match compute_sun_time_utc(local_date, lat, lon, *ev) {
                    Some(t) => Some(
                        t + solar_offset
                            + ChronoDuration::minutes(i64::from(*after_offset_minutes)),
                    ),
                    // Polar day / polar night: requested solar event does not occur.
                    None => return false,
                },
                None => None,
            };
            let before_t = match before {
                Some(ev) => match compute_sun_time_utc(local_date, lat, lon, *ev) {
                    Some(t) => Some(
                        t + solar_offset
                            + ChronoDuration::minutes(i64::from(*before_offset_minutes)),
                    ),
                    None => return false,
                },
                None => None,
            };
            time_in_window(local_now.time(), after_t, before_t)
        }
        ConditionSpec::And { conditions } => conditions
            .iter()
            .all(|c| evaluate_condition(c, state, location, now)),
        ConditionSpec::Or { conditions } => conditions
            .iter()
            .any(|c| evaluate_condition(c, state, location, now)),
        ConditionSpec::Not {
            condition,
            conditions,
        } => {
            let single_ok = condition
                .as_deref()
                .is_none_or(|c| evaluate_condition(c, state, location, now));
            let list_ok = conditions
                .iter()
                .all(|c| evaluate_condition(c, state, location, now));
            !(single_ok && list_ok)
        }
    }
}

async fn remove_active_run(slot: &AsyncMutex<AutomationSlot>, run_id: &str) {
    let mut guard = slot.lock().await;
    guard.active.retain(|r| r.run_id != run_id);
}

fn is_cancelled(cancel_rx: &watch::Receiver<bool>, shutdown_rx: &watch::Receiver<bool>) -> bool {
    *cancel_rx.borrow() || *shutdown_rx.borrow() || shutdown_rx.has_changed().is_err()
}

async fn record_run(inner: &EngineInner, record: AutomationRunRecord) {
    let run_id = record.run_id.clone();
    let automation_id = record.automation_id.clone();
    let outcome = record.outcome;
    let at = record.finished_at;

    if let Err(err) = inner.history.record_automation_run(&record).await {
        warn!(
            automation_id = %automation_id,
            run_id = %run_id,
            error = %err,
            "failed to persist automation run audit record"
        );
    }

    inner.events.publish(CoreEvent::AutomationRunRecorded {
        run_id,
        automation_id,
        outcome,
        at,
    });
}
