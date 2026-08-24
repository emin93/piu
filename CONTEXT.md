# Local agent workspace

This glossary defines the planned development environment and keeps the graphical shell, agent behavior, and model execution separate.

## Language

**Più**:
The product's display name. Machine-facing identifiers use the ASCII spelling `piu`, including the `emin93/piu` source repository, `ch.emin.piu` macOS bundle identifier, and `piu.emin.ch` product website.
_Avoid_: Piu as the display name, Pi harness as the product name

**Agent runtime**:
The single execution system that owns sessions, tools, skills, extensions, permissions, context management, and the agent loop. Switching models must not replace this runtime.
_Avoid_: Harness manager, agent launcher, backend

**Pi transport**:
The bundled Pi process's native JSONL RPC protocol over standard input and output. Più supervises one Pi process per active chat and translates the protocol once at the host boundary. Each child starts through a small Più-owned launcher that uses Pi's public session APIs and official RPC runner so it can supply application-owned credentials without copying Pi's CLI. The desktop shell does not host the Pi SDK or place AI SDK HarnessAgent, ACP, or another agent abstraction between the application and Pi.
_Avoid_: AI SDK Pi Harness adapter, Pi SDK inside the desktop shell, sandbox bridge, copied Pi CLI, second agent protocol

**Frontend**:
The purpose-built macOS application used to open projects, chat with the agent, inspect tool calls and diffs, approve actions, and change models. Its single main window has a global chat inbox, a primary conversation view, and adjacent Diff, Files, and Terminal views, but no direct code editor.
_Avoid_: Harness, runtime

**Più runtime state**:
The application-owned Pi configuration, credentials, model routes, extensions, packages, skills, and sessions used by every chat. It never reads, modifies, imports, or synchronizes a standalone Pi installation's runtime state.
_Avoid_: Shared `~/.pi/agent`, synchronized Pi configuration

**Desktop shell**:
The Apple-Silicon-only Tauri 2 application that embeds the React frontend in the macOS system WebView. Its Rust host owns native windows, filesystem boundaries, processes, terminals, Git operations, Pi sessions, oMLX lifecycle, and application shutdown.
_Avoid_: Electron, browser server, frontend process manager

**Model route**:
A selectable model and its authentication or local endpoint. A route changes inference while leaving the agent runtime and its capabilities intact. The selector lists every route configured in Pi, not only the routes included in onboarding.
_Avoid_: Agent, harness

**Selected model route**:
The route most recently chosen by the user. A new chat starts with this route selected, and the user may switch routes during the conversation.
_Avoid_: Default model, fixed model

**Reasoning effort**:
A chat control whose available values come from the selected Pi model's effective reasoning metadata. Più shows only levels the model supports, changes the list immediately when the model changes, and never maintains a hard-coded universal effort list. It remembers the last chosen effort separately for each model. The bundled Qwen route exposes Low, Medium, and Extra High.
_Avoid_: Static effort list, unsupported effort, frontend-invented capability

**Codex subscription route**:
A model route authenticated with the user's ChatGPT/Codex subscription and executed by the agent runtime. It does not start or delegate to the Codex agent runtime.
_Avoid_: Codex harness, Codex agent

**Local inference service**:
A process on the Mac that loads an MLX model from Più's fixed application-managed model directory and exposes it to the agent runtime through a local API. The frontend starts it when the application opens, allows the model to unload after inactivity, and stops the service when the application closes.
_Avoid_: Local harness, local agent

**Project**:
A local Git repository opened in the frontend as a source of development chats. Più remembers multiple projects and permits their chats to run concurrently, but the first release does not clone or create repositories.
_Avoid_: Chat, session

**Remove project**:
An action available only when the project has no unmerged chats. It forgets the local repository without modifying or deleting it. Existing merged chat records remain in Merged history until the user deletes them individually.
_Avoid_: Delete repository, cascade-delete history, remove with active chat

**Chat inbox**:
The default stable list of every unmerged chat across opened projects, ordered newest-created-first and optionally filtered to one project. Agent activity changes indicators but never moves the row.
_Avoid_: Task inbox, project-grouped sidebar, recent-activity feed, configurable board

**Chat draft**:
The one unsent new-chat prompt retained for a project when the user navigates away. It appears above existing chats, but Più creates no chat, worktree, branch, or Pi session until the first prompt is sent.
_Avoid_: Empty chat, draft worktree

