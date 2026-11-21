# Hooks

Hooks are commands you set to run on this computer around an agent's tool calls. They are for
your own rules ("no deploys on Friday", "log every shell command", "shrink what this MCP tool
returns"). Set them on the **Hooks** screen; only you can: a model or a channel person cannot add,
change or switch one.

## Order with the guardrails

The guardrails (see `guardrails.md`) look at a call first. A hook only sees a call they let
through, and it can refuse it, hold it for your go-ahead, or change it. A hook never allows what a
guardrail stopped. A call a guardrail held for your go-ahead runs its hooks once you allow it.

## Events

| Event | When | A hook can |
|---|---|---|
| `before_tool` | after the guardrails, before the tool runs | refuse, ask, replace the arguments |
| `after_tool` | when the call has its result, before the model sees it | withhold, replace the result |
| `before_agent` | a new message starts the agent's work | watch |
| `before_model` | before each call to the model | watch |
| `after_model` | the model's reply has arrived | watch |
| `on_error` | a reply failed | watch |
| `after_agent` | nothing more is running for the agent | watch |

The tool events can change what happens. The other five only watch: the hook is told, nothing waits
for it, and what it prints is ignored (a non-zero exit is noted in the run as a failure). Their
input has the common fields plus: `message` (`before_agent`); `model`, `system`, `turns`, `tools`
(`before_model`); `text`, `tool_calls`, `output_tokens` (`after_model`); `error` (`on_error`);
`status` (`done`, `stopped` or `failed`), `reply`, `error` (`after_agent`). The model events carry
what the model sees, with secrets hidden as placeholders; tool events carry real values.

`after_tool` sees every result a call gets, including its errors. It does not see a call that was
refused, or one you stopped.

On the tool events a hook is for the tools its **Tools** field names: names split by `|`, `*` for any characters
(`shell`, `write_file|edit_file`, `mcp__*`); empty means every tool. Hooks for one event run in
the order on the screen, each given the arguments or result the one before left.

## What a hook is given and answers

The event is JSON on the command's input, and the command runs with `sh -c` in the agent's folder:

```json
{"event": "before_tool", "chat": "…", "agent": "Scout", "cwd": "/home/me/project",
 "tool": "shell", "args": {"command": "ls"}}
```

`after_tool` adds `"result": "…"`. Hooks see real values (secrets are hidden only from models).

It answers with its exit code and what it prints:

- exit 0 and nothing printed: go on.
- exit 2: refuse; stderr is the reason, told to the model.
- exit 0 and JSON: `{"action": "allow" | "block" | "ask" | "replace", "reason": "…"}`.
  `ask` (before a tool only) holds the call on a card for your go-ahead. `replace` carries
  `"args": {…}` before a tool or `"result": "…"` after one; replaced arguments must be JSON.

Anything else counts as a **failure**: it did not start, ran past its time limit (5–60 s), exited
with another code, or printed something that is not an answer. Each hook says what a failure does:
**Go on** (default; the failure is noted in the run) or **Stop it** (the call is refused, or its
result withheld).

## In the run

A refusal, a hold, a changed argument or result, and a failure are each a step on the Runs timeline.
While hooks run, the chat shows as busy; Stop ends the wait and the calls still waiting do not run.

## Per agent

The **Agents** section of the Hooks screen has a switch for each agent (team members too): one switched
off runs no hooks.

## Not yet

Hooks run in a chat's own work. Team members' tool calls and workflow steps run none yet, and
a hook cannot yet stop or change a model call.
