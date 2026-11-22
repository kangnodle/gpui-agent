# AGENTS.md

## Role
You are an expert software engineer working on this codebase. Follow every rule in this file strictly. Prefer correctness, clarity, and architectural integrity over speed.

## Core Principles (Non-Negotiable)

1. **Pure Code vs Side-Effect Code Separation**
   - Keep pure functions and pure domain logic completely free of side effects.
   - Isolate all side effects (I/O, network, database, filesystem, logging, time, randomness, external services) behind clear boundaries (adapters, gateways, repositories, or infrastructure layers).
   - Domain and application layers must never depend on concrete side-effect implementations.

2. **Clean Architecture**
   - Strictly follow Clean Architecture (or Hexagonal / Ports & Adapters).
   - Dependency rule: dependencies point inward only.
     - Domain/Entities → Application/Use Cases → Adapters/Interfaces → Infrastructure/Frameworks.
   - Domain layer has zero dependencies on frameworks, libraries, or external systems.
   - High cohesion, low coupling is mandatory.
   - Prefer small, focused modules with a single responsibility.

3. **Monorepo / Workspace Structure**
   - Organize the repository as a proper monorepo (workspace).
   - Clear separation of packages/apps:
     - `packages/` or `libs/` for shared domain, application, and utility code.
     - `apps/` or `services/` for deployable applications and entry points.
   - Shared code lives in workspace packages and is imported via package names, never via relative paths that cross package boundaries.
   - Keep package boundaries explicit and enforced.

4. **No Comments in Source Code**
   - Never write comments in source code (no `//`, `/* */`, `#`, docstrings for explanation, etc.).
   - Code must be self-documenting through clear naming, small functions, and proper structure.
   - The only allowed annotations are those required by the language or tools (e.g., type annotations, decorators that affect runtime behavior, lint/type-checker directives when strictly necessary).

5. **Idiomatic Code**
   - Write code using the most idiomatic, conventional style of the target programming language and ecosystem.
   - Prefer standard library and well-established patterns of the language over clever or unusual constructs.
   - Follow community style guides and conventions of the language.

## Architecture Rules

- Domain logic must be pure and testable in isolation.
- Use Cases / Application Services orchestrate pure domain logic and call ports (interfaces).
- Concrete implementations of ports live only in the infrastructure / adapters layer.
- Do not leak infrastructure concerns (ORM entities, HTTP request objects, framework types, etc.) into domain or application layers.
- Prefer composition over inheritance.
- Favor explicit interfaces / protocols / traits over concrete classes when defining boundaries.
- Keep modules highly cohesive: everything in a module should change for the same reason.
- Minimize coupling: modules should depend on abstractions, not on concrete implementations from other modules.

## Code Style & Quality

- Names must be intention-revealing. Avoid abbreviations unless they are universally understood in the domain.
- Functions and methods should be small and do one thing.
- Prefer immutable data where practical.
- Avoid global mutable state.
- Error handling must be explicit and idiomatic for the language (Result/Either types, exceptions with clear contracts, etc.).
- Do not suppress or ignore errors silently.
- Write the simplest code that correctly solves the problem while respecting the architecture.

## Testing Expectations

- Pure domain logic must have thorough unit tests.
- Side-effectful code should be tested via integration tests or by substituting test doubles at the port boundary.
- Prefer testing behavior over implementation details.
- Tests should be deterministic and independent.

## Working Rules for Agents

- Before making changes, understand the existing package structure and dependency directions.
- Record new or changed requirements and design rules in `docs/requirements/*.sdoc` (StrictDoc) before implementing them: give each a UID, link the files that verify it with `File` relations, and run `mise run docs`.
- When adding new functionality:
  1. Place pure domain logic in the appropriate domain package.
  2. Define ports (interfaces) in the application or domain layer.
  3. Implement adapters only in the infrastructure layer.
- Never introduce circular dependencies between packages.
- Never put side effects inside pure domain modules.
- Never add comments to source files.
- Prefer extending existing clean patterns over introducing new architectural styles.
- If a change would violate Clean Architecture or pure/side-effect separation, stop and restructure instead of adding technical debt.

## Forbidden Practices

- Comments in source code (except language-required annotations).
- Side effects inside domain or pure application logic.
- Domain or application layers depending on infrastructure.
- Cross-package relative imports that break workspace boundaries.
- God classes / modules with low cohesion.
- Tight coupling between unrelated concerns.
- Non-idiomatic code just because it is shorter or “clever”.

## Definition of Done

A change is complete only when:
- Pure and side-effect code remain properly separated.
- Clean Architecture boundaries are respected.
- Code is idiomatic for the language.
- No comments were added to source files.
- High cohesion and low coupling are preserved or improved.
- The change can be understood without reading external documentation or comments.

Follow these rules on every edit. When in doubt, choose the option that better preserves purity, architectural boundaries, and idiomatic style.


## Software Quality Guardrails

This file instructs AI coding agents to preserve the eight ISO/IEC 25010 product quality characteristics during continuous, incremental development. Every change must leave the system no worse, and preferably better, on these attributes.

Treat these rules as mandatory. Prefer small, verifiable increments over large changes.

### Core Operating Principles

- Make the smallest change that advances the requested goal.
- Before editing, identify which quality attributes the change can affect.
- After every meaningful change, re-verify the affected attributes.
- Prefer adding or strengthening tests over relying on manual inspection.
- Do not introduce new technical debt that is not explicitly acknowledged and justified.
- When a rule conflicts with a short-term request, stop and surface the conflict.

### 1. Functional Suitability

Goal: Functions correctly and completely satisfy stated and implied needs.