**Project setup script**:
The optional repository-owned `.piu/setup.sh` executable that prepares every new chat worktree by installing dependencies and provisioning ignored or generated files. Più runs it directly from the worktree root before starting the agent, honors its shebang, and supplies `PIU_PROJECT_ROOT` and `PIU_WORKTREE_ROOT`. It must be non-interactive, executable, and safe to retry. The application does not copy ignored files from the main checkout implicitly or expose a setup-command setting.
_Avoid_: Automatic `.env` copying, global setup script, setup-command field, auto-detected `bin/setup`

**Setup failure**:
A failed, signaled, cancelled, or unlaunchable `.piu/setup.sh` run. Più keeps the chat and worktree, shows the streamed setup log, and offers Retry and Open Terminal without starting the agent.
_Avoid_: Preparing lifecycle state, partial agent launch, automatic setup retry

**Chat**:
One agent conversation paired with its own isolated, application-managed Git worktree created from the freshly fetched `origin/main` reference. Its branch name is generated from the first prompt as `agent/<short-chat-id>-<prompt-slug>`. Multiple chats may run concurrently. A chat is either unmerged or merged; Più does not introduce a separate task lifecycle.
_Avoid_: Task, job, shared checkout, unscoped conversation

**Chat title**:
A concise title generated from the first message and editable from the chat context menu. Renaming changes presentation only and never renames the branch or worktree.
_Avoid_: Branch name as title, immutable generated title

**Chat search**:
Inbox and Merged-history search over chat titles, project names, branch names, and pull-request numbers. The first release does not index full conversation contents.
_Avoid_: Full-transcript search, semantic search

**Chat activity**:
Transient information shown on a chat row while its agent is running, needs input, finishes a turn, or fails. Activity and unread indicators help direct attention but are not persisted chat lifecycle states and never reorder the inbox.
_Avoid_: Task status, Settled, Snoozed, Pinned, approval state

**Steering message**:
A user message sent while the chat's agent is running. Più always queues it through Pi's steering behavior for delivery at the next safe point and does not expose a steer-versus-follow-up selector.
_Avoid_: Queue-mode setting, hidden heuristic, second composer

**Sent message**:
An immutable user or assistant turn already accepted by the chat. Più does not edit, regenerate, fork, or branch past conversation turns in the first release because the worktree may already contain their effects.
_Avoid_: Edit and resend, regenerate response, conversation fork

**Merge state**:
The only durable completion state for a chat: unmerged or merged, derived from its GitHub pull request. Merging automatically archives the chat into read-only Merged history and permits Più to remove its worktree and local branch. Archive is a consequence of merge, not a separate user-managed state.
_Avoid_: Done task, completed task, settlement

**Prompt attachment**:
An image or individual file added to a user turn. The selected model route must support its media type; folders are not attachments because the agent can inspect its chat worktree directly.
_Avoid_: Context picker, attached folder

**Chat notification**:
An unread indicator and macOS notification emitted when a background chat finishes a turn or needs user input. Più sends the system notification only while its main window is unfocused.
_Avoid_: Progress notification, notification preferences

**User setting**:
A persistent choice that changes user intent, such as the selected model route or whether a Più resource is enabled. Runtime paths, ports, inference tuning, storage layout, process policies, theme, and other managed defaults are not user settings.
_Avoid_: Advanced option, expert setting, exposed parameter

**Merged chat**:
A read-only, automatically archived chat record in the collapsible and searchable Merged history. It retains its conversation, pull-request link, and final metadata after Più removes its worktree and local branch. The user may permanently delete the record separately.
_Avoid_: Completed task, resumable merged chat, deleted chat

**Delete chat**:
The only way to remove an unmerged chat. After explicit confirmation, Più permanently removes its local conversation, worktree, and local branch. If its agent or terminal is active, the same confirmation explains that Più will stop those processes first. It does not close a GitHub pull request or delete a remote branch. Merged history records can also be permanently deleted.
_Avoid_: Manual archive, soft delete, remote cleanup side effect

**Stop**:
The chat-toolbar action that aborts the active Pi turn and its currently owned tool process without deleting or resetting the conversation, worktree, or terminal. The chat remains immediately available for another message.
_Avoid_: Delete chat, pause chat, reset session

