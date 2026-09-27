# AI_PROVIDER_SPEC — The provider abstraction

Normative. Implements `ADR-001`, `ADR-005`, and `ADR-006`. Target interface version: **1**.

---

## 1. Constraint

**The context engine must not know that models exist.** `aicontext-context` assembles a
`ContextPacket` and stops. Any provider-specific type appearing in a core or context signature is a
defect, and the crate dependency check in CI enforces it.

```text
ContextPacket ──▶ AIProvider ──▶ CompletionRequest
                        │
                        └─◀── CompletionResponse (text + ToolCall list + usage)
```

The host owns policy, permissions, and the tool gateway. The provider owns tokenisation, transport,
and translating its own wire format into ours. Nothing else crosses this line.

---

## 2. Interface

```rust
#[async_trait]
pub trait AIProvider: Send + Sync {
    fn id(&self) -> &ProviderId;

    async fn complete(&self, req: CompletionRequest)
        -> Result<CompletionResponse, ProviderError>;

    async fn stream(&self, req: CompletionRequest)
        -> Result<BoxStream<StreamEvent>, ProviderError>;

    fn capabilities(&self) -> Capabilities;
}
```

```rust
pub struct CompletionRequest {
    pub policy: PolicyRegion,          // Z0 + Z3 only. Host-built, never model-built.
    pub messages: Vec<Message>,        // user / assistant / tool roles
    pub data: Vec<DataBlock>,          // Z4 content, each tagged trust: untrusted
    pub tools: Vec<ToolSpec>,          // from the gateway, permission-filtered
    pub budget: TokenBudget,           // max output tokens, hard ceiling
    pub options: ModelOptions,         // temperature, seed, stop sequences
    pub idempotency_key: String,
}

pub struct CompletionResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: FinishReason,   // Stop | ToolUse | Length | Refusal | Error
    pub usage: Usage,                  // input, output, cached_input; provider-reported
    pub provider_meta: serde_json::Value, // opaque; never interpreted by the core
}
```

### 2.1 The two regions

`PolicyRegion` and `DataBlock` are structurally separate types, not a convention. A host that puts
external content into `PolicyRegion` is violating the contract, and a test asserts that the
assembler only ever populates it from Z0 policy files and the developer instruction.

`DataBlock { source, path, tier, trust: Untrusted, content }` — the trust tag is not optional and
is not settable by a caller to anything but `Untrusted`.

### 2.2 Tool calls

```rust
pub struct ToolCall {
    pub id: String,             // provider-scoped; remapped to a host id
    pub tool: String,           // "aws.ec2.stop" — provider-proposed, gateway-verified
    pub arguments: serde_json::Value, // validated against input_schema before dispatch
    pub raw: serde_json::Value, // retained for audit only
}
```

A `ToolCall` is a **proposal**, never an instruction. It is validated, permission-checked, and
approved before anything happens. A tool name the gateway does not recognise is dropped with an
audit record, not passed through.

---

## 3. Adapters

| Adapter | Phase | Notes |
|---------|-------|-------|
| `openai-compatible` | 3 | Covers OpenAI plus any `base_url`-compatible server: Ollama, llama.cpp, vLLM, LM Studio, OpenRouter, Azure OpenAI |
| `anthropic` | 3 | Maps `tool_use` and `stop_reason` |
| `gemini` | 3 | Maps `functionCall` and `finishReason` |
| `local` | future | Same wire shape as `openai-compatible`; a distinct adapter only if a non-HTTP transport appears |

**The OpenAI-compatible adapter ships first, and alone.** It reaches the hosted vendor and the local
model with one implementation, which is the cheapest possible proof that the abstraction is real
rather than theoretical. The others follow only after the contract test suite exists.

**`base_url` support is not a testing convenience** — it is how the local-model and
offline-by-proxy requirements are met (`PRD.md` C-1, §44).

---

## 4. Credentials

Per `ADR-005`. Resolution order: explicit flag → environment → OS keychain reference from the local
config. Values never appear in `.ai/`, logs, errors, or audit records. Redaction is applied at the
logging boundary, not per call site.

`.ai/ai.yaml` may name a credential *source*:

```yaml
providers:
  - id: local
    adapter: openai-compatible
    base_url: http://localhost:11434/v1
    model: qwen2.5-coder
    auth:
      type: none

  - id: hosted
    adapter: openai-compatible
    base_url: https://api.openai.com/v1
    model: <configured at connect time>
    auth:
      type: env_or_keychain
      env: AICON_AI_HOSTED_KEY      # the NAME of the variable, never its value
```