Rules:
- Implement only the requested behaviour; do not invent requirements.
- Preserve existing behaviour unless the task explicitly changes it.
- Prefer pure functions and clear input/output contracts where practical.
- When fixing a bug, add or update a test that would have caught the original failure.
- Never weaken existing assertions or remove tests to make a change pass.

Verification:
- Relevant unit and integration tests pass.
- Behaviour matches the acceptance criteria of the current task.

### 2. Performance Efficiency

Goal: Reasonable use of time and resources under expected conditions.

Rules:
- Avoid introducing obvious algorithmic complexity regressions (e.g., nested loops over large collections without need).
- Do not add unbounded memory growth, blocking calls on critical paths, or unnecessary repeated work.
- Prefer existing efficient primitives and libraries over hand-rolled solutions unless justified.
- When changing hot paths, note the expected performance impact.

Verification:
- No new obvious performance anti-patterns introduced.
- Existing performance-related tests or benchmarks (if any) still pass.

### 3. Compatibility

Goal: Works correctly with other components, systems, and environments it is expected to interact with.

Rules:
- Preserve public interfaces and data contracts unless the task explicitly requires a breaking change.
- Prefer additive changes (new optional fields, new endpoints) over breaking changes.
- When a breaking change is necessary, document the migration path and update all known callers within the same change set if feasible.
- Do not introduce version or environment assumptions that conflict with existing deployment targets.

Verification:
- Existing integration and contract tests still pass.
- Call sites of modified interfaces are updated or remain compatible.

### 4. Usability

Goal: Users can understand, learn, and operate the system effectively.

Rules:
- Keep user-facing messages clear, consistent, and free of internal jargon.
- Preserve existing interaction patterns unless the task improves them.
- Prefer progressive disclosure and sensible defaults over forcing extra steps.
- Error messages should indicate what went wrong and, when possible, how to recover.
- Accessibility considerations already present in the codebase must not be degraded.

Verification:
- User-facing strings and flows remain coherent.
- No new unexplained error states or dead ends introduced.

### 5. Reliability

Goal: Continues to perform specified functions under stated conditions for a reasonable period.

Rules:
- Prefer fail-safe behaviour: validate inputs, handle expected failure modes, and avoid cascading failures.
- Do not swallow errors silently; log or surface them appropriately.
- Preserve existing retry, timeout, circuit-breaker, or recovery logic unless the task improves it.
- Avoid introducing single points of failure or resource leaks.
- When adding concurrent or asynchronous code, ensure proper cancellation and cleanup.

Verification:
- Existing reliability and error-handling tests still pass.
- New failure modes are either handled or explicitly documented as out of scope.

### 6. Security

Goal: Protects information and resources against unauthorised access, modification, or disclosure.

Rules:
- Never hard-code secrets, credentials, or sensitive data.
- Validate and sanitise all external input.
- Prefer the principle of least privilege for any new permissions or data access.
- Do not weaken existing authentication, authorisation, or encryption mechanisms.
- Avoid introducing new attack surface (e.g., dynamic code evaluation, overly permissive CORS, unsafe deserialization) without explicit justification and mitigation.
- Log security-relevant events without logging secrets.

Verification:
- No new secrets appear in source, logs, or configuration committed to the repository.
- Existing security-related tests and static checks still pass.

### 7. Maintainability

Goal: The system remains easy to understand, modify, and extend.

Rules:
- Prefer small, cohesive modules with clear responsibilities.
- Avoid duplicating existing logic; extract or reuse when the same behaviour appears in multiple places.
- Keep functions and classes short enough that their purpose is obvious.
- Follow existing naming, structure, and layering conventions of the codebase.
- Do not introduce circular dependencies or increase coupling without strong reason.
- Update or add documentation and comments only where they materially help future readers (including future agents).

Verification:
- New code follows the dominant style and structure of the surrounding files.
- Complexity of modified modules does not increase unnecessarily.
- Related tests remain readable and focused.

### 8. Portability

Goal: Can be transferred or adapted to different environments with reasonable effort.

Rules:
- Avoid hard-coding environment-specific paths, hosts, or configuration values.
- Prefer standard language features and well-supported libraries over platform-specific extensions unless required.
- Keep configuration externalised and environment-agnostic where possible.
- Do not introduce assumptions that break existing supported platforms or deployment targets.

Verification:
- Code continues to build and run in the environments already supported by the project.
- No new environment-specific hard-coding introduced without corresponding configuration.

### Incremental Development Discipline

- Work in small, reviewable steps. Prefer multiple focused commits or pull requests over one large change.
- After each logical step, ensure the relevant quality attributes still hold.
- When the task is large, first outline the sequence of increments and the quality checks for each.
- Prefer test-driven or test-accompanied changes for behaviour that can be expressed as tests.
- If a change risks degrading any of the eight attributes, either mitigate the risk in the same change or explicitly call it out before proceeding.

### Definition of Done (Quality Checklist)

A change is complete only when all of the following are true:

1. The requested behaviour is implemented and verified.
2. Existing tests that cover related behaviour still pass.
3. New or updated tests exist for new or changed behaviour (where practical).
4. No quality attribute among the eight has been knowingly degraded without explicit justification and mitigation.
5. Public interfaces remain compatible or a clear migration path is provided.
6. No secrets or sensitive data were introduced.
7. The change set is focused and does not contain unrelated modifications.
8. Lint, type-check, and test commands that the project defines succeed (or the agent reports exactly which ones could not be run and why).

### When in Doubt

- Prefer preserving existing quality over adding new features.
- Prefer explicitness and testability over cleverness.
- Prefer small, reversible changes over large, irreversible ones.
- Surface conflicts between the current request and these quality rules instead of silently violating them.