**Turn failure**:
An inline end to the current agent turn that preserves every prior tool effect and leaves the composer available. Più never replays a failed turn automatically; Retry is limited to operations known to be safe, such as setup or model startup.
_Avoid_: Automatic turn replay, rollback claim

**Pull request action**:
An explicit frontend button that asks the chat's Pi agent to commit as needed, push its branch, and create a ready-for-review GitHub pull request. After the pull request exists, the same control becomes Update PR and asks the agent to commit and push later changes. It does not create a draft pull request or bypass the agent runtime with a second source-control workflow.
_Avoid_: Automatic PR, draft PR, frontend-owned PR implementation

**Diff review**:
A view where the user can attach comments to selected diff lines and send the collected comments to the chat agent as one visible review turn.
_Avoid_: GitHub review submission, direct code editing

**Onboarding**:
The in-application setup flow for ChatGPT/Codex authentication, Hugging Face access, model download, oMLX configuration, Pi configuration, and GitHub authentication. App-owned secrets live in macOS Keychain, GitHub authentication remains owned by `gh`, and the finished application must not require terminal setup.
_Avoid_: README-only setup, terminal onboarding

**Pi credential bridge**:
The narrow credential adapter used by the bundled Pi launcher. It implements Pi's public credential-store contract over macOS Keychain, serializes provider updates across concurrent chat processes, and lets Pi own login and token-refresh semantics without writing secrets to `auth.json`.
_Avoid_: Plaintext credential file, imported standalone Pi credentials, environment-only OAuth token, duplicated provider auth flow

**Git host**:
GitHub is the only pull-request host supported by the first release.
_Avoid_: Provider-agnostic forge integration

**Resource management**:
The Models & Resources settings surface that discovers and loads Pi model routes, skills, extensions, and packages, with basic enable and disable controls. MCP and a marketplace are outside the first release. The complete Settings structure is Accounts, Models & Resources, Projects, Diagnostics, and About.
_Avoid_: Separate frontend plugin system, marketplace

**Theme**:
The coherent light or dark visual appearance applied to every Più view and changed live with the macOS system appearance. Più has no theme override or theme setting.
_Avoid_: Manual theme, per-view theme, theme editor

**Chat terminal**:
A collapsible interactive terminal whose working directory is the chat worktree. Each chat owns its terminal state independently.
_Avoid_: Agent command log, global terminal

**Bundled runtime**:
The self-contained, application-managed distributions of Node, Pi, CPython, oMLX, and MLX shipped inside each Più release, together with an exact revision of the Qwen target and MTP drafter. Più accepts the larger application bundle to avoid a second runtime download or system dependency. It selects the latest stable compatible versions when developing or cutting a release, then pins the complete tested set for reproducibility. The product does not depend on arbitrary system-installed runtime versions or independently update model assets.
_Avoid_: System Pi, system oMLX, floating latest version, stale template version, unpinned runtime

**Supported Mac**:
An Apple Silicon Mac running macOS 15.0 or newer. The minimum follows the bundled stable oMLX and MLX runtime rather than a lower Tauri capability floor.
_Avoid_: Intel Mac, macOS 14 compatibility layer, cross-platform target

**Application shutdown**:
Closing the application while an agent or terminal command is active requires confirmation. Confirming stops all Pi sessions, chat terminal processes, and oMLX; idle chat histories remain resumable after the next launch.
_Avoid_: Background agent daemon, silent termination

**Crash recovery**:
Restoration of chat tabs and Pi sessions in a stopped state after an application or system failure. Più marks interrupted agent turns and terminal commands as interrupted and never replays them automatically.
_Avoid_: Automatic chat resumption, command replay

**Diagnostics**:
Local application and runtime logs that Più never uploads automatically. The user may export them explicitly for troubleshooting.
_Avoid_: Telemetry, automatic crash reporting, analytics

**Local inference failure**:
An explicit error shown when the local service or model cannot start, respond, or fit in available memory. Più offers Retry and Switch Model but never silently sends the chat to a cloud route.
_Avoid_: Automatic cloud fallback, hidden retry loop

**Source repository**:
The public MIT-licensed `emin93/piu` GitHub repository created after the design is confirmed.
_Avoid_: Private repository, organization-owned repository
