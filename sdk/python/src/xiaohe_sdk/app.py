"""Async JSON-RPC 2.0 over stdio runtime helper for Xiaohe Skills."""

from __future__ import annotations

import asyncio
import inspect
import json
import sys
from collections.abc import Awaitable, Callable
from typing import Any, TextIO

JSONRPC_VERSION = "2.0"
ERR_METHOD_NOT_FOUND = -32601
ERR_INVALID_PARAMS = -32602
ERR_INTERNAL_ERROR = -32603


class RpcError(Exception):
    """Raised when a JSON-RPC 2.0 callback to Xiaohe Core returns an error."""

    def __init__(self, code: int, message: str, data: Any = None) -> None:
        super().__init__(f"JSON-RPC error {code}: {message}")
        self.code = code
        self.message = message
        self.data = data


class SkillApp:
    """Ergonomic async application runner for both `device` and `action` Xiaohe Skills.

    All protocol traffic uses NDJSON on `stdin`/`stdout`; logs are written exclusively to `stderr`.
    """

    def __init__(
        self,
        *,
        stdin: TextIO | None = None,
        stdout: TextIO | None = None,
        stderr: TextIO | None = None,
    ) -> None:
        self._stdin = stdin if stdin is not None else sys.stdin
        self._stdout = stdout if stdout is not None else sys.stdout
        self._stderr = stderr if stderr is not None else sys.stderr

        self._init_handler: Callable[[dict[str, Any], dict[str, str]], Awaitable[Any]] | None = None
        self._shutdown_handler: Callable[[], Awaitable[Any]] | None = None
        self._command_handlers: dict[str, Callable[..., Awaitable[Any]]] = {}
        self._default_command_handler: Callable[..., Awaitable[Any]] | None = None
        self._invoke_handlers: dict[str, Callable[..., Awaitable[Any]]] = {}
        self._default_invoke_handler: Callable[..., Awaitable[Any]] | None = None

        self._write_lock = asyncio.Lock()
        self._next_cb_id = 1000
        self._pending_callbacks: dict[int | str, asyncio.Future[Any]] = {}
        self._background_tasks: set[asyncio.Task[Any]] = set()
        self._stop_event = asyncio.Event()
        self.skill_id: str = ""
        self.config: dict[str, Any] = {}
        self.secrets: dict[str, str] = {}

    # ── Decorators ────────────────────────────────────────────────────────────

    def on_init(
        self, fn: Callable[[dict[str, Any], dict[str, str]], Awaitable[Any]]
    ) -> Callable[[dict[str, Any], dict[str, str]], Awaitable[Any]]:
        """Register the `skill.init` handler: `async def init(config, secrets)`."""
        self._init_handler = fn
        return fn

    def on_shutdown(self, fn: Callable[[], Awaitable[Any]]) -> Callable[[], Awaitable[Any]]:
        """Register an optional graceful shutdown hook."""
        self._shutdown_handler = fn
        return fn

    def on_command(
        self, entity_id: str | None = None
    ) -> Callable[[Callable[..., Awaitable[Any]]], Callable[..., Awaitable[Any]]]:
        """Register an `entity.command` handler for a specific entity_id or catch-all."""

        def decorator(fn: Callable[..., Awaitable[Any]]) -> Callable[..., Awaitable[Any]]:
            if entity_id is None:
                self._default_command_handler = fn
            else:
                self._command_handlers[entity_id] = fn
            return fn

        return decorator

    def on_invoke(
        self, action: str | None = None
    ) -> Callable[[Callable[..., Awaitable[Any]]], Callable[..., Awaitable[Any]]]:
        """Register a `skill.invoke` handler for a specific action name or catch-all."""

        def decorator(fn: Callable[..., Awaitable[Any]]) -> Callable[..., Awaitable[Any]]:
            if action is None:
                self._default_invoke_handler = fn
            else:
                self._invoke_handlers[action] = fn
            return fn

        return decorator

    # ── Logging & Outbound Helpers ────────────────────────────────────────────

    def log(self, message: str) -> None:
        """Write a diagnostic line to `stderr` (never touches `stdout`)."""
        self._stderr.write(f"{message}\n")
        self._stderr.flush()

    async def register_entities(
        self, *, device: dict[str, Any], entities: list[dict[str, Any]]
    ) -> None:
        """Emit an `entity.register` notification to Core."""
        await self._send_json(
            {
                "jsonrpc": JSONRPC_VERSION,
                "method": "entity.register",
                "params": {"device": device, "entities": entities},
            }
        )

    async def emit_state(
        self,
        entity_id: str,
        state: dict[str, Any],
        *,
        availability: str = "online",
        extra: dict[str, Any] | None = None,
    ) -> None:
        """Emit an `entity.state_changed` notification to Core."""
        params: dict[str, Any] = {
            "entity_id": entity_id,
            "availability": availability,
            "state": state,
        }
        if extra:
            params["extra"] = extra
        await self._send_json(
            {
                "jsonrpc": JSONRPC_VERSION,
                "method": "entity.state_changed",
                "params": params,
            }
        )

    async def get_entity(self, entity_id: str, *, timeout: float = 10.0) -> dict[str, Any]:
        """Call back to Core via `core.get_entity` and return the current EntityRecord."""
        result = await self._call_core(
            "core.get_entity", {"entity_id": entity_id}, timeout=timeout
        )
        return result

    async def call_entity(
        self,
        entity_id: str,
        action: str,
        params: dict[str, Any] | None = None,
        *,
        timeout: float = 15.0,
    ) -> Any:
        """Call back to Core via `core.call_entity` to command another entity."""
        return await self._call_core(
            "core.call_entity",
            {"entity_id": entity_id, "action": action, "params": params or {}},
            timeout=timeout,
        )

    def spawn_poll_loop(
        self, coro_fn: Callable[[], Awaitable[Any]], *, interval: float
    ) -> asyncio.Task[None]:
        """Spawn a resilient background polling task that runs until `skill.shutdown`."""
        sleep_secs = max(0.05, float(interval))

        async def _runner() -> None:
            while not self._stop_event.is_set():
                try:
                    await coro_fn()
                except asyncio.CancelledError:
                    raise
                except Exception as exc:  # noqa: BLE001
                    self.log(f"poll loop error: {exc}")
                try:
                    await asyncio.wait_for(self._stop_event.wait(), timeout=sleep_secs)
                except TimeoutError:
                    pass

        task = asyncio.create_task(_runner())
        self._track_task(task)
        return task

    # ── Event Loop Runner ─────────────────────────────────────────────────────

    def run(self) -> None:
        """Run the Skill event loop until `skill.shutdown` or EOF on `stdin`."""
        asyncio.run(self.run_async())

    async def run_async(self) -> None:
        """Async entrypoint (also directly usable in unit tests with custom streams)."""
        self._stop_event.clear()
        try:
            while not self._stop_event.is_set():
                line = await asyncio.to_thread(self._stdin.readline)
                if not line:
                    break
                trimmed = line.strip()
                if not trimmed:
                    continue
                try:
                    msg = json.loads(trimmed)
                except json.JSONDecodeError as exc:
                    self.log(f"ignoring malformed JSON line on stdin: {exc}")
                    continue
                self._dispatch_message(msg)
        finally:
            self._stop_event.set()
            for fut in list(self._pending_callbacks.values()):
                if not fut.done():
                    fut.set_exception(RpcError(ERR_INTERNAL_ERROR, "skill shutting down"))
            self._pending_callbacks.clear()

            for task in list(self._background_tasks):
                task.cancel()
            if self._background_tasks:
                await asyncio.gather(*self._background_tasks, return_exceptions=True)

            if self._shutdown_handler is not None:
                try:
                    await self._shutdown_handler()
                except Exception as exc:  # noqa: BLE001
                    self.log(f"shutdown handler error: {exc}")

    # ── Internal JSON-RPC Dispatch ────────────────────────────────────────────

    def _track_task(self, task: asyncio.Task[Any]) -> None:
        self._background_tasks.add(task)
        task.add_done_callback(self._background_tasks.discard)

    async def _send_json(self, obj: dict[str, Any]) -> None:
        payload = json.dumps(obj, separators=(",", ":")) + "\n"
        async with self._write_lock:
            self._stdout.write(payload)
            self._stdout.flush()

    async def _call_core(
        self, method: str, params: dict[str, Any], *, timeout: float
    ) -> Any:
        loop = asyncio.get_running_loop()
        req_id = self._next_cb_id
        self._next_cb_id += 1
        fut: asyncio.Future[Any] = loop.create_future()
        self._pending_callbacks[req_id] = fut

        await self._send_json(
            {
                "jsonrpc": JSONRPC_VERSION,
                "id": req_id,
                "method": method,
                "params": params,
            }
        )
        try:
            return await asyncio.wait_for(fut, timeout=timeout)
        finally:
            self._pending_callbacks.pop(req_id, None)

    def _dispatch_message(self, msg: dict[str, Any]) -> None:
        # Case 1: Response to our outbound core.* callback
        if "method" not in msg:
            resp_id = msg.get("id")
            fut = self._pending_callbacks.pop(resp_id, None)
            if fut is not None and not fut.done():
                if "error" in msg and msg["error"] is not None:
                    err = msg["error"]
                    fut.set_exception(
                        RpcError(
                            int(err.get("code", ERR_INTERNAL_ERROR)),
                            str(err.get("message", "unknown error")),
                            err.get("data"),
                        )
                    )
                else:
                    fut.set_result(msg.get("result"))
            return

        method = msg.get("method")
        req_id = msg.get("id")
        params = msg.get("params") or {}

        if method == "skill.shutdown":
            self._stop_event.set()
            return

        if method == "skill.ping" and req_id is not None:
            task = asyncio.create_task(self._send_ok(req_id, {"ok": True}))
            self._track_task(task)
            return

        if req_id is None:
            return

        task = asyncio.create_task(self._handle_request(req_id, str(method), params))
        self._track_task(task)

    async def _handle_request(
        self, req_id: int | str, method: str, params: dict[str, Any]
    ) -> None:
        try:
            if method == "skill.init":
                self.skill_id = str(params.get("skill_id", ""))
                self.config = dict(params.get("config") or {})
                self.secrets = dict(params.get("secrets") or {})
                if self._init_handler is not None:
                    await _maybe_await(self._init_handler(self.config, self.secrets))
                await self._send_ok(req_id, {"ok": True})
                return

            if method == "entity.command":
                entity_id = params.get("entity_id")
                action = params.get("action")
                cmd_params = dict(params.get("params") or {})
                if not entity_id or not action:
                    await self._send_err(
                        req_id, ERR_INVALID_PARAMS, "entity_id and action are required"
                    )
                    return

                if entity_id in self._command_handlers:
                    res = await _maybe_await(
                        self._command_handlers[entity_id](action, cmd_params)
                    )
                elif self._default_command_handler is not None:
                    res = await _maybe_await(
                        self._default_command_handler(entity_id, action, cmd_params)
                    )
                else:
                    await self._send_err(
                        req_id, ERR_METHOD_NOT_FOUND, f"no command handler for '{entity_id}'"
                    )
                    return
                await self._send_ok(req_id, {"ok": True} if res is None else res)
                return

            if method == "skill.invoke":
                action = params.get("action")
                invoke_params = dict(params.get("params") or {})
                context = dict(params.get("context") or {})
                if not action:
                    await self._send_err(req_id, ERR_INVALID_PARAMS, "action is required")
                    return

                if action in self._invoke_handlers:
                    res = await _maybe_await(
                        self._invoke_handlers[action](invoke_params, context)
                    )
                elif self._default_invoke_handler is not None:
                    res = await _maybe_await(
                        self._default_invoke_handler(action, invoke_params, context)
                    )
                else:
                    await self._send_err(
                        req_id, ERR_METHOD_NOT_FOUND, f"unknown action '{action}'"
                    )
                    return
                await self._send_ok(req_id, {"ok": True} if res is None else res)
                return

            await self._send_err(req_id, ERR_METHOD_NOT_FOUND, f"unknown method '{method}'")
        except RpcError as exc:
            await self._send_err(req_id, exc.code, exc.message)
        except Exception as exc:  # noqa: BLE001
            self.log(f"unhandled error in {method}: {exc}")
            await self._send_err(req_id, ERR_INTERNAL_ERROR, str(exc))

    async def _send_ok(self, req_id: int | str, result: Any) -> None:
        await self._send_json(
            {"jsonrpc": JSONRPC_VERSION, "id": req_id, "result": result}
        )

    async def _send_err(self, req_id: int | str, code: int, message: str) -> None:
        await self._send_json(
            {
                "jsonrpc": JSONRPC_VERSION,
                "id": req_id,
                "error": {"code": code, "message": message},
            }
        )


async def _maybe_await(value: Any) -> Any:
    if inspect.isawaitable(value):
        return await value
    return value
