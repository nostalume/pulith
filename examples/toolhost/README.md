# toolhost example

`toolhost` demonstrates how an application can build, harvest, verify, publish, activate, dispatch,
and optionally register a tool as a native system service while keeping those behaviors orthogonal.
Service-manager code remains example-owned rather than becoming a universal Pulith abstraction.

## Executables

The example consists of three cooperating binaries:

| Binary | Responsibility |
| --- | --- |
| `toolhost` | recipe resolution, build/harvest, publication, activation, environment, and service commands |
| `toolhost-shim` | compiled release-local dispatcher placed under `shims/` |
| `toolhost-service` | systemd or Windows SCM host that launches and supervises the payload |

Build all three together because installation harvests the companion binaries from beside
`toolhost`:

```text
cargo build --no-default-features --features serde,process \
  --example toolhost --example toolhost-shim --example toolhost-service
```

On PowerShell, place the command on one line or use PowerShell's continuation syntax.

## Recipe workflow

A recipe names the real outputs of an ordinary build. Toolhost resolves the command once, executes
it in the admitted worktree, copies the declared artifacts into a private release stage, runs each
verification in an exact environment, validates the layout, and publishes:

```text
<root>/installs/<name>/<version>/
  bin/<name>                 # harvested payload
  private-runtime/           # optional private dependencies
  shims/<name>               # compiled dispatcher
  service/<name>             # native service host
```

```toml
name = "demo-tool"
version = "1.2.0"

[build]
command = "cargo"
args = ["build", "--release"]
working_dir = "."
timeout_seconds = 120

[outputs]
binary = "target/release/demo-tool"

[[verify]]
args = ["--version"]
stdout = "demo-tool 1.2.0\n"
```

On Windows, the declared binary normally includes `.exe`. If `[build]` is absent, install performs
no filesystem or process effects and prints `no-build`; `[outputs]` remains required by the strict
schema. A declared runtime directory requires exactly one `loaded_runtime` verification so the
loaded dependency identity and origin are observed, not merely assumed.

## Use

```text
toolhost install    --root <absolute-root> <toolhost.toml>
toolhost activate   --root <absolute-root> <name> <version>
toolhost deactivate --root <absolute-root>
toolhost env        --root <absolute-root>
toolhost run        --root <absolute-root> -- <command> [args...]
```

Activation links `<root>/current` to one published release. `env` prints the values a caller can
inject. `run` prepends the active compiled-shim directory and sets `TOOLHOST_HOME` for one child
without mutating the parent process environment.

## Service declaration

```toml
schema = 1
id = "demo-tool"
payload = "demo-tool"
args = ["serve", "--foreground"]

[environment]
RUST_LOG = "info"
```

Service operations are independent and idempotent where their underlying manager permits it:

```text
toolhost service install --root <absolute-protected-root> <service.toml>
toolhost service rebind|enable|start|restart|status|stop|disable|remove \
  --root <absolute-protected-root> <service.toml>
```

The root must already be link-free and protected from untrusted writes. Toolhost does not elevate
or repair permissions. Linux uses a hardened dynamic-user systemd unit. Windows SCM uses
`LocalService`, a restricted service SID, and receipted read/execute grants. These mechanisms differ
but the declaration, lifecycle verbs, payload identity, exact environment, and status meanings are
the same.

## Pulith features and APIs

| Feature | APIs exercised | Purpose |
| --- | --- | --- |
| `process` | `WorktreeProcess`, `ManagedProcess`, `EnvVars`, process evidence and diagnostics | bounded builds, exact verification environments, and supervised payloads |
| `local` via `process` | `StagedTree`, `LocalSource`, `LocalTarget`, `Link`, `Unlink`, `Remove`, `RecordStore` | harvest, atomic publication, active views, definitions, and Windows receipts |
| `serde` | recipe and service declaration parsing | strict declarative input |

`signal-hook` and `sd-notify` are Linux example dependencies used by the systemd host. They are not
Pulith features. Windows SCM calls are example-owned `windows-sys` integration.

## Guarantees and non-goals

- Build paths and outputs are admitted beneath the selected recipe worktree.
- Publication occurs only after declared verification and layout validation.
- Verification starts with an exact environment; private runtime injection is explicit.
- The compiled shim derives its release from its own location and does not embed an install path.
- Service operations do not request elevation and do not broaden root permissions.
- Toolhost is an architecture probe, not a stable service-management ecosystem or a replacement for
  Cargo, systemd, or Windows SCM tooling.

See the crate [architecture](../../docs/architecture.md) and the `local` and `process` modules in
rustdoc for the underlying contracts.
