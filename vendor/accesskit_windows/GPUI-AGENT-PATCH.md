# Windows accessibility lifetime fix

This is the runtime source of `accesskit_windows` 0.34.0 from crates.io,
upstream revision `c88605b96d04431f9c3c792464a0f2f253480e94`
(`platforms/windows`). The original MIT/Apache-2.0 notices and license texts
are retained. Cargo's workspace patch selects this copy only for consumers
of the existing Windows adapter; the application package graph is unchanged.

An accessibility client can retain a provider while its HWND is destroyed.
Upstream `util::client_top_left` unwraps `ClientToScreen`, which panics with
`0x80070578` inside a COM callback that cannot unwind. It now returns
`UIA_E_ELEMENTNOTAVAILABLE`. `context.rs`, `node.rs`, and `text.rs` propagate
that error through element bounds, hit testing, text range lookup, and text
range bounds. Live-window geometry and expired-context behavior are preserved.

The manifest retains upstream runtime dependencies. Upstream examples and
integration-test-only dependencies are omitted. Existing inline tests remain;
`adapter.rs` registers `lifecycle_tests.rs`, which uses actual hidden HWNDs
and the public COM provider interfaces without a screen reader or credentials.
The new tests cover a destroyed HWND with a retained adapter/provider, text
geometry after destruction, and a released adapter with a retained provider.

Run on Windows from the workspace root:

```text
cargo test --locked --package accesskit_windows --lib
cargo harness check --include-desktop
```

The latter runs this suite explicitly: Cargo's workspace test selection does
not include vendored dependencies. Keep this directory excluded from the
workspace so non-Windows builds do not compile a Windows-only test target.

Remove this patch and its harness gate when a compatible upstream release
propagates window-destruction errors through all four geometry queries.
