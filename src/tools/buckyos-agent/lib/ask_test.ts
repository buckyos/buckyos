import {
  askCall,
  askIdempotencyKey,
  askNotifyRequest,
  askRecipient,
  askResult,
  askSpec,
  askUser,
  parseAskArgv,
} from "./ask.ts";
import type { AskCall, AskNotifyView, AskRuntime } from "./ask.ts";
import { ArgError, HelpRequested } from "./cli.ts";

function assertEquals(actual: unknown, expected: unknown): void {
  const stable = (value: unknown): unknown => {
    if (Array.isArray(value)) return value.map(stable);
    if (value !== null && typeof value === "object") {
      return Object.fromEntries(
        Object.entries(value).sort(([left], [right]) =>
          left.localeCompare(right)
        ).map(([key, entry]) => [key, stable(entry)]),
      );
    }
    return value;
  };
  if (JSON.stringify(stable(actual)) !== JSON.stringify(stable(expected))) {
    throw new Error(
      `expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
    );
  }
}

function assertThrows(
  fn: () => unknown,
  type: new (...args: never[]) => Error,
  includes: string,
): void {
  try {
    fn();
  } catch (err) {
    if (!(err instanceof type)) {
      throw new Error(`expected ${type.name}, got ${err}`);
    }
    if (!err.message.includes(includes)) {
      throw new Error(`expected ${JSON.stringify(includes)} in ${err.message}`);
    }
    return;
  }
  throw new Error(`expected ${type.name} (${includes})`);
}

function token(claims: Record<string, unknown>): string {
  const part = (value: unknown) =>
    btoa(JSON.stringify(value)).replace(/\+/g, "-").replace(/\//g, "_")
      .replace(/=+$/, "");
  return `${part({ alg: "EdDSA" })}.${part(claims)}.sig`;
}

const APP_TOKEN = token({
  iss: "verify-hub",
  sub: "alice",
  appid: "jarvis.test.buckyos.io",
  principal_kind: "app",
  token_use: "session",
  target_kind: "app",
  app_instance_id: "jarvis.test.buckyos.io@alice",
  app_owner_user_id: "alice",
});

const CALL: AskCall = { runId: "run-1", callId: "call-7", sessionId: "ui-1" };
const NOW = 1_700_000_000_000;

function view(patch: Partial<AskNotifyView>): AskNotifyView {
  return {
    notify_id: "nt-1",
    url: "https://test.buckyos.io/messagehub?notify=nt-1",
    status: "open",
    prompt: {
      title: "Which plan?",
      controls: [{
        kind: "choice",
        id: "option",
        options: [
          { value: "a", label: "Plan A" },
          { value: "b", label: "Plan B" },
        ],
        multiple: false,
        required: false,
      }, { kind: "text_input", id: "text", multiline: true, required: false }],
    },
    task_id: "t-42",
    expires_at_ms: NOW + 10_800_000,
    ...patch,
  };
}

Deno.test("ask_user argv: a text question and a choice with its options in order", () => {
  assertEquals(parseAskArgv(["What is the deadline?"]), {
    question: "What is the deadline?",
    options: [],
    allowText: false,
    expiresSecs: 10_800,
  });
  assertEquals(
    parseAskArgv([
      "--option",
      "b=Plan B",
      "Which plan?",
      "--allow-text",
      "--option",
      "a = Plan A=cheap",
      "--expires",
      "600",
    ]),
    {
      question: "Which plan?",
      options: [{ id: "b", label: "Plan B" }, {
        id: "a",
        label: "Plan A=cheap",
      }],
      allowText: true,
      expiresSecs: 600,
    },
  );
});

Deno.test("ask_user argv: refusals", () => {
  const refused: Array<[string[], string]> = [
    [[], "missing <question>"],
    [["Which", "plan?"], "one argument"],
    [["  "], "empty"],
    [["Q", "--option", "a=A"], "at least two"],
    [["Q", "--option", "a=A", "--option", "a=B"], "the id a"],
    [["Q", "--option", "a=A", "--option", "b=A"], "the label A"],
    [["Q", "--option", "a", "--option", "b=B"], "<id>=<label>"],
    [["Q", "--option", "=A", "--option", "b=B"], "<id>=<label>"],
    [["Q", "--option"], "--option requires a value"],
    [["Q", "--allow-text"], "--allow-text goes with --option"],
    [["Q", "--expires", "0"], "positive"],
    [["Q", "--expires", "1.5"], "integer"],
    [["Q", "--expires"], "requires a value"],
    [["Q", "--model", "x"], "unknown option --model"],
  ];
  for (const [argv, includes] of refused) {
    assertThrows(() => parseAskArgv(argv), ArgError, includes);
  }
  assertThrows(() => parseAskArgv(["Q", "--help"]), HelpRequested, "help");
});

Deno.test("ask_user needs the run and call of the shell tool", () => {
  assertEquals(askCall({ XLLM_CALL_ID: "call-7" }), null);
  assertEquals(askCall({ XLLM_RUN_ID: "run-1", XLLM_CALL_ID: " " }), null);
  assertEquals(askCall({ XLLM_RUN_ID: "run-1", XLLM_CALL_ID: "call-7" }), {
    runId: "run-1",
    callId: "call-7",
  });
  const call = askCall({
    XLLM_RUN_ID: "run-1",
    XLLM_CALL_ID: "call-7",
    OPENDAN_SESSION_ID: "ui-1",
  });
  assertEquals(call, CALL);
  assertEquals(askIdempotencyKey(call!), "ask:run-1:call-7");
});

Deno.test("ask_user asks the user the App session speaks for", () => {
  assertEquals(askRecipient(APP_TOKEN), "alice");
  assertEquals(
    askRecipient(token({ sub: "bob", principal_kind: "user" })),
    "bob",
  );
  const refused: Array<[string | undefined, string]> = [
    [undefined, "missing or unreadable"],
    ["not-a-jwt", "missing or unreadable"],
    [
      token({ sub: "alice", principal_kind: "app", app_owner_user_id: "bob" }),
      "app_owner_user_id bob",
    ],
    [token({ principal_kind: "app" }), "no subject"],
    [
      token({ sub: "did:bns:jarvis", principal_kind: "agent" }),
      "a agent session token",
    ],
    [token({ sub: "carol", principal_kind: "user", external: true }), "user"],
    [token({ sub: "alice" }), "non verify-hub"],
  ];
  for (const [value, includes] of refused) {
    assertThrows(() => askRecipient(value), Error, includes);
  }
});

Deno.test("ask_user builds an input notification of the call", () => {
  const text = askNotifyRequest(
    askSpec("What is the deadline?", [], false, 600),
    CALL,
    "alice",
    NOW,
  );
  assertEquals(text, {
    to_user: "alice",
    kind: "input",
    level: "timely",
    prompt: {
      title: "What is the deadline?",
      controls: [{
        kind: "text_input",
        id: "text",
        multiline: true,
        required: true,
      }],
    },
    source: { call_id: "call-7", session_id: "ui-1" },
    expires_at_ms: NOW + 600_000,
    idempotency_key: "ask:run-1:call-7",
  });

  const question = `Which plan?\nA is cheaper, B is faster. ${"x".repeat(200)}`;
  const choice = askNotifyRequest(
    askSpec(question, ["a=Plan A", "b=Plan B"], false),
    { runId: "run-1", callId: "call-8" },
    "alice",
    NOW,
  );
  assertEquals(choice.prompt, {
    title: "Which plan?",
    message: question,
    controls: [{
      kind: "choice",
      id: "option",
      options: [{ value: "a", label: "Plan A" }, {
        value: "b",
        label: "Plan B",
      }],
      multiple: false,
      required: true,
    }],
  });
  assertEquals(choice.source, { call_id: "call-8" });
  assertEquals(choice.expires_at_ms, NOW + 10_800_000);

  const long = askNotifyRequest(
    askSpec("y".repeat(130), [], false),
    CALL,
    "alice",
    NOW,
  );
  assertEquals(Array.from(long.prompt.title!).length, 120);
  assertEquals(long.prompt.message, "y".repeat(130));

  const open = askNotifyRequest(
    askSpec("Which plan?", ["a=Plan A", "b=Plan B"], true),
    CALL,
    "alice",
    NOW,
  );
  assertEquals(
    open.prompt.controls?.map((c) => (c as { required: boolean }).required),
    [false, false],
  );
});

Deno.test("ask_user is pending on the notification's task while it is open", () => {
  const { result, exitCode } = askResult(view({}));
  assertEquals(exitCode, 0);
  assertEquals(result, {
    agent_tool_protocol: "1",
    tool: "ask_user",
    status: "pending",
    pending_reason: "user_input",
    task_id: "t-42",
    title: "ask_user => waiting",
    summary: "waiting for the user's answer",
    detail: {
      notify_id: "nt-1",
      task_id: "t-42",
      status: "open",
      question: "Which plan?",
      options: [{ id: "a", label: "Plan A" }, { id: "b", label: "Plan B" }],
      expires_at_ms: NOW + 10_800_000,
      url: "https://test.buckyos.io/messagehub?notify=nt-1",
    },
  });
  const noTask = askResult(view({ task_id: undefined }));
  assertEquals([noTask.result.status, noTask.result.title, noTask.exitCode], [
    "error",
    "ask_user => no_task",
    2,
  ]);
});

Deno.test("ask_user returns the answer of a closed question", () => {
  const answered = (values: Record<string, unknown>) =>
    askResult(view({
      status: "answered",
      result: {
        action: "answer",
        values,
        submitted_by: "alice",
        submitted_at: NOW + 5,
      },
    }));
  const option = answered({ option: "b" });
  assertEquals(option.exitCode, 0);
  assertEquals(option.result.status, "success");
  assertEquals(option.result.return_code, 0);
  assertEquals(option.result.output, "Plan B");
  assertEquals(option.result.summary, "the user answered: Plan B");
  assertEquals((option.result.detail as Record<string, unknown>).answer, {
    option: "b",
    label: "Plan B",
    text: null,
    submitted_by: "alice",
    submitted_at: NOW + 5,
  });
  assertEquals(answered({ text: "Neither" }).result.output, "Neither");
  assertEquals(
    answered({ option: "a", text: "but after Friday" }).result.output,
    "Plan A\nbut after Friday",
  );

  const closed: Array<[Partial<AskNotifyView>, string, number]> = [
    [{ status: "dismissed" }, "ask_user => dismissed", 2],
    [{ status: "expired" }, "ask_user => expired", 4],
    [
      { status: "canceled", closed_reason: "superseded" },
      "ask_user => canceled",
      2,
    ],
  ];
  for (const [patch, title, exitCode] of closed) {
    const outcome = askResult(view(patch));
    assertEquals(
      [outcome.result.status, outcome.result.title, outcome.exitCode],
      ["error", title, exitCode],
    );
  }
  assertEquals(
    askResult(view({ status: "canceled", closed_reason: "superseded" })).result
      .summary,
    "the question was canceled (superseded)",
  );
});

function runtimeWith(
  notifySend: (req: unknown) => Promise<unknown>,
  sessionToken = APP_TOKEN,
): AskRuntime {
  return {
    buckyos: { getMsgCenterClient: () => ({ notifySend }) },
    sessionToken,
  } as unknown as AskRuntime;
}

Deno.test("ask_user sends once per call and finds the question again on a re-run", async () => {
  const sent: unknown[] = [];
  const views = [
    view({}),
    view({
      status: "answered",
      result: { action: "answer", values: { option: "a" } },
    }),
  ];
  const runtime = runtimeWith((req) => {
    sent.push(req);
    return Promise.resolve(views[sent.length - 1]);
  });
  const spec = askSpec("Which plan?", ["a=Plan A", "b=Plan B"], true);

  const first = await askUser(runtime, spec, CALL, NOW);
  assertEquals([first.result.status, first.result.task_id], [
    "pending",
    "t-42",
  ]);
  const again = await askUser(runtime, spec, CALL, NOW + 60_000);
  assertEquals([again.result.status, again.result.output], [
    "success",
    "Plan A",
  ]);
  assertEquals(
    sent.map((req) => (req as { idempotency_key: string }).idempotency_key),
    ["ask:run-1:call-7", "ask:run-1:call-7"],
  );
  assertEquals((sent[0] as { to_user: string }).to_user, "alice");
});

Deno.test("ask_user reports a refused send and a session without a user", async () => {
  const refused = await askUser(
    runtimeWith(() => Promise.reject(new Error("NoPermission: denied"))),
    askSpec("Q?", [], false),
    CALL,
    NOW,
  );
  assertEquals(refused.exitCode, 2);
  assertEquals(refused.result.title, "ask_user => notify_failed");
  assertEquals(refused.result.detail, {
    method: "notify.send",
    to_user: "alice",
    idempotency_key: "ask:run-1:call-7",
    error: "NoPermission: denied",
  });

  let called = false;
  const agentToken = token({ sub: "did:bns:jarvis", principal_kind: "agent" });
  const noUser = await askUser(
    runtimeWith(() => {
      called = true;
      return Promise.resolve(view({}));
    }, agentToken),
    askSpec("Q?", [], false),
    CALL,
    NOW,
  );
  assertEquals([noUser.result.title, noUser.exitCode, called], [
    "ask_user => no_recipient",
    2,
    false,
  ]);
});