`aicontext connect` stores the reference and verifies connectivity with one minimal call.
`aicontext disconnect` removes the reference and leaves no residue.

---

## 5. Configuration

```yaml
version: 1
default_provider: local
default_model: qwen2.5-coder
budget:
  max_input_tokens: 32000
  max_output_tokens: 4000
  max_run_usd: 1.00                # stop before overspend
  degrade: true
providers: [ … ]
```

Layering, later wins: built-in defaults → `.ai/ai.yaml` (committed) → `.aicontext/config.toml`
(local, git-ignored) → environment → flags. Committing provider *preferences* is fine; committing
credentials is not.

---

## 6. Reliability

- **Timeouts:** connect, request, and total, each configurable, each producing `PROVIDER_TIMEOUT`.
- **Retries:** bounded (default 2), only for transport errors and 429/5xx, with exponential
  backoff and jitter, and only when the request is idempotent. Every retry is logged. Silent
  retrying is forbidden.
- **Rate limits:** surfaced as a typed error with the retry-after hint. The CLI does not sleep
  silently.
- **Cancellation:** Ctrl-C cancels the in-flight request, records a partial-usage record, and exits 9.
- **Degradation:** if a provider is unavailable, the run **fails with a clear message**. It does not
  silently switch provider or model, because a silent downgrade changes both cost and behaviour
  behind the user's back. A deliberate fallback must be configured, and the switch is logged.
- **Refusals:** `finish_reason: Refusal` is a first-class outcome, reported as a refusal with the
  model's text — not retried, not worked around.

---

## 7. Cost and usage

`CompletionResponse.usage` is provider-reported. Where a local server reports nothing, the estimate
is derived from the context budget and is **labelled as an estimate**. Per-run totals are printed
and optionally appended to the audit log.

Cost control is exposed, never inferred: per-provider model choice, per-task ceilings, a per-run
ceiling, and the degradation report. The platform may *suggest* a cheaper model for a simple task,
but it does not silently spend differently from what was configured.

---

## 8. Model routing (seam only)

`ProviderRegistry::route(&RoutingRequest) -> ProviderId` exists in Phase 3 and is a no-op
(returns the configured default). Later it may classify a task, select a model, apply a budget, and
fall back to a local model. The seam exists so that adding routing does not change the context
engine, the CLI contract, or the gateway. No routing logic ships in the MVP (`PRD.md` §49).

---

## 9. Structured output

Models that support it return typed actions. The host validates every action against
`action-proposal.schema.json` regardless of whether the provider enforces it.

```yaml
kind: action_proposal
version: 1
actions:
  - type: tool_call
    tool: aws.ec2.describe
    arguments: { instance_id: "i-0abc1234" }
    reason: "Confirm the instance is idle before proposing a stop"
  - type: plan
    summary: "Stop the idle instance, then update TASKS.md"
    files_modify: [".ai/TASKS.md"]
    files_create: []
    files_delete: []
    risks: []
```

A malformed or unrecognised action is rejected and recorded. A model cannot emit an action the
gateway does not recognise, and cannot widen its own scope: `files_modify` is intersected with the
task's `touches` allowlist before anything is written.

---

## 10. Autonomy

Per `ADR-006`, the provider layer has no write capability whatsoever. It returns text and action
proposals. Execution lives exclusively behind the tool gateway and the approval broker. There is no
provider SDK function that touches the filesystem, spawns a process, or makes a tool call directly.

---

## 11. Contract test suite

Every adapter must pass the same suite, run against a local fake server (no network in CI):

```text
T1  maps a text-only response, including an empty one
T2  maps a tool call and its arguments
T3  maps each finish reason, including Refusal
T4  reports usage; tolerates absent usage
T5  maps 400, 401, 403, 404, 429, 5xx, and a malformed body to distinct typed errors
T6  honours connect, request, and total timeouts
T7  retries only idempotent requests, bounded, with backoff
T8  propagates cancellation and exits 9
T9  never places DataBlock content into the policy region
T10 emits no credential in any error, log, or usage record
T11 respects max_output_tokens as a hard ceiling
T12 reports an unrecognised finish reason rather than guessing
```

An adapter that cannot pass T9 or T10 does not ship.

---

## 12. Extending

To add a provider: implement `AIProvider`, map wire types to the neutral types, and pass T1–T12.
No other crate changes. That is the whole test of whether this abstraction is honest — and if adding
a provider ever requires touching `aicontext-context`, this specification has been violated.
