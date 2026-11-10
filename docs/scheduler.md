# Scheduler

Agents and workflows run on a schedule: once, every N minutes or hours, daily or weekly at a time,
or on a cron expression. You manage schedules on a Schedules screen, and an agent can manage its
own through a `schedule` tool, with your approval for anything it creates or changes.

Status: agreed 2026-11-10, not built.

## What it does

**A schedule** has a name, an agent, a target, a timing, and an on/off switch.

- Target: a **prompt** (text only) sent to the agent, or one of the agent's **saved workflows** with
  its inputs.
- Timing, in local time:
  - **Once** at a date and time.
  - **Every N minutes or hours**, on a fixed grid from its start (`start + k·N`). Late or skipped
    fires do not shift later ones.
  - **Daily or weekly** at HH:MM on chosen weekdays. The form turns this into a cron string.
  - **Cron**: a 5-field expression.

**A fire** is one occurrence.

- A prompt fire opens a **new chat** with the agent, titled `<schedule> · <date time>`, posts the
  prompt, and runs like any chat until the agent stops (tools, commands and delegations included).
- A workflow fire runs the workflow (a normal workflow run, `started_by = "schedule:<name>"`) and
  opens a short chat that says how it ended and points to the run on the Workflows screen.
- If the agent calls `ask_user_question` during a fire, it is answered at once with the existing
  `SKIPPED` text, and the agent carries on.
- When a fire ends, a desktop notification says whether it succeeded or failed. Clicking it does
  nothing in v1.

**Runtime rules**

- Schedules run only while the app is open. There is no background daemon.
- At launch, a schedule that missed one or more occurrences fires **once**, whatever their age,
  then returns to its normal timing.
- A schedule that comes due while its previous fire is still going **skips** that fire and records
  "skipped: still running".
- A fire that was running when the app closed is marked failed ("the app closed") at the next
  launch.
- A fire whose agent's model is not ready (no key, provider gone) fails at once with the reason.
- A "once" schedule turns itself off after it fires.
- One switch in Settings ("Run schedules") stops every schedule. On by default.
- DST: a time skipped by the clock change fires at the next valid minute; a repeated hour fires
  once.

**The `schedule` tool**

- Actions: `list`, `create`, `update`, `pause`, `resume`, `delete`.
- An agent sees and changes only its own schedules, and can target only itself or its own saved
  workflows.
- `create` and `update` do not take effect: they make a **draft card** in the chat (Approve, Edit,
  Reject). The tool answers "waiting for the person's approval". `delete`, `pause` and `resume`
  apply at once.
- An agent running in a fire may call it too; the draft waits in that fire's chat.

**The Schedules screen** (new rail entry)

- List: name, agent, timing in words, next fire, last result.
- Form: name, agent, prompt or workflow (and inputs), timing, on/off. Run now, delete.
- History: the schedule's recent fires, each opening its chat.

**Lifecycle**

- Deleting the agent or the workflow deletes the schedule.
- The newest **20 fires** per schedule are kept. At launch, older fires are deleted with their chats,
  except chats you wrote in after the scheduled prompt, which stay as ordinary chats.

**Not in v1:** running with the window closed, notification click actions, attachments in
scheduled prompts, one agent scheduling another, per-schedule catch-up or retention settings.

## Decisions

### ADR-1: Run schedules inside the app, catch up once at launch

**Context.** Fires need the app's state: providers and keys, the agent's tools, MCP servers, the
chat UI. The app has no daemon, tray or autostart today.

**Decision.** One scheduler thread in the app wakes every 30 seconds and asks the UI thread to fire
what is due (the same `AppHandle::update` route `update::every_so_often` and `delegate::beat` use).
Each schedule stores its next fire time (`next_at`); "due" is `next_at <= now`. After a fire,
`next_at` becomes the first occurrence after now, which gives catch-up-once for free.

**Alternatives.** A headless `--scheduler` mode under a systemd user timer: fires with the window
closed, but needs a second process sharing the database and keyring, its own provider and MCP
setup, and a way to show results. Much more to build and run; not needed now.

