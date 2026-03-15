"""agent_loop_pdca_v2.py

用“新设计（Router + PDCA + Self-Improve + 多 Session 调度 + SubAgent 并行）”
改造旧版 agent_loop.py 的 Python 伪代码。

目标：把旧版的“单事件->Resolver->Router->Executor 多步循环”改造成：

  Global Event Inbox
        |
  resolve_session (只负责确定 session_id)
        |
  enqueue to Session.inbox
        |
  Agent Main Scheduler (多 Session 轮询，一个 step 一次推理)
        |
  Session Step (串行)：ROUTER -> PLAN -> DO -> CHECK -> ADJUST -> (SELF_IMPROVE)
        |
  SubAgent 并行：dispatch 后通过事件回传唤醒主 Session

说明：
- 这是“可运行结构”的伪代码：接口/存储/LLM/工具均为占位符。
- 重点展示：
  1) Agent Main Loop（全局调度）
  2) Session Loop（session 内严格串行，step 边界可切换 session）
  3) wait_events / 唤醒逻辑
  4) SubAgent 并行与回传事件
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum, auto
from typing import Any, Dict, List, Optional, Literal, TypedDict, Tuple
import time


# ============================================================
# 0) 行为枚举（新设计核心）
# ============================================================


class Behavior(Enum):
    ROUTER = auto()       # resolve-router/router：快速回应 + 决定 next_behavior
    PLAN = auto()         # 只读 workspace（不改交付物），生成/调整 TODO
    DO = auto()           # 执行 TODO（写 workspace），可派发 SubAgent 并行
    CHECK = auto()        # 验收（不修复），Complete->Done；失败->CHECK_FAILED
    ADJUST = auto()       # 失败归因/调整计划（readonly 为主）
    SELF_IMPROVE = auto() # 记忆/摘要/自我改进/给 KB Agent 发需求
    END = auto()


# ============================================================
# 1) 事件与等待
# ============================================================


class EventType(Enum):
    USER_MSG = auto()
    SYSTEM_EVENT = auto()
    TOOL_RESULT = auto()
    SUBAGENT_DONE = auto()
    TIMER = auto()
    SYSTEM_RESTART = auto()


@dataclass
class Event:
    type: EventType
    payload: Dict[str, Any]
    session_id: Optional[str] = None
    event_id: str = ""
    run_id: str = ""
    ts: float = field(default_factory=lambda: time.time())


@dataclass
class WaitEvent:
    """主 Agent 的等待条件；被满足时 session 从 waiting -> runnable"""

    type: Literal[
        "WAIT_USER_INPUT",
        "WAIT_USER_APPROVAL",
        "WAIT_SUBAGENT",
        "WAIT_SYSTEM_EVENT",
    ]
    key: str
    detail: Dict[str, Any] = field(default_factory=dict)


# ============================================================
# 2) Session / Workspace（新设计：Session topic + Workspace 交付）
# ============================================================


@dataclass
class SessionMeta:
    id: str
    title: str
    summary: str
    status: Literal["active", "archived", "deleted"] = "active"
    created_ts: float = field(default_factory=lambda: time.time())
    last_activity_ts: float = field(default_factory=lambda: time.time())

    # 新设计：session 绑定 workspace_id（workspace 生命周期不绑定 session）
    workspace_id: Optional[str] = None


@dataclass
class Provenance:
    session_id: str
    run_id: str
    event_id: str
    source_type: Literal["user", "tool", "workspace", "system", "other_agent"]
    source_ref: str
    ts: float = field(default_factory=lambda: time.time())


@dataclass
class TodoItem:
    """新设计 TODO：更接近工程可执行的结构"""

    todo_id: str
    title: str
    type: Literal["TASK", "BENCH"] = "TASK"
    status: Literal[
        "NEW",
        "WAIT",
        "IN_PROGRESS",
        "COMPLETE",
        "DONE",
        "FAILED",
        "CHECK_FAILED",
    ] = "NEW"

    deps: List[str] = field(default_factory=list)
    skills: List[str] = field(default_factory=list)
    assignee: str = "MAIN"  # MAIN | SUBAGENT:<name>
    can_start_immediately: bool = False

    acceptance_criteria: List[str] = field(default_factory=list)
    artifacts: List[str] = field(default_factory=list)
    blockers: List[str] = field(default_factory=list)

    next_action: str = ""  # 可选：给执行器的最近一步提示
    provenance: Optional[Provenance] = None


@dataclass
class WorkspaceSnapshot:
    summary: str
    recent_changes: List[str]
    errors: List[str] = field(default_factory=list)


@dataclass
class SessionDomainState:
    """旧版 SessionState 中的“长期状态”，保留：记忆/事实/日志/产物等。
    TODO 不再建议放在 session state（应在 workspace），但允许缓存。
    """

    working_memory: List[Dict[str, Any]] = field(default_factory=list)
    facts: List[Dict[str, Any]] = field(default_factory=list)
    worklog: List[Dict[str, Any]] = field(default_factory=list)
    artifacts: List[Dict[str, Any]] = field(default_factory=list)
    last_workspace_snapshot: Optional[WorkspaceSnapshot] = None


@dataclass
class SessionExecState:
    """新设计：session 内“运行态”，支持串行 step + wait + 并发调度"""

    behavior: Behavior = Behavior.ROUTER
    step_seq: int = 0

    inbox: List[Event] = field(default_factory=list)
    waiting: bool = False
    wait_events: List[WaitEvent] = field(default_factory=list)

    # Do/Check 输入关键：当前 todo
    current_todo_id: Optional[str] = None

    # tool call 的“意图与结果”建议记录到 workspace worklog，这里只存 pending
    pending_tool_calls: List[Dict[str, Any]] = field(default_factory=list)

    # 上一步摘要（注入下一步 input）
    last_step_summary: str = ""

    # todo 级别的自修复计数
    retry_counter: Dict[str, int] = field(default_factory=dict)


@dataclass
class Session:
    meta: SessionMeta
    domain: SessionDomainState = field(default_factory=SessionDomainState)
    exec: SessionExecState = field(default_factory=SessionExecState)


# ============================================================
# 3) Store 接口（保留旧版 SessionStore，并新增 WorkspaceStore）
# ============================================================


class SessionStore:
    def get(self, session_id: str) -> Optional[Session]:
        raise NotImplementedError

    def create(self, meta: SessionMeta) -> Session:
        raise NotImplementedError

    def upsert_meta(self, meta: SessionMeta) -> None:
        raise NotImplementedError

    def save_session(self, session: Session) -> None:
        raise NotImplementedError

    def list_recent_metas(self, limit: int = 20) -> List[SessionMeta]:
        raise NotImplementedError


class WorkspaceStore:
    """Workspace：交付空间；TODO/worklog/产物都应归档在 workspace。

    关键约束：workspace 生命周期不绑定 session。
    """

    def create_workspace(self) -> str:
        raise NotImplementedError

    def load_todo(self, workspace_id: str, session_id: str) -> List[TodoItem]:
        raise NotImplementedError

    def upsert_todo(self, workspace_id: str, session_id: str, todo: TodoItem) -> None:
        raise NotImplementedError

    def bulk_patch_todos(self, workspace_id: str, session_id: str, patches: List[Dict[str, Any]]) -> None:
        raise NotImplementedError

    def append_worklog(self, workspace_id: str, session_id: str, entry: Dict[str, Any]) -> None:
        raise NotImplementedError

    def observe(self, workspace_id: str, level: Literal["none", "light", "deep"]) -> WorkspaceSnapshot:
        raise NotImplementedError


class MemoryIndex:
    def search(self, queries: List[str], session_id: Optional[str], boost: float, limit: int) -> List[Dict[str, Any]]:
        raise NotImplementedError


# ============================================================
# 4) LLM 输出协议（新设计：RouterResult + BehaviorLLMResult）
# ============================================================


class RouterResult(TypedDict):
    """ROUTER 行为只做快速回应 + next_behavior 决策（不做重执行）"""

    immediate_reply: Optional[str]
    next_behavior: Literal["PLAN", "END"]
    reason: str

    # 可选：需要的检索/工具（为 PLAN 做准备）
    memory_queries: List[str]
    workspace_need: Literal["none", "light", "deep"]
    tool_calls: List[Dict[str, Any]]
    risk_flags: List[str]


class BehaviorLLMResult(TypedDict):
    """PLAN/DO/CHECK/ADJUST/SELF_IMPROVE 通用输出协议"""

    behavior: Literal["PLAN", "DO", "CHECK", "ADJUST", "SELF_IMPROVE"]

    step: Dict[str, Any]  # {"step_id":..., "is_wait":bool, "wait_events":[...]}

    actions: List[Dict[str, Any]]  # SEND_MSG/CALL_TOOL/DISPATCH_SUBAGENT/UPDATE_TODO/...
    todo_updates: List[Dict[str, Any]]

    next: Dict[str, Any]  # {"next_behavior":"DO|CHECK|ADJUST|SELF_IMPROVE|END", "hint":...}
    user_reply: Dict[str, Any]  # {"should_reply":bool, "message":str|null}
    last_step_summary: str
    diagnostics: Dict[str, Any]


# ============================================================
# 5) Session Resolver（沿用旧版思想：event.session_id 优先，否则推断）
# ============================================================


def new_id() -> str:
    return f"ulid_{int(time.time() * 1000)}"


def resolve_session_id(ev: Event, store: SessionStore) -> str:
    """只负责确定 session_id：
    - event 带 session_id 且存在 -> 用之
    - 否则：调用 LLM 做 session resolver（这里省略 prompt）
    """

    if ev.session_id:
        sess = store.get(ev.session_id)
        if sess and sess.meta.status != "deleted":
            return sess.meta.id

    # 伪：真实实现应调用 session_resolver prompt
    # 这里直接创建新 session（保守）
    meta = SessionMeta(
        id=new_id(),
        title="Untitled Session",
        summary="",
        status="active",
    )
    sess = store.create(meta)
    return sess.meta.id


# ============================================================
# 6) Agent Main Loop（多 Session 调度：并发切换发生在 step 边界）
# ============================================================


@dataclass
class AgentState:
    agent_id: str
    sessions: Dict[str, Session] = field(default_factory=dict)

    global_inbox: List[Event] = field(default_factory=list)
    runnable_sessions: List[str] = field(default_factory=list)
    session_locks: Dict[str, bool] = field(default_factory=dict)


class AgentRuntime:
    def __init__(
        self,
        state: AgentState,
        session_store: SessionStore,
        workspace_store: WorkspaceStore,
        memory_index: MemoryIndex,
        llm,
        tools,
        subagent_pool,
    ):
        self.S = state
        self.session_store = session_store
        self.workspace_store = workspace_store
        self.memory_index = memory_index
        self.llm = llm
        self.tools = tools
        self.subagent_pool = subagent_pool

    # -----------------------------
    # Main Loop
    # -----------------------------

    def run_forever(self):
        """全局调度：
        - ingest 全局事件
        - enqueue 到 session inbox
        - 轮询挑一个 runnable session，执行一个 step
        - step 结束后回到调度点（这就是 session 间“跳转”的边界）
        """

        while True:
            # 0) 拉取外部事件（阻塞 or 非阻塞都可）
            ext = self._poll_external_events()
            for ev in ext:
                self.S.global_inbox.append(self._enrich_event_ids(ev))

            # 1) 分发 global inbox -> session inbox
            self._drain_global_inbox_and_enqueue()

            # 2) 选择一个 runnable session 执行 1 个 step
            sid = self._pick_next_runnable_session()
            if sid is None:
                # 没有可运行 session：阻塞等待（避免空转）
                self._sleep_until_new_event()
                continue

            self._run_one_session_step(sid)

    # -----------------------------
    # Ingest / Routing
    # -----------------------------

    def _drain_global_inbox_and_enqueue(self):
        items = self.S.global_inbox
        self.S.global_inbox = []
        for ev in items:
            sid = resolve_session_id(ev, self.session_store)
            sess = self._load_or_attach_session(sid)
            ev.session_id = sid
            sess.exec.inbox.append(ev)
            self._mark_runnable(sid)

    def _load_or_attach_session(self, session_id: str) -> Session:
        if session_id in self.S.sessions:
            return self.S.sessions[session_id]
        sess = self.session_store.get(session_id)
        if not sess:
            # 防御：store 丢了就新建
            meta = SessionMeta(id=session_id, title="Recovered Session", summary="")
            sess = self.session_store.create(meta)
        self.S.sessions[session_id] = sess
        return sess

    # -----------------------------
    # Scheduler
    # -----------------------------

    def _pick_next_runnable_session(self) -> Optional[str]:
        if not self.S.runnable_sessions:
            return None
        # 简单轮询：队首
        sid = self.S.runnable_sessions.pop(0)
        sess = self.S.sessions.get(sid)
        if not sess:
            return None
        # waiting 且无法唤醒：跳过
        if sess.exec.waiting and not self._can_wake_session(sess):
            return None
        return sid

    def _mark_runnable(self, session_id: str):
        if session_id not in self.S.runnable_sessions:
            self.S.runnable_sessions.append(session_id)

    def _mark_not_runnable(self, session_id: str):
        self.S.runnable_sessions = [x for x in self.S.runnable_sessions if x != session_id]

    # -----------------------------
    # Session Step（串行执行）
    # -----------------------------

    def _run_one_session_step(self, session_id: str):
        if self.S.session_locks.get(session_id, False):
            return
        self.S.session_locks[session_id] = True

        try:
            sess = self.S.sessions[session_id]

            # A) waiting -> wake
            if sess.exec.waiting and self._can_wake_session(sess):
                sess.exec.waiting = False
                sess.exec.wait_events = []

            # B) 确保 workspace（PLAN 会初始化；但为了后续能 load_todo，这里也可懒创建）
            if not sess.meta.workspace_id:
                sess.meta.workspace_id = self.workspace_store.create_workspace()
                self.session_store.upsert_meta(sess.meta)

            # C) 生成 step input：无 input -> 跳过并标记不可运行
            step_input = self._generate_step_input(sess)
            if step_input is None:
                self._mark_not_runnable(session_id)
                return

            # D) 执行当前 behavior 的一步
            if sess.exec.behavior == Behavior.ROUTER:
                out = self._step_router(sess, step_input)
                self._apply_router_result(sess, out)
            else:
                out = self._step_behavior(sess, step_input)
                self._apply_behavior_result(sess, out)

            # E) step 完成后持久化（满足“推理后立刻保存状态”）
            sess.exec.step_seq += 1
            sess.meta.last_activity_ts = time.time()
            self.session_store.save_session(sess)

            # F) 决定是否继续 runnable
            if self._has_more_work(sess):
                self._mark_runnable(session_id)
            else:
                self._mark_not_runnable(session_id)

        finally:
            self.S.session_locks[session_id] = False

    # -----------------------------
    # Wake / Input
    # -----------------------------

    def _can_wake_session(self, sess: Session) -> bool:
        """满足任一 wait_event 即可唤醒"""
        if not sess.exec.wait_events:
            return False
        for ev in sess.exec.inbox:
            for w in sess.exec.wait_events:
                if self._match_wait_event(w, ev):
                    return True
        return False

    def _match_wait_event(self, w: WaitEvent, ev: Event) -> bool:
        if w.type == "WAIT_USER_INPUT" and ev.type == EventType.USER_MSG:
            return True
        if w.type == "WAIT_SUBAGENT" and ev.type == EventType.SUBAGENT_DONE:
            return ev.payload.get("task_id") == w.key
        if w.type == "WAIT_SYSTEM_EVENT" and ev.type == EventType.SYSTEM_EVENT:
            return ev.payload.get("key") == w.key
        if w.type == "WAIT_USER_APPROVAL" and ev.type == EventType.USER_MSG:
            # 伪：真实应解析用户授权指令/按钮事件
            return w.key in (ev.payload.get("message") or "")
        return False

    def _generate_step_input(self, sess: Session) -> Optional[Dict[str, Any]]:
        """生成 user prompt 的 Input 部分；无 input 时返回 None -> skip step。

        新设计要求：
        - 每 step 都能看到“新的 msg/event”
        - DO/CHECK 需要 Current Todo Details（否则 skip）
        """

        new_events = sess.exec.inbox
        if not new_events and not self._has_executable_todo(sess):
            return None

        # DO/CHECK：必须选出 current todo
        if sess.exec.behavior in (Behavior.DO, Behavior.CHECK):
            todo = self._select_current_todo(sess)
            if not todo:
                return None
            sess.exec.current_todo_id = todo.todo_id

        current_todo = None
        if sess.exec.current_todo_id:
            current_todo = self._load_todo_by_id(sess, sess.exec.current_todo_id)

        # 组装输入（memory 编译/预算裁剪略写）
        step_input = {
            "session": {
                "session_id": sess.meta.id,
                "title": sess.meta.title,
                "summary": sess.meta.summary,
                "workspace_id": sess.meta.workspace_id,
            },
            "behavior": sess.exec.behavior.name,
            "step_seq": sess.exec.step_seq,
            "memory": {
                # 伪：可复用旧版 compile_context_yaml 的策略
                "session_working_memory": sess.domain.working_memory[-20:],
                "session_summary": sess.meta.summary,
                "workspace_snapshot": (sess.domain.last_workspace_snapshot.summary if sess.domain.last_workspace_snapshot else ""),
            },
            "input": {
                "new_events": [self._serialize_event(e) for e in new_events],
                "current_todo": self._serialize_todo(current_todo) if current_todo else None,
                "last_step_summary": sess.exec.last_step_summary,
            },
        }

        # 消费 inbox：本 step 已看见（标 readed）
        sess.exec.inbox = []
        return step_input

    # -----------------------------
    # ROUTER step
    # -----------------------------

    def _step_router(self, sess: Session, step_input: Dict[str, Any]) -> RouterResult:
        """resolve-router/router：快速回应 + 决定 next_behavior。

        旧版：build_router_prompt -> RouterResult
        新版：RouterResult 必须给出 next_behavior=PLAN/END
        """

        prompt = build_router_prompt_v2(step_input)
        out: RouterResult = self.llm.call_json(prompt, schema="RouterResultV2")
        return out

    def _apply_router_result(self, sess: Session, out: RouterResult):
        if out.get("immediate_reply"):
            self._send_msg(out["immediate_reply"], session_id=sess.meta.id)

        # 可选：预取 workspace snapshot
        level = out.get("workspace_need", "none")
        if level != "none":
            snap = self.workspace_store.observe(sess.meta.workspace_id, level)  # type: ignore
            sess.domain.last_workspace_snapshot = snap

        # 可选：预取 memory
        queries = out.get("memory_queries", [])
        if queries:
            retrieved = self.memory_index.search(queries, session_id=sess.meta.id, boost=1.2, limit=12)
            # 注入方式略；这里示意记录到 domain.worklog
            self.workspace_store.append_worklog(sess.meta.workspace_id, sess.meta.id, {
                "type": "MEMORY_RETRIEVAL",
                "queries": queries,
                "items": retrieved[:5],
            })

        # Router 决策下一个 behavior
        if out.get("next_behavior") == "PLAN":
            sess.exec.behavior = Behavior.PLAN
        else:
            sess.exec.behavior = Behavior.END

        sess.exec.last_step_summary = f"ROUTER: next={out.get('next_behavior')} reason={out.get('reason','')}"

        # Router 给的 tool_calls 可以缓存给后续 PLAN 使用（也可立即执行）
        sess.exec.pending_tool_calls = list(out.get("tool_calls") or [])

    # -----------------------------
    # PDCA / Self-Improve step
    # -----------------------------

    def _step_behavior(self, sess: Session, step_input: Dict[str, Any]) -> BehaviorLLMResult:
        """PLAN/DO/CHECK/ADJUST/SELF_IMPROVE 的一步。

        注意：新设计的“多 step”不是在一次 LLM 推理里循环，而是在 Main Scheduler
        中通过多次 step 逐步推进（每 step 保存状态）。
        """

        # 1) 如果有 pending tool_calls：本 step 先执行（也可异步事件化）
        if sess.exec.pending_tool_calls:
            self._execute_pending_tools(sess)

        # 2) 生成行为 prompt（process_rules/policy/toolbox 由 behavior 决定）
        prompt = build_behavior_prompt_v2(behavior=sess.exec.behavior, step_input=step_input)
        out: BehaviorLLMResult = self.llm.call_json(prompt, schema="BehaviorLLMResultV2")
        return out

    def _apply_behavior_result(self, sess: Session, out: BehaviorLLMResult):
        # 1) user reply
        if out.get("user_reply", {}).get("should_reply"):
            self._send_msg(out["user_reply"].get("message") or "", session_id=sess.meta.id)

        # 2) todo updates（写 workspace）
        if out.get("todo_updates"):
            self.workspace_store.bulk_patch_todos(sess.meta.workspace_id, sess.meta.id, out["todo_updates"])  # type: ignore

        # 3) actions
        for act in out.get("actions", []):
            self._execute_action(sess, act)

        # 4) wait
        step = out.get("step") or {}
        if step.get("is_wait"):
            sess.exec.waiting = True
            sess.exec.wait_events = [WaitEvent(**w) for w in (step.get("wait_events") or [])]
            sess.exec.last_step_summary = out.get("last_step_summary") or ""
            self._mark_not_runnable(sess.meta.id)
            return

        # 5) next behavior
        nb = (out.get("next") or {}).get("next_behavior")
        if nb:
            sess.exec.behavior = Behavior[nb]

        sess.exec.last_step_summary = out.get("last_step_summary") or ""

    # -----------------------------
    # Actions
    # -----------------------------

    def _execute_action(self, sess: Session, act: Dict[str, Any]):
        kind = act.get("kind")
        name = act.get("name")
        payload = act.get("input") or {}

        if kind == "SEND_MSG":
            self._send_msg(payload.get("message") or "", session_id=sess.meta.id)
            return

        if kind == "CALL_TOOL":
            # 记录 intent（支持崩溃恢复时识别中断点）
            self.workspace_store.append_worklog(sess.meta.workspace_id, sess.meta.id, {
                "type": "TOOL_INTENT",
                "tool": name,
                "input": payload,
            })
            result = self.tools.call(name, payload)
            self.workspace_store.append_worklog(sess.meta.workspace_id, sess.meta.id, {
                "type": "TOOL_RESULT",
                "tool": name,
                "output": result,
            })
            # tool result 作为事件进入 inbox，供下一 step 使用
            sess.exec.inbox.append(Event(
                type=EventType.TOOL_RESULT,
                session_id=sess.meta.id,
                payload={"tool": name, "result": result},
            ))
            self._mark_runnable(sess.meta.id)
            return

        if kind == "DISPATCH_SUBAGENT":
            sub_name = name
            task_id = self.subagent_pool.dispatch(
                subagent_name=sub_name,
                parent_session_id=sess.meta.id,
                workspace_id=sess.meta.workspace_id,
                payload=payload,
            )
            self.workspace_store.append_worklog(sess.meta.workspace_id, sess.meta.id, {
                "type": "DISPATCH_SUBAGENT",
                "subagent": sub_name,
                "task_id": task_id,
                "payload": payload,
            })
            # 常见做法：把相关 todo 标记 WAIT（由 todo_updates 做也行）
            return

        if kind == "UPDATE_SESSION":
            # 只允许安全 patch：title/summary/status/workspace_id
            patch = payload
            if "title" in patch and isinstance(patch["title"], str):
                sess.meta.title = patch["title"].strip() or sess.meta.title
            if "summary" in patch and isinstance(patch["summary"], str):
                sess.meta.summary = patch["summary"].strip()
            if "status" in patch and patch["status"] in ("active", "archived", "deleted"):
                sess.meta.status = patch["status"]
            self.session_store.upsert_meta(sess.meta)
            return

        if kind == "UPDATE_WORKSPACE":
            # 伪：写文件/commit/生成 artifact 等
            self.workspace_store.append_worklog(sess.meta.workspace_id, sess.meta.id, {
                "type": "WORKSPACE_UPDATE",
                "detail": payload,
            })
            return

        # NOOP / unknown
        self.workspace_store.append_worklog(sess.meta.workspace_id, sess.meta.id, {
            "type": "UNKNOWN_ACTION",
            "raw": act,
        })

    # -----------------------------
    # Tool execution (pending)
    # -----------------------------

    def _execute_pending_tools(self, sess: Session):
        calls = sess.exec.pending_tool_calls
        sess.exec.pending_tool_calls = []
        for tc in calls:
            name = tc.get("name")
            args = tc.get("args") or {}
            self._execute_action(sess, {"kind": "CALL_TOOL", "name": name, "input": args})

    # -----------------------------
    # TODO selection（DO/CHECK input 生成）
    # -----------------------------

    def _has_executable_todo(self, sess: Session) -> bool:
        if not sess.meta.workspace_id:
            return False
        todos = self.workspace_store.load_todo(sess.meta.workspace_id, sess.meta.id)
        # DO：NEW/IN_PROGRESS 且 deps 满足
        if sess.exec.behavior == Behavior.DO:
            return any(
                (t.assignee == "MAIN")
                and (
                    t.status == "IN_PROGRESS"
                    or (t.status == "NEW" and self._deps_done(t, todos))
                )
                for t in todos
            )

        # CHECK：存在 COMPLETE 或（BENCH 且 WAIT）
        if sess.exec.behavior == Behavior.CHECK:
            return any(
                (t.status == "COMPLETE")
                or (t.type == "BENCH" and t.status == "WAIT")
                for t in todos
            )

        # 其它 behavior：由 inbox 驱动；todo 是否可执行不做强约束
        return False

    def _select_current_todo(self, sess: Session) -> Optional[TodoItem]:
        todos = self.workspace_store.load_todo(sess.meta.workspace_id, sess.meta.id)

        # 优先：IN_PROGRESS 且 assignee=MAIN
        for t in todos:
            if t.assignee == "MAIN" and t.status == "IN_PROGRESS":
                return t

        # 次选：deps 满足的 NEW
        for t in todos:
            if t.assignee == "MAIN" and t.status == "NEW" and self._deps_done(t, todos):
                return t

        # CHECK：COMPLETE 的 todo
        if sess.exec.behavior == Behavior.CHECK:
            for t in todos:
                if t.status == "COMPLETE":
                    return t

        return None

    # 旧版 helper 已被 _has_executable_todo 内联；保留接口位置方便扩展
    # def _is_todo_executable(self, t: TodoItem, all_todos: List[TodoItem]) -> bool:
    #     ...

    def _deps_done(self, t: TodoItem, all_todos: List[TodoItem]) -> bool:
        done = {x.todo_id for x in all_todos if x.status == "DONE"}
        return all(dep in done for dep in (t.deps or []))

    def _load_todo_by_id(self, sess: Session, todo_id: str) -> Optional[TodoItem]:
        todos = self.workspace_store.load_todo(sess.meta.workspace_id, sess.meta.id)
        for t in todos:
            if t.todo_id == todo_id:
                return t
        return None

    # -----------------------------
    # More work?
    # -----------------------------

    def _has_more_work(self, sess: Session) -> bool:
        if sess.exec.waiting:
            return self._can_wake_session(sess)
        if sess.exec.inbox:
            return True
        if self._has_executable_todo(sess):
            return True
        return False

    # -----------------------------
    # Serialization helpers
    # -----------------------------

    def _serialize_event(self, ev: Event) -> Dict[str, Any]:
        return {
            "type": ev.type.name,
            "payload": ev.payload,
            "session_id": ev.session_id,
            "event_id": ev.event_id,
            "run_id": ev.run_id,
            "ts": ev.ts,
        }

    def _serialize_todo(self, t: Optional[TodoItem]) -> Optional[Dict[str, Any]]:
        if not t:
            return None
        return {
            "todo_id": t.todo_id,
            "title": t.title,
            "type": t.type,
            "status": t.status,
            "deps": t.deps,
            "skills": t.skills,
            "assignee": t.assignee,
            "acceptance_criteria": t.acceptance_criteria,
            "artifacts": t.artifacts,
            "blockers": t.blockers,
            "next_action": t.next_action,
        }

    # -----------------------------
    # External IO placeholders
    # -----------------------------

    def _poll_external_events(self) -> List[Event]:
        """从系统收集新的 user msg / system event / subagent done 等。
        真实实现：阻塞/回调/消息队列。
        """
        return []

    def _sleep_until_new_event(self):
        time.sleep(0.05)

    def _send_msg(self, message: str, session_id: str):
        # 真实实现：send_msg tool
        print(f"[send_msg sid={session_id}] {message}")

    def _enrich_event_ids(self, ev: Event) -> Event:
        ev.event_id = ev.event_id or new_id()
        ev.run_id = ev.run_id or new_id()
        return ev


# ============================================================
# 7) SubAgent 并行（独立 loop，通过 Event 回传主 Agent）
# ============================================================


class SubAgentPool:
    def dispatch(self, subagent_name: str, parent_session_id: str, workspace_id: str, payload: Dict[str, Any]) -> str:
        raise NotImplementedError


class EventBus:
    def publish_to_main(self, ev: Event) -> None:
        raise NotImplementedError


class SubAgentRuntime:
    """SubAgent 典型结构：
    - 自己也可以有小型 step loop
    - 最终发回 SUBAGENT_DONE 事件
    """

    def __init__(self, name: str, llm, tools, event_bus: EventBus):
        self.name = name
        self.llm = llm
        self.tools = tools
        self.event_bus = event_bus

    def run_task(self, task_id: str, parent_session_id: str, workspace_id: str, payload: Dict[str, Any]):
        # 伪：subagent 自己规划/执行/工具
        result = self.llm.call_json({"subagent": self.name, "payload": payload}, schema="SubAgentResult")

        self.event_bus.publish_to_main(Event(
            type=EventType.SUBAGENT_DONE,
            session_id=parent_session_id,
            payload={
                "task_id": task_id,
                "subagent": self.name,
                "workspace_id": workspace_id,
                "result": result,
            },
        ))


# ============================================================
# 8) Prompt 构造占位符（真实实现应加载 role/self/process_rules/policy/toolbox）
# ============================================================


def build_router_prompt_v2(step_input: Dict[str, Any]) -> Dict[str, Any]:
    return {
        "type": "router_prompt",
        "step_input": step_input,
        "output_schema": "RouterResultV2",
    }


def build_behavior_prompt_v2(behavior: Behavior, step_input: Dict[str, Any]) -> Dict[str, Any]:
    return {
        "type": "behavior_prompt",
        "behavior": behavior.name,
        "step_input": step_input,
        "output_schema": "BehaviorLLMResultV2",
    }

