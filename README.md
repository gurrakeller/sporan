# Speran (Esperanto for Agents)

> **A safe, token-efficient, sandboxed programming language designed for LLM agents.**

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Build Status](https://img.shields.io/badge/Tests-10%20passed-brightgreen.svg)]()

---

## Why Speran?

Today, when an LLM agent wants to execute a multi-step workflow, it has two flawed options:

1. **Classic Tool-Calling (One-by-one):**
   The model emits one tool call, waits for the host, receives output, and repeats. A 10-item workflow takes 10 full round-trips, multiplies token costs, and adds seconds of latency per step.
2. **Arbitrary Python/Bash Execution:**
   The model writes Python scripts. While expressive, it creates severe sandbox escape risks, requires heavy containerization, and easily enters infinite loops or hangs.

**Speran** solves both:
- **Write Once, Execute in One Turn:** The agent writes loops, conditionals, parallel maps (`par`), and tool invocations in a single script.
- **Sandboxed by Default:** No unrestricted OS, disk, or network access. Code can **only** touch tools explicitly declared with `use` and granted by the host.
- **Bounded Resource Limits:** Every run is enforced with fuel limits (`max_steps`), maximum tool calls, and timeouts. Infinite loops are physically impossible.
- **Self-Correcting LLM Diagnostics:** Errors are structured with precise spans, syntax translations, and `did_you_mean` suggestions so models fix mistakes on the first retry.

---

## A Taste of Speran

```python
# Declare the capabilities this program requires
use github.search_issues, github.get_issue, slack.post

# 1. Search issues
issues = github.search_issues(repo: "acme/api", label: "bug")
emit "Discovered {len(issues)} open issues"

# 2. Fetch issue details concurrently with `par`
details = par issue in issues {
  github.get_issue(id: issue.id)
}

# 3. Filter using familiar comprehensions
critical = [d for d in details if d.severity == "critical" or d.severity == "high"]

if len(critical) == 0 {
  return {status: "clean", message: "No critical bugs found"}
}

# 4. Safe failure handling with `try`
report = try slack.post(
  channel: "#alerts",
  text: "Found {len(critical)} critical issues: {join([c.title for c in critical], \", \")}"
)

if report.error {
  return {status: "partial_failure", error: report.error.message}
}

return {
  status: "success",
  count: len(critical),
  issues: [c.title for c in critical]
}
```

---

## Defining Methods & Functions

In Speran, functions and procedures use the `fn` keyword with braces `{}`:

```python
# Defining a reusable method / function
fn triage_ticket(ticket, urgent_threshold) {
  severity = get(ticket, "severity", "low")
  comment_count = get(ticket, "comments", 0)

  if severity == "critical" or comment_count >= urgent_threshold {
    return {
      id: ticket.id,
      priority: "P0",
      action: "escalate",
      assignee: "oncall"
    }
  }

  return {
    id: ticket.id,
    priority: "P2",
    action: "backlog",
    assignee: null
  }
}

# Calling methods
issue = {id: "BUG-404", severity: "critical", comments: 12}
result = triage_ticket(issue, 5)
```

### Why No Object Methods (`obj.method()`)?
To prevent hallucinations and API confusion for LLMs, **everything is a pure function** with exactly one way to call it:

| Python / JavaScript Method | Speran Idiom | Rationale |
|---|---|---|
| `text.upper()` | `upper(text)` | Consistent top-level function |
| `items.append(x)` | `items = items + [x]` | Values are immutable JSON |
| `data.keys()` | `keys(data)` | Uniform across all maps |
| `", ".join(tags)` | `join(tags, ", ")` | Natural argument order |
| `items.sort(key=...)` | `sort(items, by: "score")` | Pure function returns new sorted list |
| `dict.get("k", default)` | `get(dict, "k", default)` | Safe null-coalescing lookup |

---

## Key Features

### 1. Capability Security
Tools are namespaces (e.g. `github.search_issues`). Programs declare them upfront:
```python
use github.search_issues, slack.post
```
The host matches these against its capability policy (e.g. `["github.*", "slack.post"]`). Any undeclared tool call or unauthorized capability is rejected **statically before the program starts**.

### 2. LLM-Friendly Diagnostics
When models hallucinate syntax from Python or JS, Speran guides them immediately:
```
Error [SyntaxError]: Method calls like 'items.append()' are not supported
  --> line 2, col 7
   |
   | items.append(4)
   |
  Hint: Values are immutable. Use: items = items + [item]
```
```
Error [UnknownName]: Variable 'countr' is not defined
  --> line 3, col 12
   |
   | return countr + 1
   |
  Hint: Did you mean 'counter'?
```

### 3. Full Tracing & Replay
Running with `--json` outputs a machine-readable report containing every tool call, input argument, execution duration, and step count:
```json
{
  "ok": true,
  "value": { "status": "success", "count": 2 },
  "emits": ["Discovered 3 open issues"],
  "trace": [
    {
      "step": 5,
      "tool": "github.search_issues",
      "args": { "repo": "acme/api", "label": "bug" },
      "ok": true,
      "duration_ms": 12
    }
  ],
  "usage": {
    "steps": 81,
    "tool_calls": 5
  }
}
```

---

## Installation & CLI Usage

### Build from source
```bash
cargo build --release
```

### Run an interactive REPL
```bash
cargo run -- --repl
```

### Run a file with mock tools
```bash
cargo run -- examples/02_agent_workflow.spe --tools examples/mock_tools.json --json
```

### Command Line Options
```text
OPTIONS:
    -e, --eval <code>         Execute code string directly
    --check                   Run parser and static capability checker without executing
    --tools <file.json>       Load mock tool definitions from a JSON file
    --grants <list>           Comma-separated capabilities to grant (e.g. github.*,slack.post)
    --json                    Output structured JSON report (value, trace, usage, error)
    --max-steps <N>           Maximum execution steps allowed (default: 100000)
    --max-tools <N>           Maximum tool calls allowed (default: 50)
    --timeout <ms>            Timeout in milliseconds (default: 30000)
    --repl                    Start interactive REPL session
```

---

## Running Tests

```bash
cargo test
```

All 10 integration tests cover:
- Parser & lexical analyzer
- Function recursion and stack frames
- List comprehensions and string interpolations
- LLM method-to-builtin hint translations
- Typo suggestions (`did_you_mean`)
- Fuel limits preventing infinite loops
- Capability-based authorization
- Resilience under `try` expressions
- Parallel evaluation and trace generation

---

## License

MIT © Gustav Keller