**Consequences.** Nothing fires while the app is closed; a fire can be up to 30 s late. Suspend
and resume need no special handling, since the check compares against the wall clock.

### ADR-2: Use `croner` for cron and daily/weekly timing

**Context.** Cron, daily and weekly timing need a parser and a "next occurrence after t" search
that is correct across DST changes.

**Decision.** Add `croner` (v4, chrono-based, 5- and 6-field, DST-aware). Daily and weekly are
stored as cron strings; the form recognizes the `M H * * D` shape to show them as presets again.
"Once" and "every N" are computed in our code (they do not fit cron).

**Alternatives.** A hand-written parser: no dependency, but about 200 lines plus tests, and DST
edge cases would be ours to get right.

**Consequences.** One dependency on chrono, which the app already uses. It builds its parser with
`derive_builder`, which adds the `darling` proc-macro crates to the build (compile time only). DST
handling is croner's and tested there; our tests pin the zone with a fixed offset.

### ADR-3: Agent-made schedules wait for approval as draft cards

**Context.** A schedule runs the agent's tools unattended and repeatedly; the shell and file tools
already run without asking. Creating or widening that must be your decision.

**Decision.** The tool's `create` and `update` store a draft (`schedule_drafts`, keyed by the tool
call) and show a card in the chat, as `teach_workflow` does for workflow drafts. Approve saves it,
Edit opens it in the Schedules form, Reject drops it. `delete`, `pause` and `resume` only reduce
what runs and apply at once. The tool checks ownership: an agent can name only its own schedules
and workflows.

**Consequences.** An agent cannot start unattended work without you seeing it first. The agent
does not learn the outcome of its draft in that turn; `list` shows it later.

### ADR-4: Notify with `notify-send`

**Context.** Rune has no notification API. `zbus` is in the tree (through Rune and the keyring),
but only with async APIs on a tokio runtime.

**Decision.** Run `notify-send` (libnotify, present on Omarchy and most desktops) as a child
process when a fire ends. If it is missing, log once and carry on.

**Alternatives.** Calling `org.freedesktop.Notifications` through `zbus`: no external binary, but
an async runtime and more code for the same result. Worth it only when notifications need click
actions, which v1 leaves out.

**Consequences.** No new dependency; no notification on a desktop without `notify-send`.

### ADR-5: Keep the newest 20 fires per schedule

**Context.** A schedule every 15 minutes makes about 100 chats a day.

**Decision.** At launch, for each schedule, delete fires past the newest 20 and their chats,
unless you posted in the chat after the scheduled prompt (it then stays as an ordinary chat). A
fixed number, not a setting.

**Consequences.** Older results are gone unless you engaged with them. Runs and workflow runs keep
their own 30-day rule.

## Data

`db/migrations/010_schedules.sql` has three tables:

- `schedules`: agent, target (`workflow_id` NULL means a prompt; a `CHECK` makes it exactly one of
  the two), `timing` as JSON, `enabled`, `next_at` (NULL once a "once" schedule has fired). Goes
  with its agent and its workflow (`ON DELETE CASCADE`).
- `schedule_fires`: due, start and end times, status (`running`, `succeeded`, `failed`,
  `skipped`), a note, and the chat and workflow run it made (`ON DELETE SET NULL`).
- `schedule_drafts`: proposals by tool call, with the chat they wait in and the whole schedule as
  JSON; `schedule_id` is set for an update.

The "Run schedules" switch is a row in the existing settings table.

## Code

