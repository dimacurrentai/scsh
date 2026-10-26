# SSH build-proxy verification

This executable harness verifies the external SSH and runtime lifecycle for `--ssh-proxy`. Pure destination/argument/network parsing is covered by Rust unit tests; `python3 -m unittest discover -s scripts -p 'test_build_proxy.py'` covers HTTP, CONNECT, origin authentication, invalid targets, and remote-helper shutdown using ephemeral loopback ports.

## Prerequisites

Use Apple Containers or rootful Linux Docker/Podman with a local bridge. Set `PROXY_HOST` to a Linux SSH alias configured with `HostName` and `User` in the host machine's `~/.ssh/config`. Noninteractive authentication, TCP forwarding, and Python 3 must work: `ssh "$PROXY_HOST" python3 --version`. Explicit `username@hostname` destinations are also accepted. No special remote packages or services are needed.

Build `scsh` with `cargo build` and use `./target/debug/scsh` below. Keep full command output visible. Run real image rebuilds only when no other session is using or restarting the shared builder.

## Download and cache behavior

1. Run `./target/debug/scsh build-images base --rebuild-base --ssh-proxy "$PROXY_HOST"`.
   - Predict: the build row reports SSH proxy setup, the Debian package and tool downloads succeed, and the command exits zero. The base image remains local.
2. Run `./target/debug/scsh build-images base --ssh-proxy unreachable.invalid` with the freshly built image.
   - Predict: exit zero with “nothing to build”; no SSH connection is attempted. The proxy destination does not change the image fingerprint.
3. Inspect the image environment with the runtime's image-inspection command.
   - Predict: no proxy endpoint or SSH destination is stored in the image environment. The Dockerfile does not declare proxy ARG/ENV instructions.
4. Run `./target/debug/scsh run harness-smoke --no-cache --ssh-proxy "$PROXY_HOST"` from a clean committed checkout.
   - Predict: any stale harness images build through SSH; every available model writes its fresh `tmp/harness-smoke-<route>.json` with `result.status` equal to `OK`. A cached image requires no SSH, and model traffic follows its usual path.

## Failure and cleanup

1. Force a build with `./target/debug/scsh build-images base --rebuild-base --ssh-proxy unreachable.invalid`.
   - Predict: nonzero exit with SSH diagnostics and an actionable proxy-startup error within the startup deadline; no fallback or builder starts after proxy setup fails.
2. Start the successful rebuild command, wait until downloads begin, then press Ctrl+C.
   - Predict: exit 130; both local SSH children and their local bridge listeners disappear. The remote Python server exits when its SSH stdin closes.
3. Repeat with SIGTERM; repeat after disconnecting SSH during the build, and with a deliberately failing build in an isolated fixture.
   - Predict: signal exit 143 or a nonzero build exit as appropriate; SSH keepalives bound dead-connection detection. Local proxy children and listeners disappear on every exit, and no remote Python process remains. Compare process/socket inventories before and after, identifying only this invocation's processes; do not kill another session's connections.

The proxy is deliberately scoped to build-step HTTP/HTTPS downloads. It does not change image pulls, agent/model traffic, build architecture, or the location where builds execute. Docker Desktop, rootless gateways, and remote engines without a host-bindable local bridge must fail with an actionable unsupported-network message.