| Where | What |
| --- | --- |
| `src/schedule/mod.rs` | `Schedule`, `Target` (`Prompt(String)` / `Workflow { id, inputs }`), `Fire`, `FireStatus`. Validation in one fallible constructor. |
| `src/schedule/timing.rs` | `Timing` (`Once`, `Every`, `Cron`), `next_after(t)`, parsing and describing in words, the daily/weekly preset ↔ cron mapping. Pure; generic over the time zone so tests pin one. |
| `src/store/schedules.rs`, `db/queries/schedules.sql` | Typed queries; covered by the existing prepare-every-statement test. |
| `src/schedules.rs` | The app side: the 30 s ticker, `fire_due`, starting prompt and workflow fires, overlap and catch-up, ending a fire, auto-skipping questions, launch cleanup and retention, notifications, the global switch, draft approve/reject. |
| `src/tools.rs` | The `schedule` tool spec. `run_schedule_calls(sid)` beside `run_teach_calls` in `settle`. |
| `src/run.rs` | One hook, `fire_settled`, at the end of `settle` and of a `resolve` that does not carry on: it answers a pending question with `SKIPPED` in a fire's chat, and ends the fire when its chat is no longer busy. |
| `src/ui/schedules.rs`, `src/ui/rail.rs`, `src/ui/chat.rs`, `src/ui/settings.rs` | The screen, its rail entry, the draft card, the switch. `Route::Schedules`. |
| `src/demo.rs` | A `schedules` scene for `--snap`. |

**Ending a prompt fire.** A fire's chat is one chain of runs: reply, tool calls, results, reply.
At the end of `settle`, once the chat is not `busy_in` (no reply, command or delegation pending),
the fire ends: succeeded if the last run was OK, failed with its error otherwise, failed "stopped"
if you stopped it. In-memory, `App.firing: HashMap<schedule_id, fire_id>` drives the overlap check,
and the fire's chat is found through `schedule_fires.session_id`.

## Build order

Each step builds, passes `cargo clippy --all-targets -- -D warnings` and `cargo test`, and leaves
the app working.

1. **Timing.** Add `croner`. `Timing` with `next_after`, parse and describe. Tests: grid without
   drift, once in the past and future, cron weekdays in a given zone, preset round-trip. Done.
2. **Store.** Migration 010, queries, `store/schedules.rs`. Tests: round-trip, cascade on agent and
   workflow delete, the target `CHECK`. Done.
3. **Runner.** Ticker, prompt fires, workflow fires, catch-up once, overlap skip, stale-running
   cleanup, auto-skip, notifications, global switch. End-to-end tests with the existing mocked
   provider streams (as in `run.rs` tests): a due prompt fire makes a chat and ends succeeded; a
   question is skipped; a second due fire while running is skipped; a missed weekly fires once.
   Done. A chat can also stop without a `settle` (a stopped command or delegation resolves its
   call with `carry_on = false`), so `resolve` runs the same end-of-fire check in that case.
4. **Tool.** Spec, ownership checks, drafts, approve and reject. Tests: create makes a draft and
   nothing runs; approve saves it with `next_at`; another agent's id is refused; delete applies at
   once. Done: argument parsing in `src/schedule/tool.rs`, handling in `src/schedules.rs`.
5. **UI.** Schedules screen, draft card, Settings switch, demo scene. Check with
   `cargo run -- --snap out.png --scene schedules` (also `schedule-form`, `schedule-draft`). Done:
   `src/ui/schedules.rs`, the form's state and checks in `src/schedules/form.rs`.
6. **Retention.** Launch tidy. Test: 25 fires leave 20, a chat you wrote in survives. Done.
7. **README.** Add the scheduler to "Where things live" and "Layout".

## Risks

| Risk | Mitigation |
| --- | --- |
| An approved schedule runs shell commands unattended, often. | Approval for create/update; the card shows the timing in words and the next three fire times; the global switch stops everything at once. |
| A failing schedule fires every minute and spams notifications and chats. | Retention caps chats at 20; overlap skip stops pile-ups. If this bites, add auto-pause after N failures in a row (not in v1). |
| Fires cost provider tokens without you watching. | Each fire is a normal run on the Runs screen with its tokens. |
| Detecting the end of a prompt fire misses a path (a new tool kind that runs async). | One check at the end of `settle`, keyed on `busy_in`, which every async tool already feeds. Covered by the end-to-end tests. |
| A long sleep or a clock jump makes many occurrences due at once. | Catch-up fires once and recomputes `next_at` from now. |
